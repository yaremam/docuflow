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
use crate::web::templates::{DocumentListItem, DocumentNewTemplate, DocumentShowTemplate, DocumentsListTemplate};

/// Also used by the router to size its `DefaultBodyLimit` layer — kept as
/// one constant so the two enforcement points (that layer, and the
/// mid-stream check in `BlobStore::stream_upload`) can't drift apart.
/// Larger than `profile::MAX_PICTURE_BYTES` (8MB) — scanned bills/policies
/// run bigger than an avatar photo.
pub const MAX_DOCUMENT_BYTES: usize = 20 * 1024 * 1024;

/// Content types accepted at all. Images in this set get real OCR; `PDF`
/// is accepted and stored but not OCR'd yet (rasterizing PDF pages is a
/// larger follow-up feature) — its rows are inserted with
/// `ocr_status = 'skipped'`, matching the placeholder copy already in
/// `document_show.html`. Everything else is rejected with 400.
const OCR_ELIGIBLE_CONTENT_TYPES: &[&str] = &["image/jpeg", "image/png", "image/tiff", "image/webp"];
const OTHER_ACCEPTED_CONTENT_TYPES: &[&str] = &["application/pdf"];

fn format_date(date: time::Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year(), date.month() as u8, date.day())
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
                r#"select id, title, original_filename, tags, date_issued, ocr_status, created_at
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
                r#"select id, title, original_filename, tags, date_issued, ocr_status, created_at
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
                r#"select id, title, original_filename, tags, date_issued, ocr_status, created_at
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
                r#"select id, title, original_filename, tags, date_issued, ocr_status, created_at
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
                r#"select id, title, original_filename, tags, date_issued, ocr_status, created_at
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

    let documents = rows
        .into_iter()
        .map(|row| DocumentListItem {
            id: row.id,
            title: row.title.unwrap_or_else(|| row.original_filename.clone()),
            original_filename: row.original_filename,
            tags: row.tags,
            date_issued: row.date_issued.map(format_date),
            uploaded_at: format_date(row.created_at.date()),
            ocr_status: row.ocr_status,
        })
        .collect();

    Ok(DocumentsListTemplate {
        active_tab: "documents",
        authenticated: true,
        nav_avatar_url,
        q: query.q,
        sort: sort.as_str(),
        documents,
    })
}

struct DocumentRow {
    id: Uuid,
    title: Option<String>,
    original_filename: String,
    content_type: String,
    file_size_bytes: i64,
    tags: Vec<String>,
    date_issued: Option<time::Date>,
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
        r#"select id, title, original_filename, content_type, file_size_bytes, tags, date_issued, ocr_status, ocr_text, created_at
           from documents
           where id = $1 and tenant_id = $2"#,
        id,
        tenancy.tenant_id.0,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppWebError::NotFound)?;

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
        tags_input_value: row.tags.join(", "),
        date_issued_input_value: row.date_issued.map(format_date).unwrap_or_default(),
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

fn bad_request(message: &'static str) -> Response {
    (StatusCode::BAD_REQUEST, message).into_response()
}

/// Parses a multipart text field's value into a validated form newtype,
/// shared by `create`'s title/tags/date_issued arms below — these don't go
/// through the `Form` extractor's automatic serde validation (multipart
/// bodies mix text fields with a file field), so each has to call
/// `TryFrom<String>` by hand; this just avoids repeating the "match, keep
/// value or bail with 400" shape three times.
fn parse_metadata_field<T: TryFrom<String>>(text: String, error_message: &'static str) -> Result<T, Response> {
    T::try_from(text).map_err(|_| bad_request(error_message))
}

/// Runs the OCR pass for a just-uploaded, OCR-eligible document as detached
/// background work — spawned by `create` below, never awaited by the
/// request. Errors are caught and turned into `ocr_status = 'failed'`
/// locally; there's no request left to propagate an `AppWebError` to.
#[tracing::instrument(skip(state))]
async fn run_ocr(state: AppState, document_id: Uuid, tenant_id: Uuid, blob_key: String) {
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
        Ok(bytes) => crate::ocr::extract_text(&bytes).await.map_err(|e| e.to_string()),
        Err(error) => Err(error.to_string()),
    };

    let update_result = match outcome {
        Ok(text) => {
            sqlx::query!(
                "update documents set ocr_status = 'done', ocr_text = $3 where id = $1 and tenant_id = $2",
                document_id,
                tenant_id,
                text,
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

#[tracing::instrument(skip(state, tenancy, multipart))]
pub async fn create(
    tenancy: TenantContext,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, AppWebError> {
    let mut title = ProfileField::default();
    let mut tags = Tags::default();
    let mut date_issued = DateIssuedField::default();
    let mut uploaded: Option<(String, String, i64)> = None; // (content_type, original_filename, file_size_bytes)
    let document_id = Uuid::new_v4();
    let blob_key = format!("documents/{}/{document_id}", tenancy.user_id.0);

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
                Err(response) => return Ok(response),
            },
            "tags" => match parse_metadata_field(field.text().await?, "invalid tags") {
                Ok(value) => tags = value,
                Err(response) => return Ok(response),
            },
            "date_issued" => {
                match parse_metadata_field(field.text().await?, "date issued must be blank or YYYY-MM-DD") {
                    Ok(value) => date_issued = value,
                    Err(response) => return Ok(response),
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
                let file_size_bytes = state
                    .blob
                    .stream_upload(&blob_key, &content_type, field, MAX_DOCUMENT_BYTES)
                    .await?;
                uploaded = Some((content_type, original_filename, file_size_bytes as i64));
            }
            _ => {}
        }
    }

    let Some((content_type, original_filename, file_size_bytes)) = uploaded else {
        return Ok(bad_request("no file provided"));
    };

    let ocr_eligible = OCR_ELIGIBLE_CONTENT_TYPES.contains(&content_type.as_str());
    let initial_ocr_status = if ocr_eligible { "pending" } else { "skipped" };

    sqlx::query!(
        "insert into documents
            (id, tenant_id, user_id, original_filename, title, content_type, file_size_bytes, blob_key, tags, date_issued, ocr_status)
         values ($1, $2, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        document_id,
        tenancy.tenant_id.0,
        original_filename,
        title.into_option(),
        content_type,
        file_size_bytes,
        blob_key,
        &tags.into_vec(),
        date_issued.into_option(),
        initial_ocr_status,
    )
    .execute(&state.pool)
    .await?;

    if ocr_eligible {
        let ocr_state = state.clone();
        let tenant_id = tenancy.tenant_id.0;
        let key = blob_key.clone();
        tokio::spawn(run_ocr(ocr_state, document_id, tenant_id, key).instrument(tracing::Span::current()));
    }

    Ok(Redirect::to(&format!("/documents/{document_id}?uploaded=true")).into_response())
}
