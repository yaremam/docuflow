//! S3-compatible blob storage (LocalStack in dev, real S3 in prod — the same
//! code targets either, purely via standard AWS environment variables:
//! `AWS_ENDPOINT_URL`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
//! `AWS_REGION`). `stream_upload` uses S3's multipart-upload API in
//! fixed-size parts read directly off the incoming multipart body, so memory
//! use stays bounded to one part's size regardless of the object's total
//! length — per CLAUDE.md's blob-storage rule: "Files... must be securely
//! chunked and pushed to the storage layers using streaming wrappers to
//! prevent high memory spikes." This is the first real blob-storage code in
//! the project (`aws-sdk-s3`/`aws-config` were previously declared
//! dependencies with no usage anywhere), so it's written as the reusable
//! primitive future document-upload features build on, not a
//! profile-picture-only shortcut.

use std::time::Duration;

use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_smithy_types::byte_stream::ByteStream;
use axum::extract::multipart::{Field, MultipartError};

/// S3's minimum part size for all but the final part of a multipart upload.
const PART_SIZE: usize = 5 * 1024 * 1024;

/// How long a presigned picture URL stays valid before the client needs a
/// fresh one (re-requested on the next page load).
const PRESIGNED_URL_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
pub struct BlobStore {
    client: aws_sdk_s3::Client,
    presign_client: aws_sdk_s3::Client,
    bucket: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("s3 error: {0}")]
    S3(String),
    #[error("multipart error: {0}")]
    Multipart(#[from] MultipartError),
    #[error("upload exceeds the {0}-byte limit")]
    TooLarge(usize),
}

/// Every S3 SDK call site below maps its error the same way — one place to
/// keep that formatting consistent, rather than each call site picking
/// `{e:?}` vs `.to_string()` independently.
fn s3_err(e: impl std::fmt::Debug) -> BlobError {
    BlobError::S3(format!("{e:?}"))
}

/// Builds an S3 client from the given (already-loaded) AWS config.
/// `force_path_style` is required for LocalStack's `http://host:port/bucket`
/// addressing (real S3 accepts it too, so no environment-specific branching
/// is needed here). `request_checksum_calculation` is pinned to
/// `WhenRequired` (off by default) rather than the SDK's own default of
/// `WhenSupported` — that default silently attaches a CRC32 checksum to
/// `UploadPart` requests, which LocalStack's multipart-upload
/// implementation doesn't reconcile with `CompleteMultipartUpload`,
/// failing every multipart upload with "Checksum Type mismatch". We don't
/// rely on this integrity feature ourselves, so disabling it is a clean fix
/// rather than a workaround. `endpoint_override` is set only for the
/// presign client when `BLOB_PUBLIC_ENDPOINT_URL` is configured (see
/// `clients_from_env`).
fn build_client(config: &aws_config::SdkConfig, endpoint_override: Option<String>) -> aws_sdk_s3::Client {
    let mut builder = aws_sdk_s3::config::Builder::from(config)
        .force_path_style(true)
        .request_checksum_calculation(aws_sdk_s3::config::RequestChecksumCalculation::WhenRequired);
    if let Some(endpoint) = endpoint_override {
        builder = builder.endpoint_url(endpoint);
    }
    aws_sdk_s3::Client::from_conf(builder.build())
}

/// Builds the pair of clients `BlobStore` needs from standard AWS
/// environment variables, loading the shared AWS config only once. The
/// second client is used only for generating presigned URLs, and only
/// differs from the first when `BLOB_PUBLIC_ENDPOINT_URL` is set: in Docker
/// Compose dev, the app reaches LocalStack via the internal service hostname
/// (`AWS_ENDPOINT_URL=http://localstack:4566`), but a presigned URL embedded
/// in a rendered page must resolve from the *user's own browser* instead —
/// which can't see Docker's internal network and needs the host-mapped
/// `localhost:4566`. When unset (real S3, or host-side non-Docker
/// `cargo run`, where there's only one endpoint and it's already
/// browser-reachable), the presign client is simply a clone of the first —
/// no second config load or client build needed.
pub async fn clients_from_env() -> (aws_sdk_s3::Client, aws_sdk_s3::Client) {
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let client = build_client(&config, None);
    let presign_client = match std::env::var("BLOB_PUBLIC_ENDPOINT_URL") {
        Ok(public_endpoint) => build_client(&config, Some(public_endpoint)),
        Err(_) => client.clone(),
    };
    (client, presign_client)
}

impl BlobStore {
    pub fn new(client: aws_sdk_s3::Client, presign_client: aws_sdk_s3::Client, bucket: String) -> Self {
        Self {
            client,
            presign_client,
            bucket,
        }
    }

    /// Idempotent: a `head_bucket` check, only `create_bucket` on
    /// not-found. Safe to call on every boot, same treatment as
    /// `state::migrate`.
    pub async fn ensure_bucket(&self) -> Result<(), BlobError> {
        match self.client.head_bucket().bucket(&self.bucket).send().await {
            Ok(_) => Ok(()),
            Err(err) => {
                let service_err = err.into_service_error();
                if service_err.is_not_found() {
                    self.client
                        .create_bucket()
                        .bucket(&self.bucket)
                        .send()
                        .await
                        .map_err(s3_err)?;
                    Ok(())
                } else {
                    Err(s3_err(service_err))
                }
            }
        }
    }

    /// Streams `field` to S3 and returns the total number of bytes uploaded
    /// (already tracked internally for the `max_bytes` check, so callers
    /// that need the final size — e.g. to populate a `file_size_bytes`
    /// column — don't need a second read).
    #[tracing::instrument(skip(self, field))]
    pub async fn stream_upload(
        &self,
        key: &str,
        content_type: &str,
        mut field: Field<'_>,
        max_bytes: usize,
    ) -> Result<usize, BlobError> {
        let create = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .send()
            .await
            .map_err(s3_err)?;
        let upload_id = create
            .upload_id()
            .ok_or_else(|| BlobError::S3("create_multipart_upload: missing upload id".to_string()))?
            .to_string();

        let mut part_number: i32 = 1;
        let mut completed_parts = Vec::new();
        let mut buf: Vec<u8> = Vec::with_capacity(PART_SIZE);
        let mut total = 0usize;

        while let Some(chunk) = field.chunk().await? {
            total += chunk.len();
            if total > max_bytes {
                // Best-effort cleanup — the upload is being rejected either
                // way, so a failure here doesn't change the outcome for the
                // caller.
                let _ = self
                    .client
                    .abort_multipart_upload()
                    .bucket(&self.bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .send()
                    .await;
                return Err(BlobError::TooLarge(max_bytes));
            }
            buf.extend_from_slice(&chunk);
            if buf.len() >= PART_SIZE {
                // `mem::replace` (not `mem::take`) so the next part's buffer
                // starts pre-sized too — `mem::take` would leave a
                // zero-capacity `Vec` behind, forcing every subsequent part
                // to regrow via repeated reallocation as chunks stream in.
                let part_bytes = std::mem::replace(&mut buf, Vec::with_capacity(PART_SIZE));
                self.upload_part(key, &upload_id, part_number, part_bytes, &mut completed_parts)
                    .await?;
                part_number += 1;
            }
        }

        // S3 requires at least one part, even for an empty/small file.
        if !buf.is_empty() || completed_parts.is_empty() {
            self.upload_part(key, &upload_id, part_number, buf, &mut completed_parts)
                .await?;
        }

        self.client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(&upload_id)
            .multipart_upload(
                CompletedMultipartUpload::builder()
                    .set_parts(Some(completed_parts))
                    .build(),
            )
            .send()
            .await
            .map_err(s3_err)?;

        Ok(total)
    }

    async fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: i32,
        bytes: Vec<u8>,
        completed_parts: &mut Vec<CompletedPart>,
    ) -> Result<(), BlobError> {
        let output = self
            .client
            .upload_part()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(upload_id)
            .part_number(part_number)
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(s3_err)?;

        let e_tag = output
            .e_tag()
            .ok_or_else(|| BlobError::S3("upload_part: missing e_tag".to_string()))?
            .to_string();

        // The SDK attaches a CRC32 checksum to the upload_part request by
        // default; `complete_multipart_upload` must echo it back per part or
        // S3 (and LocalStack) reject the completion with a checksum-type
        // mismatch, since the completion request would otherwise imply "no
        // checksum" for a part that was actually uploaded with one.
        let mut part = CompletedPart::builder().part_number(part_number).e_tag(e_tag);
        if let Some(checksum) = output.checksum_crc32() {
            part = part.checksum_crc32(checksum);
        }
        completed_parts.push(part.build());
        Ok(())
    }

    /// Downloads an object's full body into memory. A plain buffered GET,
    /// not a streaming one — deliberately: this is used only by detached
    /// background work (OCR) on an already-size-bounded (≤20MB) file, not
    /// the client-facing upload path `stream_upload` above exists to keep
    /// bounded; re-fetching here instead of having the request handler hand
    /// its already-streamed bytes to the background task keeps "bounded
    /// streaming upload" and "backgrounded full-buffer OCR read" as
    /// separate concerns, rather than making the request path hold a whole
    /// file in memory just to save one background-only round trip.
    pub async fn get_object(&self, key: &str) -> Result<Vec<u8>, BlobError> {
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(s3_err)?;
        let bytes = output.body.collect().await.map_err(s3_err)?.into_bytes();
        Ok(bytes.to_vec())
    }

    /// Short-lived presigned GET URL, for rendering the picture in an
    /// `<img>` without making the bucket itself public.
    pub async fn presigned_get_url(&self, key: &str) -> Result<String, BlobError> {
        let presigning_config = PresigningConfig::expires_in(PRESIGNED_URL_TTL).map_err(s3_err)?;
        let presigned = self
            .presign_client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(presigning_config)
            .await
            .map_err(s3_err)?;
        Ok(presigned.uri().to_string())
    }
}
