use axum::extract::{Form, Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use tracing::Instrument;
use uuid::Uuid;

use crate::web::error::AppWebError;
use crate::web::forms::{DateIssuedField, ProfileField, Tags};
use crate::web::nav;
use crate::web::state::AppState;
use crate::web::tenancy::TenantContext;
use crate::web::templates::{
    DocumentDeleteTemplate, DocumentListItem, DocumentNewTemplate, DocumentShowTemplate, DocumentsListTemplate,
};

/// Also used by the router to size its `DefaultBodyLimit` layer — kept as
/// one constant so the two enforcement points (that layer, and the
/// mid-stream check in `BlobStore::stream_upload`) can't drift apart.
/// Larger than `profile::MAX_PICTURE_BYTES` (8MB) — scanned bills/policies
/// run bigger than an avatar photo.
pub const MAX_DOCUMENT_BYTES: usize = 20 * 1024 * 1024;

/// Content types accepted at all. Kept separate from `ocr::PDF_CONTENT_TYPE`
/// rather than folding PDF into this list, because `document_preview` below
/// also reuses this exact list as its "can the browser inline this as
/// `<img>`" check — a PDF needs an `<embed>`, not an `<img>`, even though
/// (per `ocr_eligible` below) it is just as OCR-eligible as these image
/// types. Everything not in this list or `OTHER_ACCEPTED_CONTENT_TYPES` is
/// rejected with 400. Also used by `web::handlers::scan` (feature 009's
/// phone-camera capture) to decide OCR eligibility for the image types it
/// accepts.
pub(crate) const OCR_ELIGIBLE_CONTENT_TYPES: &[&str] = &["image/jpeg", "image/png", "image/tiff", "image/webp"];
const OTHER_ACCEPTED_CONTENT_TYPES: &[&str] = &[crate::ocr::PDF_CONTENT_TYPE];

/// Whether a content type gets a real OCR pass: the direct-image types plus
/// PDF — `ocr::extract` itself owns the decision of whether a given content
/// type needs rasterizing first, this just decides whether to queue the
/// background pass at all.
fn ocr_eligible(content_type: &str) -> bool {
    OCR_ELIGIBLE_CONTENT_TYPES.contains(&content_type) || content_type == crate::ocr::PDF_CONTENT_TYPE
}

fn format_date(date: time::Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year(), date.month() as u8, date.day())
}

/// A presigned view URL plus whether the browser can inline it as an
/// `<img>` (vs. needing a PDF `<embed>`) — shared by `list` and `show` since
/// both render a preview from the same `blob_key`/`content_type` pair.
/// Reuses `OCR_ELIGIBLE_CONTENT_TYPES` for the image check: that constant is
/// named for OCR eligibility, but today's four image types are exactly the
/// set a browser can inline too, so it doubles as the "is this an image"
/// answer without a second, parallel list to keep in sync.
async fn document_preview(
    blob: &crate::blob::BlobStore,
    blob_key: &str,
    content_type: &str,
) -> Result<(String, bool), AppWebError> {
    let file_url = blob.presigned_get_url(blob_key).await?;
    let is_image = OCR_ELIGIBLE_CONTENT_TYPES.contains(&content_type);
    Ok((file_url, is_image))
}

#[derive(Debug, Clone, Copy)]
enum Sort {
    CreatedAtDesc,
    CreatedAtAsc,
    DateIssuedDesc,
    DateIssuedAsc,
    TagsAsc,
}

impl Sort {
    fn parse(value: Option<&str>) -> Self {
        match value {
            Some("created_at_asc") => Self::CreatedAtAsc,
            Some("date_issued_desc") => Self::DateIssuedDesc,
            Some("date_issued_asc") => Self::DateIssuedAsc,
            Some("tags_asc") => Self::TagsAsc,
            _ => Self::CreatedAtDesc,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::CreatedAtDesc => "created_at_desc",
            Self::CreatedAtAsc => "created_at_asc",
            Self::DateIssuedDesc => "date_issued_desc",
            Self::DateIssuedAsc => "date_issued_asc",
            Self::TagsAsc => "tags_asc",
        }
    }
}

/// Parses the search box's comma-separated tag list into an overlap filter.
/// A deliberately ad hoc parse (not the `Tags` form newtype) since this is a
/// transient query filter, not data being stored.
fn parse_tag_search(q: &str) -> Option<Vec<String>> {
    let tags: Vec<String> = q
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect();

    if tags.is_empty() {
        None
    } else {
        Some(tags)
    }
}

struct DocumentListRow {
    id: Uuid,
    title: Option<String>,
    original_filename: String,
    content_type: String,
    blob_key: String,
    tags: Vec<String>,
    date_issued: Option<time::Date>,
    ocr_status: String,
    created_at: time::OffsetDateTime,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    q: String,
    sort: Option<String>,
    #[serde(default)]
    deleted: bool,
}

#[tracing::instrument(skip(state, tenancy))]
pub async fn list(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<DocumentsListTemplate, AppWebError> {
    let nav_avatar_url = nav::avatar_url(&state.pool, &state.blob, tenancy.user_id.0).await?;
    let tag_filter = parse_tag_search(&query.q);
    let sort = Sort::parse(query.sort.as_deref());

    // Each arm differs only in `ORDER BY` — sqlx's compile-time `query_as!`
    // macro can't parameterize that clause, so the small, fixed set of sort
    // modes is spelled out literally rather than building the SQL string at
    // runtime (which would forgo compile-time verification for every query
    // on this page, not just the ordering).
    let rows = match sort {
        Sort::CreatedAtDesc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, created_at
                   from documents
                   where tenant_id = $1 and ($2::text[] is null or tags && $2)
                   order by created_at desc"#,
                tenancy.tenant_id.0,
                tag_filter.as_deref(),
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::CreatedAtAsc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, created_at
                   from documents
                   where tenant_id = $1 and ($2::text[] is null or tags && $2)
                   order by created_at asc"#,
                tenancy.tenant_id.0,
                tag_filter.as_deref(),
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::DateIssuedDesc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, created_at
                   from documents
                   where tenant_id = $1 and ($2::text[] is null or tags && $2)
                   order by date_issued desc nulls last"#,
                tenancy.tenant_id.0,
                tag_filter.as_deref(),
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::DateIssuedAsc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, created_at
                   from documents
                   where tenant_id = $1 and ($2::text[] is null or tags && $2)
                   order by date_issued asc nulls last"#,
                tenancy.tenant_id.0,
                tag_filter.as_deref(),
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::TagsAsc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, created_at
                   from documents
                   where tenant_id = $1 and ($2::text[] is null or tags && $2)
                   order by array_to_string(tags, ',') asc"#,
                tenancy.tenant_id.0,
                tag_filter.as_deref(),
            )
            .fetch_all(&state.pool)
            .await?
        }
    };

    let mut documents = Vec::new();
    for row in rows {
        let (file_url, is_image) = document_preview(&state.blob, &row.blob_key, &row.content_type).await?;
        documents.push(DocumentListItem {
            id: row.id,
            title: row.title.unwrap_or_else(|| row.original_filename.clone()),
            original_filename: row.original_filename,
            file_url,
            is_image,
            tags: row.tags,
            date_issued: row.date_issued.map(format_date),
            uploaded_at: format_date(row.created_at.date()),
            ocr_status: row.ocr_status,
        });
    }

    Ok(DocumentsListTemplate {
        active_tab: "documents",
        authenticated: true,
        nav_avatar_url,
        q: query.q,
        sort: sort.as_str(),
        deleted: query.deleted,
        documents,
    })
}

struct DocumentRow {
    id: Uuid,
    title: Option<String>,
    original_filename: String,
    content_type: String,
    file_size_bytes: i64,
    blob_key: String,
    tags: Vec<String>,
    date_issued: Option<time::Date>,
    ocr_suggested_date_issued: Option<time::Date>,
    ocr_status: String,
    ocr_text: Option<String>,
    created_at: time::OffsetDateTime,
}

#[derive(Debug, Deserialize)]
pub struct ShowQuery {
    #[serde(default)]
    saved: bool,
    #[serde(default)]
    uploaded: bool,
}

#[tracing::instrument(skip(state, tenancy))]
pub async fn show(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ShowQuery>,
) -> Result<DocumentShowTemplate, AppWebError> {
    let nav_avatar_url = nav::avatar_url(&state.pool, &state.blob, tenancy.user_id.0).await?;

    let row = sqlx::query_as!(
        DocumentRow,
        r#"select id, title, original_filename, content_type, file_size_bytes, blob_key, tags, date_issued,
                  ocr_suggested_date_issued, ocr_status, ocr_text, created_at
           from documents
           where id = $1 and tenant_id = $2"#,
        id,
        tenancy.tenant_id.0,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppWebError::NotFound)?;

    let (file_url, is_image) = document_preview(&state.blob, &row.blob_key, &row.content_type).await?;
    // Only surfaced when there's nothing already in `date_issued` — a
    // suggestion never contends with, or gets confused for, a value the
    // user already set (see TDR 012).
    let suggested_date_issued_display =
        if row.date_issued.is_none() { row.ocr_suggested_date_issued.map(format_date) } else { None };

    Ok(DocumentShowTemplate {
        active_tab: "documents",
        authenticated: true,
        nav_avatar_url,
        saved: query.saved,
        uploaded: query.uploaded,
        id: row.id,
        title: row.title.unwrap_or_else(|| row.original_filename.clone()),
        original_filename: row.original_filename,
        content_type: row.content_type,
        file_size_bytes: row.file_size_bytes,
        file_url,
        is_image,
        tags_input_value: row.tags.join(", "),
        date_issued_input_value: row.date_issued.map(format_date).unwrap_or_default(),
        suggested_date_issued_display,
        uploaded_at: format_date(row.created_at.date()),
        ocr_status: row.ocr_status,
        ocr_text: row.ocr_text,
    })
}

#[derive(Debug, Deserialize)]
pub struct DocumentMetadataForm {
    #[serde(default)]
    pub title: ProfileField,
    #[serde(default)]
    pub tags: Tags,
    #[serde(default)]
    pub date_issued: DateIssuedField,
}

#[tracing::instrument(skip(state, tenancy, form))]
pub async fn update(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(form): Form<DocumentMetadataForm>,
) -> Result<Response, AppWebError> {
    let result = sqlx::query!(
        "update documents set title = $3, tags = $4, date_issued = $5, updated_at = now()
         where id = $1 and tenant_id = $2",
        id,
        tenancy.tenant_id.0,
        form.title.into_option(),
        &form.tags.into_vec(),
        form.date_issued.into_option(),
    )
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppWebError::NotFound);
    }

    Ok(Redirect::to(&format!("/documents/{id}?saved=true")).into_response())
}

/// Copies `ocr_suggested_date_issued` into `date_issued` — the "Use this
/// date" action from the suggestion box `show` renders (see TDR 012). The
/// `where date_issued is null` guard means this is safe to call even if
/// the suggestion no longer applies (already accepted, or `date_issued`
/// was set some other way since): it's just a no-op, not an error. That
/// guard is also why this can't use `update`'s plain `rows_affected() ==
/// 0` idiom directly: 0 rows affected here is ambiguous between "document
/// doesn't exist" (404) and "exists, but the guard didn't match" (no-op)
/// — so the existence check only runs as a fallback on that path, keeping
/// the common (successful-accept) case down to one round-trip like every
/// other handler in this file.
#[tracing::instrument(skip(state, tenancy))]
pub async fn accept_suggested_date(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppWebError> {
    let result = sqlx::query!(
        "update documents set date_issued = ocr_suggested_date_issued, updated_at = now()
         where id = $1 and tenant_id = $2 and date_issued is null",
        id,
        tenancy.tenant_id.0,
    )
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        let exists = sqlx::query_scalar!(
            "select exists(select 1 from documents where id = $1 and tenant_id = $2)",
            id,
            tenancy.tenant_id.0,
        )
        .fetch_one(&state.pool)
        .await?
        .unwrap_or(false);

        if !exists {
            return Err(AppWebError::NotFound);
        }
    }

    Ok(Redirect::to(&format!("/documents/{id}?saved=true")).into_response())
}

struct DocumentSummaryRow {
    title: Option<String>,
    original_filename: String,
}

#[tracing::instrument(skip(state, tenancy))]
pub async fn confirm_delete(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<DocumentDeleteTemplate, AppWebError> {
    let nav_avatar_url = nav::avatar_url(&state.pool, &state.blob, tenancy.user_id.0).await?;

    let row = sqlx::query_as!(
        DocumentSummaryRow,
        "select title, original_filename from documents where id = $1 and tenant_id = $2",
        id,
        tenancy.tenant_id.0,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppWebError::NotFound)?;

    Ok(DocumentDeleteTemplate {
        active_tab: "documents",
        authenticated: true,
        nav_avatar_url,
        id,
        title: row.title.unwrap_or_else(|| row.original_filename.clone()),
        original_filename: row.original_filename,
    })
}

/// Deletes the DB row first (tenant-scoped `DELETE ... RETURNING` both
/// checks ownership and hands back the `blob_key` in one query), then the
/// blob — so a blob-delete failure never leaves a row pointing at storage
/// that's already gone, only the reverse (an orphaned blob with no row,
/// which is invisible to the user and harmless beyond wasted storage). The
/// blob delete's own failure is logged, not bubbled via `?`: by the time it
/// runs, the DB delete has already committed, so from the user's side the
/// document is already gone — surfacing a 500 here would report failure for
/// an action that, as far as they can tell, already succeeded.
#[tracing::instrument(skip(state, tenancy))]
pub async fn delete(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppWebError> {
    let row = sqlx::query!(
        "delete from documents where id = $1 and tenant_id = $2 returning blob_key",
        id,
        tenancy.tenant_id.0,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppWebError::NotFound)?;

    if let Err(error) = state.blob.delete_object(&row.blob_key).await {
        tracing::warn!(%error, "failed to delete blob for an already-deleted document row");
    }

    Ok(Redirect::to("/documents?deleted=true").into_response())
}

#[tracing::instrument(skip(state, tenancy))]
pub async fn new_form(
    tenancy: TenantContext,
    State(state): State<AppState>,
) -> Result<DocumentNewTemplate, AppWebError> {
    let nav_avatar_url = nav::avatar_url(&state.pool, &state.blob, tenancy.user_id.0).await?;
    Ok(DocumentNewTemplate {
        active_tab: "documents",
        authenticated: true,
        nav_avatar_url,
    })
}

/// Also used by `web::handlers::scan` (feature 009's phone-camera capture),
/// which shares this pipeline's 400-on-invalid-upload shape.
pub(crate) fn bad_request(message: &'static str) -> Response {
    (StatusCode::BAD_REQUEST, message).into_response()
}

/// Parses a multipart text field's value into a validated form newtype,
/// shared by `create`'s title/tags/date_issued arms below — these don't go
/// through the `Form` extractor's automatic serde validation (multipart
/// bodies mix text fields with a file field), so each has to call
/// `TryFrom<String>` by hand; this just avoids repeating the "match, keep
/// value or bail with 400" shape three times. Returns the plain error
/// message rather than a built `Response` — `clippy::result_large_err`
/// flags `Response` (128+ bytes) as an oversized `Err` variant, and callers
/// already have `bad_request` in scope to build the response themselves.
fn parse_metadata_field<T: TryFrom<String>>(text: String, error_message: &'static str) -> Result<T, &'static str> {
    T::try_from(text).map_err(|_| error_message)
}

/// Runs the OCR pass for a just-uploaded, OCR-eligible document as detached
/// background work — spawned by `create` below, never awaited by the
/// request. Errors are caught and turned into `ocr_status = 'failed'`
/// locally; there's no request left to propagate an `AppWebError` to.
#[tracing::instrument(skip(state))]
async fn run_ocr(state: AppState, document_id: Uuid, tenant_id: Uuid, blob_key: String, content_type: String) {
    let _permit = match state.ocr_semaphore.acquire().await {
        Ok(permit) => permit,
        Err(_) => return, // semaphore closed only if AppState is being torn down
    };

    if let Err(error) = sqlx::query!(
        "update documents set ocr_status = 'processing' where id = $1 and tenant_id = $2",
        document_id,
        tenant_id,
    )
    .execute(&state.pool)
    .await
    {
        tracing::error!(%error, "failed to mark document as processing");
        return;
    }

    let outcome = match state.blob.get_object(&blob_key).await {
        Ok(bytes) => crate::ocr::extract(&content_type, &bytes).await.map_err(|e| e.to_string()),
        Err(error) => Err(error.to_string()),
    };

    let update_result = match outcome {
        Ok(text) => {
            let suggested_date_issued = crate::date_extract::extract_issued_date(&text);
            sqlx::query!(
                "update documents set ocr_status = 'done', ocr_text = $3, ocr_suggested_date_issued = $4
                 where id = $1 and tenant_id = $2",
                document_id,
                tenant_id,
                text,
                suggested_date_issued,
            )
            .execute(&state.pool)
            .await
        }
        Err(error_message) => {
            tracing::error!(error = %error_message, "ocr extraction failed");
            sqlx::query!(
                "update documents set ocr_status = 'failed', ocr_error = $3 where id = $1 and tenant_id = $2",
                document_id,
                tenant_id,
                error_message,
            )
            .execute(&state.pool)
            .await
        }
    };

    if let Err(error) = update_result {
        tracing::error!(%error, "failed to record ocr outcome");
    }
}

/// Streams `field` to blob storage under `documents/{user_id}/{document_id}`
/// and returns the resulting byte count. Split out from
/// `insert_document_and_queue_ocr` below specifically so `create`'s
/// "metadata fields must arrive before the file field" check can still
/// reject a request (with no document row ever created) even *after* this
/// stream has already run — the file field is necessarily read as it's
/// encountered (multipart bodies can't be rewound to check what follows
/// first), so only the upload is unavoidable in that rejected case, never
/// the DB row; the S3 object it leaves behind is an accepted, harmless
/// orphan (nothing ever references it). Also called directly by
/// `web::handlers::scan::submit_scan` (feature 009's phone-camera capture),
/// which has no metadata fields that could invalidate an already-streamed
/// file, so it has no need for a combined convenience wrapper — it just
/// calls this and `insert_document_and_queue_ocr` back to back itself.
pub(crate) async fn stream_document_to_blob(
    state: &AppState,
    user_id: Uuid,
    document_id: Uuid,
    content_type: &str,
    field: axum::extract::multipart::Field<'_>,
) -> Result<(String, i64), AppWebError> {
    let blob_key = format!("documents/{user_id}/{document_id}");
    let file_size_bytes = state
        .blob
        .stream_upload(&blob_key, content_type, field, MAX_DOCUMENT_BYTES)
        .await?;
    Ok((blob_key, file_size_bytes as i64))
}

/// Inserts the document row for an already-blob-stored upload and queues its
/// OCR pass — shared by `create` below and `web::handlers::scan::submit_scan`
/// (phone camera capture, feature 009), both calling this right after
/// `stream_document_to_blob` above. OCR-eligibility is always decided by the
/// shared `ocr_eligible` helper, even though feature 008's desktop upload
/// and feature 009's phone capture validate `content_type` against their
/// own, differently-scoped accepted sets before ever reaching here.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_document_and_queue_ocr(
    state: &AppState,
    tenant_id: Uuid,
    user_id: Uuid,
    document_id: Uuid,
    blob_key: String,
    original_filename: String,
    content_type: String,
    file_size_bytes: i64,
    title: Option<String>,
    tags: Vec<String>,
    date_issued: Option<time::Date>,
) -> Result<(), AppWebError> {
    let is_ocr_eligible = ocr_eligible(&content_type);
    let initial_ocr_status = if is_ocr_eligible { "pending" } else { "skipped" };

    sqlx::query!(
        "insert into documents
            (id, tenant_id, user_id, original_filename, title, content_type, file_size_bytes, blob_key, tags, date_issued, ocr_status)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        document_id,
        tenant_id,
        user_id,
        original_filename,
        title,
        content_type,
        file_size_bytes,
        blob_key,
        &tags,
        date_issued,
        initial_ocr_status,
    )
    .execute(&state.pool)
    .await?;

    if is_ocr_eligible {
        let ocr_state = state.clone();
        let key = blob_key.clone();
        let ct = content_type.clone();
        tokio::spawn(run_ocr(ocr_state, document_id, tenant_id, key, ct).instrument(tracing::Span::current()));
    }

    Ok(())
}

#[tracing::instrument(skip(state, tenancy, multipart))]
pub async fn create(
    tenancy: TenantContext,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, AppWebError> {
    let mut title = ProfileField::default();
    let mut tags = Tags::default();
    let mut date_issued = DateIssuedField::default();
    // (blob_key, original_filename, content_type, file_size_bytes) — the DB
    // insert is deferred until after the whole multipart body has validated
    // (see `stream_document_to_blob`'s doc comment), so this just remembers
    // what to insert once we know no later field will reject the request.
    let mut uploaded: Option<(String, String, String, i64)> = None;
    let document_id = Uuid::new_v4();

    while let Some(field) = multipart.next_field().await? {
        let Some(name) = field.name().map(str::to_string) else {
            continue;
        };

        // Metadata fields must arrive before "file" in the multipart body —
        // enforced explicitly (400) rather than silently dropped, since the
        // file field is streamed straight to S3 as soon as it's reached
        // (so the blob key doesn't depend on metadata) and nothing here
        // buffers the whole request to re-order fields after the fact.
        if uploaded.is_some() && name != "file" {
            return Ok(bad_request("metadata fields must be submitted before the file field"));
        }

        match name.as_str() {
            "title" => match parse_metadata_field(field.text().await?, "title is too long") {
                Ok(value) => title = value,
                Err(message) => return Ok(bad_request(message)),
            },
            "tags" => match parse_metadata_field(field.text().await?, "invalid tags") {
                Ok(value) => tags = value,
                Err(message) => return Ok(bad_request(message)),
            },
            "date_issued" => {
                match parse_metadata_field(field.text().await?, "date issued must be blank or YYYY-MM-DD") {
                    Ok(value) => date_issued = value,
                    Err(message) => return Ok(bad_request(message)),
                }
            }
            "file" => {
                if uploaded.is_some() {
                    return Ok(bad_request("only one file may be uploaded"));
                }
                let content_type = field.content_type().unwrap_or("application/octet-stream").to_string();
                let original_filename = field.file_name().unwrap_or("upload").to_string();
                if !OCR_ELIGIBLE_CONTENT_TYPES.contains(&content_type.as_str())
                    && !OTHER_ACCEPTED_CONTENT_TYPES.contains(&content_type.as_str())
                {
                    return Ok(bad_request("unsupported file type"));
                }
                let (blob_key, file_size_bytes) =
                    stream_document_to_blob(&state, tenancy.user_id.0, document_id, &content_type, field).await?;
                uploaded = Some((blob_key, original_filename, content_type, file_size_bytes));
            }
            _ => {}
        }
    }

    let Some((blob_key, original_filename, content_type, file_size_bytes)) = uploaded else {
        return Ok(bad_request("no file provided"));
    };

    insert_document_and_queue_ocr(
        &state,
        tenancy.tenant_id.0,
        tenancy.user_id.0,
        document_id,
        blob_key,
        original_filename,
        content_type,
        file_size_bytes,
        title.into_option(),
        tags.into_vec(),
        date_issued.into_option(),
    )
    .await?;

    Ok(Redirect::to(&format!("/documents/{document_id}?uploaded=true")).into_response())
}
