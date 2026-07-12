use axum::extract::{Form, Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use uuid::Uuid;

use crate::web::error::AppWebError;
use crate::web::forms::{DateIssuedField, ProfileField, Tags};
use crate::web::nav;
use crate::web::state::AppState;
use crate::web::tenancy::TenantContext;
use crate::web::templates::{DocumentListItem, DocumentShowTemplate, DocumentsListTemplate};

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
