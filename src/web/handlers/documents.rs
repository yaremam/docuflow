use axum::extract::{Form, Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
/// `axum::extract::Query` (backed by `serde_urlencoded`) can't collect
/// repeated same-named params (`tags=a&tags=b`) into a `Vec<String>` field
/// — it only ever sees the last occurrence. `list`'s facet params need
/// exactly that, so it alone uses `axum-extra`'s `Query` (backed by
/// `serde_html_form`), which supports it; every other handler in this
/// file keeps the standard extractor.
use axum_extra::extract::Query as MultiQuery;
/// Same reasoning as `MultiQuery` above, but for POST bodies: bulk actions
/// (feature 026) submit repeated `doc_ids=...` fields (one per checked
/// row), which `axum::extract::Form` (`serde_urlencoded`) can't collect
/// into a `Vec<Uuid>` — only `axum_extra::extract::Form`
/// (`serde_html_form`) supports that.
use axum_extra::extract::Form as MultiForm;
use serde::Deserialize;
use tracing::Instrument;
use uuid::Uuid;

use crate::web::error::AppWebError;
use crate::web::facets::{assemble_facet_options, ActiveFilters};
use crate::web::forms::{CollectionName, DateIssuedField, DocTypeField, Language, ProfileField, Tags};
use crate::web::nav;
use crate::web::state::AppState;
use crate::web::tenancy::TenantContext;
use crate::web::templates::{
    AppliedFilterChip, BulkDeleteDocumentSummary, CollectionOption, DocTypeFacetOption, DocumentBulkDeleteConfirmTemplate,
    DocumentDeleteTemplate, DocumentListItem, DocumentNewTemplate, DocumentShowTemplate, DocumentsListTemplate, DuplicateMatch,
    LanguageFacetOption, MonthFacetOption, TagFacetOption, YearFacetOption,
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

/// The derived blob key a document's generated thumbnail (feature 025) is
/// stored under — a fixed suffix on the original `blob_key`, so no extra
/// column is needed to find it (only `thumbnail_status` tracks whether it
/// exists yet).
fn thumbnail_blob_key(blob_key: &str) -> String {
    format!("{blob_key}-thumb")
}

/// A presigned view URL plus whether the browser can inline it as an
/// `<img>` (vs. needing a PDF `<embed>`) — shared by `list` and `show` since
/// both render a preview from the same `blob_key`/`content_type` pair.
/// Reuses `OCR_ELIGIBLE_CONTENT_TYPES` for the image check: that constant is
/// named for OCR eligibility, but today's four image types are exactly the
/// set a browser can inline too, so it doubles as the "is this an image"
/// answer without a second, parallel list to keep in sync.
async fn document_preview(blob: &crate::blob::BlobStore, blob_key: &str, content_type: &str) -> Result<(String, bool), AppWebError> {
    let file_url = blob.presigned_get_url(blob_key).await?;
    let is_image = OCR_ELIGIBLE_CONTENT_TYPES.contains(&content_type);
    Ok((file_url, is_image))
}

/// A presigned URL for a document's generated thumbnail (feature 025),
/// only once one exists — called only by `list`, which renders this small
/// preview instead of the full-size one `document_preview` above returns;
/// `show` renders the full-size original and has no use for this, so it's
/// a separate narrow helper rather than a wider `document_preview` every
/// caller pays for.
async fn thumbnail_preview_url(
    blob: &crate::blob::BlobStore,
    blob_key: &str,
    thumbnail_status: Option<&str>,
) -> Result<Option<String>, AppWebError> {
    if thumbnail_status != Some("done") {
        return Ok(None);
    }
    Ok(Some(blob.presigned_get_url(&thumbnail_blob_key(blob_key)).await?))
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

/// `show`'s own free-text half of its `q` (feature 027 highlighting) —
/// `list`'s equivalent lives behind `ActiveFilters::search_text` (its
/// comma-split tag-overlap half behind `ActiveFilters::q_tags`), but
/// `show` has no facets to build, so it just calls this directly.
/// `'simple'` text search config deliberately, not `'english'` — this
/// app OCRs German/Dutch/Ukrainian/Cyrillic text too (feature 020), and
/// `'english'` would run every token through the English stemmer
/// regardless of actual language.
fn free_text_search(q: &str) -> Option<&str> {
    let trimmed = q.trim();
    if trimmed.is_empty() { None } else { Some(trimmed) }
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
    doc_type: Option<String>,
    thumbnail_status: Option<String>,
    created_at: time::OffsetDateTime,
    /// A `ts_headline`-marked excerpt (feature 027), only when this row's
    /// *OCR text* — not merely its tags — matched the active free-text
    /// search; `render_marked` below turns the control-character markers
    /// this carries into safe `<mark>`-wrapped HTML.
    ocr_snippet: Option<String>,
}

/// `Default` lets a saved collection's stored query string (feature 016)
/// fail safe to "no filter" rather than panicking if it's ever malformed
/// (shouldn't happen — `save_collection` validates with this same type —
/// but zero-panic per CLAUDE.md means this path can't `.unwrap()`).
#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    #[serde(default)]
    q: String,
    sort: Option<String>,
    #[serde(default)]
    deleted: bool,
    /// Smart-filters tag facet (feature 015) — AND-narrowing, independent
    /// of `q`'s OR-search (see TDR 015 §3).
    #[serde(default)]
    tags: Vec<String>,
    date_year: Option<i32>,
    /// Only respected when `date_year` is also set (TDR 015 §3).
    date_month: Option<i32>,
    #[serde(default)]
    undated: bool,
    /// `"en"` / `"cyr"` / `"unset"` — OR-combined within this facet.
    #[serde(default)]
    lang: Vec<String>,
    /// One of `doc_type_extract::dropdown_options()`'s values, or
    /// `"unset"` — OR-combined within this facet, same shape as `lang`
    /// (feature 024; see TDR 024 §3 on why this mirrors `language`, not
    /// `date_issued`).
    #[serde(default)]
    doc_type: Vec<String>,
}

const MONTH_NAMES: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn month_name(month: i32) -> &'static str {
    MONTH_NAMES.get((month - 1).max(0) as usize).copied().unwrap_or("")
}

fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Builds the query-string portion (no leading `/documents?`) from a full
/// set of filter state — shared by `build_documents_url` below and by
/// `list`'s "Save this search" hidden field (feature 016), so a saved
/// collection's stored `query` is byte-for-byte the same shape as any
/// other filter link this page produces. `sort` is taken separately from
/// `active` — it's display-order state, not a filter dimension `count_
/// documents`/`count_matching_documents` ever need (see `ActiveFilters`'
/// own doc comment).
fn build_query_string(active: &ActiveFilters, sort: &str) -> String {
    let mut params: Vec<(&str, String)> = Vec::new();
    if !active.q.is_empty() {
        params.push(("q", active.q.clone()));
    }
    params.push(("sort", sort.to_string()));
    for tag in &active.tags {
        params.push(("tags", tag.clone()));
    }
    if let Some(year) = active.date_year {
        params.push(("date_year", year.to_string()));
    }
    if let Some(month) = active.date_month {
        params.push(("date_month", month.to_string()));
    }
    if active.undated {
        params.push(("undated", "true".to_string()));
    }
    for value in &active.lang {
        params.push(("lang", value.clone()));
    }
    for value in &active.doc_type {
        params.push(("doc_type", value.clone()));
    }

    params.iter().map(|(key, value)| format!("{key}={}", url_encode(value))).collect::<Vec<_>>().join("&")
}

/// Builds a full `/documents?...` URL from a full set of filter state —
/// shared by the applied-filter chips' individual "remove this one" links
/// and "Clear all", so both stay in sync with the query params `list`
/// itself parses (see TDR 015 §3).
fn build_documents_url(active: &ActiveFilters, sort: &str) -> String {
    let query_string = build_query_string(active, sort);
    if query_string.is_empty() {
        "/documents".to_string()
    } else {
        format!("/documents?{query_string}")
    }
}

/// Normalizes a `ListQuery` into `ActiveFilters` (feature 027/028) — the
/// one place `list`, `count_matching_documents`, and `save_collection` all
/// build this from the same raw query-param shape, so they can't drift.
fn active_filters(query: &ListQuery) -> ActiveFilters {
    ActiveFilters::new(query.q.clone(), query.tags.clone(), query.date_year, query.date_month, query.undated, query.lang.clone(), query.doc_type.clone())
}

/// The "no dimension narrowed" `FacetFilters` view of the request's
/// current `ActiveFilters` — every one of `count_documents`'s ~9 call
/// sites starts here and overrides just the one field it's narrowing by
/// (TDR 028 §3), rather than each hand-writing all 10 fields itself.
fn base_facet_filters(active: &ActiveFilters) -> FacetFilters<'_> {
    FacetFilters {
        q_tags: active.q_tags(),
        facet_tags: &active.tags,
        date_year: active.date_year,
        date_month: active.date_month,
        undated: active.undated,
        lang_values: active.lang_values(),
        lang_unset: active.lang_unset(),
        search_text: active.search_text(),
        doc_type_values: active.doc_type_values(),
        doc_type_unset: active.doc_type_unset(),
    }
}

/// Counts documents matching an arbitrary filter set — the same
/// conditions `list`'s main query applies, minus any `ORDER BY` (a count
/// needs no ordering). Used both for a saved collection's live count
/// (feature 016) and could back a future "how many would this match"
/// preview; kept as its own query rather than folded into `list`'s
/// per-sort arms since it's needed once per collection, not once per
/// page load.
/// The raw multi-dimension `WHERE` clause shared by every "how many
/// documents match this filter combination" question on `/documents` —
/// a saved collection's full count (`count_matching_documents`) and,
/// since feature 018, each individual facet option's narrowed count
/// (`list` calls this directly, once per candidate, with that facet's
/// own dimension pinned to a single value and every other dimension left
/// at whatever's currently active — see TDR 018 §3).
/// The full set of facet dimensions `count_documents` narrows by, as
/// named fields rather than positional arguments. Three of these
/// dimensions are structurally identical `(&[String], bool)` "values +
/// unset-sentinel" pairs (`lang_values`/`lang_unset` and
/// `doc_type_values`/`doc_type_unset`, plus the lone `undated: bool`
/// alongside `date_year`/`date_month`) — a positional call site can't
/// visually distinguish them, so a future 4th facet could silently swap
/// with an existing one and still compile. A struct literal forces every
/// call site to name which value goes to which dimension instead.
struct FacetFilters<'a> {
    q_tags: Option<&'a [String]>,
    facet_tags: &'a [String],
    date_year: Option<i32>,
    date_month: Option<i32>,
    undated: bool,
    lang_values: &'a [String],
    lang_unset: bool,
    search_text: Option<&'a str>,
    doc_type_values: &'a [String],
    doc_type_unset: bool,
}

async fn count_documents(state: &AppState, tenant_id: Uuid, facets: FacetFilters<'_>) -> Result<i64, AppWebError> {
    let count = sqlx::query_scalar!(
        "select count(*) from documents
         where tenant_id = $1
           and (($2::text[] is null or tags && $2)
                or ($9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)))
           and (cardinality($3::text[]) = 0 or tags @> $3)
           and (($4::int4 is null and $6 = false)
                or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                    and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                or ($6 and date_issued is null))
           and ((cardinality($7::text[]) = 0 and $8 = false)
                or (language = any($7))
                or ($8 and language is null))
           and ((cardinality($10::text[]) = 0 and $11 = false)
                or (doc_type = any($10))
                or ($11 and doc_type is null))",
        tenant_id,
        facets.q_tags,
        facets.facet_tags,
        facets.date_year,
        facets.date_month,
        facets.undated,
        facets.lang_values,
        facets.lang_unset,
        facets.search_text,
        facets.doc_type_values,
        facets.doc_type_unset,
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(0);

    Ok(count)
}

/// A collection's *complete* saved filter, all dimensions at once — not
/// one facet option narrowed by the others (see `count_documents` and
/// TDR 018 §1 for why those are different questions).
async fn count_matching_documents(state: &AppState, tenant_id: Uuid, filters: &ListQuery) -> Result<i64, AppWebError> {
    let active = active_filters(filters);
    count_documents(state, tenant_id, base_facet_filters(&active)).await
}

#[tracing::instrument(skip(state, tenancy, query))]
pub async fn list(
    tenancy: TenantContext,
    State(state): State<AppState>,
    MultiQuery(query): MultiQuery<ListQuery>,
) -> Result<DocumentsListTemplate, AppWebError> {
    let nav_avatar_url = nav::avatar_url(&state.pool, &state.blob, tenancy.user_id.0).await?;
    let sort = Sort::parse(query.sort.as_deref());
    let active = active_filters(&query);

    // Each arm differs only in `ORDER BY` — sqlx's compile-time `query_as!`
    // macro can't parameterize that clause, so the small, fixed set of sort
    // modes is spelled out literally rather than building the SQL string at
    // runtime (which would forgo compile-time verification for every query
    // on this page, not just the ordering). The facet conditions ($3-$11)
    // are identical across all five arms — see TDR 015 §3 for what each
    // one means (§7's log covers $9's free-text search, feature 023;
    // $10/$11's doc_type facet, feature 024; $12 is `ts_headline`'s options
    // string reusing $9's tsquery to build the `ocr_snippet` column,
    // feature 027 — see TDR 027 §3).
    let rows = match sort {
        Sort::CreatedAtDesc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, doc_type, thumbnail_status, created_at,
                          case when $9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)
                               then ts_headline('simple', ocr_text, websearch_to_tsquery('simple', $9), $12)
                          end as ocr_snippet
                   from documents
                   where tenant_id = $1
                     and (($2::text[] is null or tags && $2)
                          or ($9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)))
                     and (cardinality($3::text[]) = 0 or tags @> $3)
                     and (($4::int4 is null and $6 = false)
                          or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                              and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                          or ($6 and date_issued is null))
                     and ((cardinality($7::text[]) = 0 and $8 = false)
                          or (language = any($7))
                          or ($8 and language is null))
                     and ((cardinality($10::text[]) = 0 and $11 = false)
                          or (doc_type = any($10))
                          or ($11 and doc_type is null))
                   order by created_at desc"#,
                tenancy.tenant_id.0,
                active.q_tags(),
                active.tags.as_slice(),
                active.date_year,
                active.date_month,
                active.undated,
                active.lang_values(),
                active.lang_unset(),
                active.search_text(),
                active.doc_type_values(),
                active.doc_type_unset(),
                crate::highlight::SNIPPET_OPTIONS,
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::CreatedAtAsc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, doc_type, thumbnail_status, created_at,
                          case when $9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)
                               then ts_headline('simple', ocr_text, websearch_to_tsquery('simple', $9), $12)
                          end as ocr_snippet
                   from documents
                   where tenant_id = $1
                     and (($2::text[] is null or tags && $2)
                          or ($9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)))
                     and (cardinality($3::text[]) = 0 or tags @> $3)
                     and (($4::int4 is null and $6 = false)
                          or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                              and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                          or ($6 and date_issued is null))
                     and ((cardinality($7::text[]) = 0 and $8 = false)
                          or (language = any($7))
                          or ($8 and language is null))
                     and ((cardinality($10::text[]) = 0 and $11 = false)
                          or (doc_type = any($10))
                          or ($11 and doc_type is null))
                   order by created_at asc"#,
                tenancy.tenant_id.0,
                active.q_tags(),
                active.tags.as_slice(),
                active.date_year,
                active.date_month,
                active.undated,
                active.lang_values(),
                active.lang_unset(),
                active.search_text(),
                active.doc_type_values(),
                active.doc_type_unset(),
                crate::highlight::SNIPPET_OPTIONS,
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::DateIssuedDesc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, doc_type, thumbnail_status, created_at,
                          case when $9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)
                               then ts_headline('simple', ocr_text, websearch_to_tsquery('simple', $9), $12)
                          end as ocr_snippet
                   from documents
                   where tenant_id = $1
                     and (($2::text[] is null or tags && $2)
                          or ($9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)))
                     and (cardinality($3::text[]) = 0 or tags @> $3)
                     and (($4::int4 is null and $6 = false)
                          or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                              and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                          or ($6 and date_issued is null))
                     and ((cardinality($7::text[]) = 0 and $8 = false)
                          or (language = any($7))
                          or ($8 and language is null))
                     and ((cardinality($10::text[]) = 0 and $11 = false)
                          or (doc_type = any($10))
                          or ($11 and doc_type is null))
                   order by date_issued desc nulls last"#,
                tenancy.tenant_id.0,
                active.q_tags(),
                active.tags.as_slice(),
                active.date_year,
                active.date_month,
                active.undated,
                active.lang_values(),
                active.lang_unset(),
                active.search_text(),
                active.doc_type_values(),
                active.doc_type_unset(),
                crate::highlight::SNIPPET_OPTIONS,
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::DateIssuedAsc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, doc_type, thumbnail_status, created_at,
                          case when $9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)
                               then ts_headline('simple', ocr_text, websearch_to_tsquery('simple', $9), $12)
                          end as ocr_snippet
                   from documents
                   where tenant_id = $1
                     and (($2::text[] is null or tags && $2)
                          or ($9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)))
                     and (cardinality($3::text[]) = 0 or tags @> $3)
                     and (($4::int4 is null and $6 = false)
                          or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                              and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                          or ($6 and date_issued is null))
                     and ((cardinality($7::text[]) = 0 and $8 = false)
                          or (language = any($7))
                          or ($8 and language is null))
                     and ((cardinality($10::text[]) = 0 and $11 = false)
                          or (doc_type = any($10))
                          or ($11 and doc_type is null))
                   order by date_issued asc nulls last"#,
                tenancy.tenant_id.0,
                active.q_tags(),
                active.tags.as_slice(),
                active.date_year,
                active.date_month,
                active.undated,
                active.lang_values(),
                active.lang_unset(),
                active.search_text(),
                active.doc_type_values(),
                active.doc_type_unset(),
                crate::highlight::SNIPPET_OPTIONS,
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::TagsAsc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, doc_type, thumbnail_status, created_at,
                          case when $9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)
                               then ts_headline('simple', ocr_text, websearch_to_tsquery('simple', $9), $12)
                          end as ocr_snippet
                   from documents
                   where tenant_id = $1
                     and (($2::text[] is null or tags && $2)
                          or ($9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)))
                     and (cardinality($3::text[]) = 0 or tags @> $3)
                     and (($4::int4 is null and $6 = false)
                          or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                              and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                          or ($6 and date_issued is null))
                     and ((cardinality($7::text[]) = 0 and $8 = false)
                          or (language = any($7))
                          or ($8 and language is null))
                     and ((cardinality($10::text[]) = 0 and $11 = false)
                          or (doc_type = any($10))
                          or ($11 and doc_type is null))
                   order by array_to_string(tags, ',') asc"#,
                tenancy.tenant_id.0,
                active.q_tags(),
                active.tags.as_slice(),
                active.date_year,
                active.date_month,
                active.undated,
                active.lang_values(),
                active.lang_unset(),
                active.search_text(),
                active.doc_type_values(),
                active.doc_type_unset(),
                crate::highlight::SNIPPET_OPTIONS,
            )
            .fetch_all(&state.pool)
            .await?
        }
    };

    let mut documents = Vec::new();
    for row in rows {
        let thumbnail_url = thumbnail_preview_url(&state.blob, &row.blob_key, row.thumbnail_status.as_deref()).await?;
        // `file_url`/`is_image` are only ever rendered by the template's
        // fallback branch, once there's no `thumbnail_url` — skip
        // presigning (and the allocation that goes with it) for the now-
        // common case where a thumbnail already covers this row.
        let (file_url, is_image) = if thumbnail_url.is_none() {
            document_preview(&state.blob, &row.blob_key, &row.content_type).await?
        } else {
            (String::new(), false)
        };
        let doc_type_label = row.doc_type.as_deref().and_then(crate::doc_type_extract::label_for);
        documents.push(DocumentListItem {
            id: row.id,
            title: row.title.unwrap_or_else(|| row.original_filename.clone()),
            original_filename: row.original_filename,
            file_url,
            is_image,
            thumbnail_url,
            tags: row.tags,
            date_issued: row.date_issued.map(format_date),
            uploaded_at: format_date(row.created_at.date()),
            ocr_status: row.ocr_status,
            doc_type_label,
            ocr_snippet_html: row.ocr_snippet.as_deref().map(crate::highlight::render_marked),
        });
    }

    // The *candidate set* for each facet (which tags/years/months/languages
    // exist at all) stays the tenant's unfiltered full set — only each
    // candidate's displayed *count*, fetched separately below via
    // `count_documents`, narrows by whichever other facets are currently
    // active (TDR 018 §3, AC-5). Since feature 020 opened `language` up to
    // any ISO 639-1 code, its candidate set is no longer fixed either — it
    // needs the same discover-then-narrow-count pattern as tags below,
    // rather than the 3 hardcoded `en`/`cyr`/`unset` queries feature 018
    // originally wrote.
    let tag_rows = sqlx::query!(
        "select unnest(tags) as tag, count(*) as count from documents where tenant_id = $1 group by tag order by count(*) desc, tag limit 10",
        tenancy.tenant_id.0,
    )
    .fetch_all(&state.pool)
    .await?;
    let tag_candidates: Vec<String> = tag_rows.into_iter().map(|row| row.tag.unwrap_or_default()).collect();
    let mut tag_counts = Vec::with_capacity(tag_candidates.len());
    for name in tag_candidates {
        let count = count_documents(
            &state,
            tenancy.tenant_id.0,
            FacetFilters { facet_tags: std::slice::from_ref(&name), ..base_facet_filters(&active) },
        )
        .await?;
        tag_counts.push((name, count));
    }
    let tag_facets = assemble_facet_options(
        tag_counts,
        |name| active.tags.iter().any(|tag| tag == name),
        |name, count, checked| TagFacetOption { name, count, checked },
    );

    let year_rows = sqlx::query!(
        "select extract(year from date_issued)::int4 as year, count(*) as count
         from documents where tenant_id = $1 and date_issued is not null
         group by year order by year desc",
        tenancy.tenant_id.0,
    )
    .fetch_all(&state.pool)
    .await?;
    let year_candidates: Vec<i32> = year_rows.into_iter().map(|row| row.year.unwrap_or(0)).collect();
    let mut year_counts = Vec::with_capacity(year_candidates.len());
    for year in year_candidates {
        let count = count_documents(
            &state,
            tenancy.tenant_id.0,
            FacetFilters { date_year: Some(year), date_month: None, undated: false, ..base_facet_filters(&active) },
        )
        .await?;
        year_counts.push((year, count));
    }
    let mut year_facets = assemble_facet_options(
        year_counts,
        |year| active.date_year == Some(*year),
        |year, count, checked| YearFacetOption { year, count, checked, months: Vec::new() },
    );

    // Months only ever exist nested under the single checked year (there's
    // at most one), so they're fetched/narrowed once here rather than
    // once per year candidate, then attached to whichever `YearFacetOption`
    // came back checked.
    if let Some(year) = active.date_year {
        let month_rows = sqlx::query!(
            "select extract(month from date_issued)::int4 as month, count(*) as count
             from documents where tenant_id = $1 and extract(year from date_issued)::int4 = $2
             group by month order by month",
            tenancy.tenant_id.0,
            year,
        )
        .fetch_all(&state.pool)
        .await?;
        let month_candidates: Vec<i32> = month_rows.into_iter().map(|row| row.month.unwrap_or(0)).collect();
        let mut month_counts = Vec::with_capacity(month_candidates.len());
        for month in month_candidates {
            let count = count_documents(
                &state,
                tenancy.tenant_id.0,
                FacetFilters { date_year: Some(year), date_month: Some(month), undated: false, ..base_facet_filters(&active) },
            )
            .await?;
            month_counts.push((month, count));
        }
        let month_facets = assemble_facet_options(
            month_counts,
            |month| active.date_month == Some(*month),
            |month, count, checked| MonthFacetOption { label: month_name(month), value: month.clamp(0, 12) as u8, count, checked },
        );
        if let Some(checked_year) = year_facets.iter_mut().find(|year_option| year_option.checked) {
            checked_year.months = month_facets;
        }
    }

    let undated_count = count_documents(
        &state,
        tenancy.tenant_id.0,
        FacetFilters { date_year: None, date_month: None, undated: true, ..base_facet_filters(&active) },
    )
    .await?;

    let language_rows = sqlx::query!(
        "select distinct language from documents where tenant_id = $1 and language is not null order by language",
        tenancy.tenant_id.0,
    )
    .fetch_all(&state.pool)
    .await?;
    let language_candidates: Vec<String> = language_rows.into_iter().filter_map(|row| row.language).collect();
    let mut language_counts = Vec::with_capacity(language_candidates.len());
    for code in language_candidates {
        let count = count_documents(
            &state,
            tenancy.tenant_id.0,
            FacetFilters { lang_values: std::slice::from_ref(&code), lang_unset: false, ..base_facet_filters(&active) },
        )
        .await?;
        language_counts.push((code, count));
    }
    let mut language_facets = assemble_facet_options(
        language_counts,
        |code| active.lang_values().iter().any(|v| v == code),
        |code, count, checked| LanguageFacetOption { label: crate::languages::display_name(&code), value: code, count, checked },
    );
    let unset_count = count_documents(
        &state,
        tenancy.tenant_id.0,
        FacetFilters { lang_values: &[], lang_unset: true, ..base_facet_filters(&active) },
    )
    .await?;
    language_facets.push(LanguageFacetOption {
        value: "unset".to_string(),
        label: "Not set".to_string(),
        count: unset_count,
        checked: active.lang_unset(),
    });

    // Document type facet (feature 024) — same discover-then-narrow-count
    // shape as the Language facet just above: candidate set is the
    // tenant's distinct doc_type values (unfiltered), each candidate's
    // count narrows by whichever other facets are active, plus a
    // trailing "Not set" option.
    let doc_type_rows = sqlx::query!(
        "select distinct doc_type from documents where tenant_id = $1 and doc_type is not null order by doc_type",
        tenancy.tenant_id.0,
    )
    .fetch_all(&state.pool)
    .await?;
    let doc_type_candidates: Vec<String> = doc_type_rows.into_iter().filter_map(|row| row.doc_type).collect();
    let mut doc_type_counts = Vec::with_capacity(doc_type_candidates.len());
    for value in doc_type_candidates {
        let count = count_documents(
            &state,
            tenancy.tenant_id.0,
            FacetFilters { doc_type_values: std::slice::from_ref(&value), doc_type_unset: false, ..base_facet_filters(&active) },
        )
        .await?;
        doc_type_counts.push((value, count));
    }
    let mut doc_type_facets = assemble_facet_options(
        doc_type_counts,
        |value| active.doc_type_values().iter().any(|v| v == value),
        |value, count, checked| {
            let label = crate::doc_type_extract::label_for(&value).map(str::to_string).unwrap_or_else(|| value.clone());
            DocTypeFacetOption { value, label, count, checked }
        },
    );
    let doc_type_unset_count = count_documents(
        &state,
        tenancy.tenant_id.0,
        FacetFilters { doc_type_values: &[], doc_type_unset: true, ..base_facet_filters(&active) },
    )
    .await?;
    doc_type_facets.push(DocTypeFacetOption {
        value: "unset".to_string(),
        label: "Not set".to_string(),
        count: doc_type_unset_count,
        checked: active.doc_type_unset(),
    });

    // Each chip is "the current active state, with just this one value
    // removed" — built by cloning `active` and mutating its raw fields
    // directly. Safe here specifically because `build_documents_url`/
    // `build_query_string` only ever read those raw fields, never the
    // derived ones (`q_tags`/`lang_values`/...), which a raw-field mutation
    // doesn't recompute — a mutated clone must never be passed to
    // `count_documents`/`base_facet_filters` for that reason.
    let mut applied_filters: Vec<AppliedFilterChip> = Vec::new();
    for tag in &active.tags {
        let mut remaining = active.clone();
        remaining.tags.retain(|t| t != tag);
        applied_filters.push(AppliedFilterChip { label: tag.clone(), remove_href: build_documents_url(&remaining, sort.as_str()) });
    }
    if let Some(year) = active.date_year {
        let label = match active.date_month {
            Some(month) => format!("Issued {year}-{month:02}"),
            None => format!("Issued {year}"),
        };
        let mut remaining = active.clone();
        remaining.date_year = None;
        remaining.date_month = None;
        applied_filters.push(AppliedFilterChip { label, remove_href: build_documents_url(&remaining, sort.as_str()) });
    }
    if active.undated {
        let mut remaining = active.clone();
        remaining.undated = false;
        applied_filters.push(AppliedFilterChip { label: "Undated".to_string(), remove_href: build_documents_url(&remaining, sort.as_str()) });
    }
    for lang_value in &active.lang {
        let mut remaining = active.clone();
        remaining.lang.retain(|v| v != lang_value);
        let label = if lang_value == "unset" { "Not set".to_string() } else { crate::languages::display_name(lang_value) };
        applied_filters.push(AppliedFilterChip { label, remove_href: build_documents_url(&remaining, sort.as_str()) });
    }
    for doc_type_value in &active.doc_type {
        let mut remaining = active.clone();
        remaining.doc_type.retain(|v| v != doc_type_value);
        let label = if doc_type_value == "unset" {
            "Not set".to_string()
        } else {
            crate::doc_type_extract::label_for(doc_type_value).map(str::to_string).unwrap_or_else(|| doc_type_value.clone())
        };
        applied_filters.push(AppliedFilterChip { label, remove_href: build_documents_url(&remaining, sort.as_str()) });
    }

    let clear_filters_href = active.has_active_facets().then(|| {
        let mut cleared = active.clone();
        cleared.tags.clear();
        cleared.date_year = None;
        cleared.date_month = None;
        cleared.undated = false;
        cleared.lang.clear();
        cleared.doc_type.clear();
        build_documents_url(&cleared, sort.as_str())
    });

    let collection_rows = sqlx::query!(
        "select id, name, query, created_at from smart_collections where tenant_id = $1 order by created_at desc",
        tenancy.tenant_id.0,
    )
    .fetch_all(&state.pool)
    .await?;
    let mut collections = Vec::with_capacity(collection_rows.len());
    for row in collection_rows {
        let filters: ListQuery = serde_html_form::from_str(&row.query).unwrap_or_default();
        let count = count_matching_documents(&state, tenancy.tenant_id.0, &filters).await?;
        collections.push(CollectionOption { id: row.id, name: row.name, href: format!("/documents?{}", row.query), count });
    }

    let can_save_search = active.has_active_filters();
    let save_search_query = build_query_string(&active, sort.as_str());
    // Carries the active free-text search into a result row's link to its
    // detail page (feature 027), so `/documents/{id}` knows what to
    // highlight in the OCR text box — a document reached any other way
    // (direct link, dashboard row with no active search) gets no `?q=`
    // suffix and renders unchanged (TDR 027 §3, AC-4/AC-7).
    let detail_link_query = active.search_text().map(|text| format!("?q={}", url_encode(text))).unwrap_or_default();

    Ok(DocumentsListTemplate {
        active_tab: "documents",
        authenticated: true,
        nav_avatar_url,
        q: query.q,
        sort: sort.as_str(),
        deleted: query.deleted,
        documents,
        tag_facets,
        year_facets,
        undated_count,
        undated_checked: query.undated,
        language_facets,
        doc_type_facets,
        applied_filters,
        clear_filters_href,
        collections,
        can_save_search,
        save_search_query,
        detail_link_query,
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
    language: Option<String>,
    doc_type: Option<String>,
    ocr_suggested_doc_type: Option<String>,
    created_at: time::OffsetDateTime,
    content_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ShowQuery {
    #[serde(default)]
    saved: bool,
    #[serde(default)]
    uploaded: bool,
    #[serde(default)]
    reprocessing: bool,
    /// The free-text search that led here (feature 027) — carried along
    /// by a search-results row's link, or typed directly into this page's
    /// own URL; either way, highlighting is a pure function of this
    /// value alone (TDR 027 §3, AC-5).
    #[serde(default)]
    q: String,
}

/// `query` now carries a free-text value in `q` — same PII rule `list`'s
/// span already applies to its own `q` (feature 023 AC-9); skipped here
/// too rather than let it flow into this span by default.
#[tracing::instrument(skip(state, tenancy, query))]
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
                  ocr_suggested_date_issued, ocr_status, ocr_text, language, doc_type, ocr_suggested_doc_type,
                  created_at, content_hash
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
    // Same "only when unset" rule as the date suggestion above (TDR 024,
    // mirroring TDR 012) — `label()` turns the stored lowercase value back
    // into the human-facing text the suggestion box shows.
    let suggested_doc_type_display =
        if row.doc_type.is_none() { row.ocr_suggested_doc_type.as_deref().and_then(crate::doc_type_extract::label_for) } else { None };

    // A `ts_headline` round-trip only when there's both text to search and
    // a search to run — with no `q`, this is just `row.ocr_text` again, so
    // a plain visit still renders exactly as it did before this feature
    // (AC-7). Reuses `free_text_search`'s trim/empty-check, the same rule
    // `list`'s own `q` already follows (feature 023).
    let search_text = free_text_search(&query.q);
    let marked_ocr_text = match (search_text, row.ocr_text.as_deref()) {
        (Some(search_text), Some(ocr_text)) => Some(
            sqlx::query_scalar!(
                "select ts_headline('simple', $1, websearch_to_tsquery('simple', $2), $3)",
                ocr_text,
                search_text,
                crate::highlight::FULL_TEXT_OPTIONS,
            )
            .fetch_one(&state.pool)
            .await?
            .unwrap_or_default(),
        ),
        _ => row.ocr_text.clone(),
    };
    // Gates the "Highlighting matches for ..." indicator — only shown
    // once something in *this* document actually matched, never for a
    // `q` that happens not to appear here (AC-6).
    let has_highlight = marked_ocr_text.as_deref().is_some_and(crate::highlight::has_match);
    let highlighting_query = has_highlight.then(|| search_text.unwrap_or_default().to_string());
    let ocr_text_html = marked_ocr_text.as_deref().map(crate::highlight::render_marked);

    // A one-shot check, gated on `uploaded` — both ingestion paths
    // (desktop upload, phone scan) redirect here with `?uploaded=true`
    // right after creating this document, and never again afterward, so
    // this doubles as the "only warn once" rule with no extra state
    // (feature 029, TDR 029 §3 Alternative E).
    let duplicate_of = match (query.uploaded, &row.content_hash) {
        (true, Some(hash)) => sqlx::query!(
            "select id, title, original_filename, created_at
             from documents
             where tenant_id = $1 and content_hash = $2 and id != $3
             order by created_at asc
             limit 1",
            tenancy.tenant_id.0,
            hash,
            row.id,
        )
        .fetch_optional(&state.pool)
        .await?
        .map(|match_row| DuplicateMatch {
            id: match_row.id,
            title: match_row.title.unwrap_or(match_row.original_filename),
            uploaded_at: format_date(match_row.created_at.date()),
        }),
        _ => None,
    };

    Ok(DocumentShowTemplate {
        active_tab: "documents",
        authenticated: true,
        nav_avatar_url,
        saved: query.saved,
        uploaded: query.uploaded,
        reprocessing: query.reprocessing,
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
        ocr_text_html,
        highlighting_query,
        language: row.language.unwrap_or_default(),
        supported_language_options: crate::languages::supported_options(),
        other_language_options: crate::languages::other_options(),
        doc_type: row.doc_type.unwrap_or_default(),
        doc_type_options: crate::doc_type_extract::dropdown_options(),
        suggested_doc_type_display,
        duplicate_of,
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
    #[serde(default)]
    pub language: Language,
    #[serde(default)]
    pub doc_type: DocTypeField,
}

#[tracing::instrument(skip(state, tenancy, form))]
pub async fn update(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(form): Form<DocumentMetadataForm>,
) -> Result<Response, AppWebError> {
    let result = sqlx::query!(
        "update documents set title = $3, tags = $4, date_issued = $5, language = $6, doc_type = $7, updated_at = now()
         where id = $1 and tenant_id = $2",
        id,
        tenancy.tenant_id.0,
        form.title.into_option(),
        &form.tags.into_vec(),
        form.date_issued.into_option(),
        form.language.into_option(),
        form.doc_type.into_option(),
    )
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppWebError::NotFound);
    }

    Ok(Redirect::to(&format!("/documents/{id}?saved=true")).into_response())
}

/// Tenant-scoped existence probe, shared by every handler below whose
/// guarded `UPDATE`'s `0 rows affected` is ambiguous between "document
/// doesn't exist" (404) and "exists, but this handler's own extra guard
/// didn't match" (a no-op, not an error) — the existence check only runs
/// as a fallback on that path to tell the two apart, never on the common
/// successful-update case.
async fn document_exists(state: &AppState, id: Uuid, tenant_id: Uuid) -> Result<bool, AppWebError> {
    let exists = sqlx::query_scalar!("select exists(select 1 from documents where id = $1 and tenant_id = $2)", id, tenant_id)
        .fetch_one(&state.pool)
        .await?
        .unwrap_or(false);
    Ok(exists)
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

    if result.rows_affected() == 0 && !document_exists(&state, id, tenancy.tenant_id.0).await? {
        return Err(AppWebError::NotFound);
    }

    Ok(Redirect::to(&format!("/documents/{id}?saved=true")).into_response())
}

/// Copies `ocr_suggested_doc_type` into `doc_type` — the "Use this" action
/// from the suggestion box `show` renders (TDR 024, same idiom as
/// `accept_suggested_date` above, including why the existence check only
/// runs as a fallback on the 0-rows-affected path).
#[tracing::instrument(skip(state, tenancy))]
pub async fn accept_suggested_doc_type(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppWebError> {
    let result = sqlx::query!(
        "update documents set doc_type = ocr_suggested_doc_type, updated_at = now()
         where id = $1 and tenant_id = $2 and doc_type is null",
        id,
        tenancy.tenant_id.0,
    )
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 && !document_exists(&state, id, tenancy.tenant_id.0).await? {
        return Err(AppWebError::NotFound);
    }

    Ok(Redirect::to(&format!("/documents/{id}?saved=true")).into_response())
}

struct ReprocessRow {
    blob_key: String,
    content_type: String,
}

/// Re-runs the current OCR pipeline against a document's already-stored
/// file — the "redo the OCR" action (see TDR 013), covering both a
/// document that predates a pipeline improvement (e.g. a `skipped` PDF
/// from before feature 010) and a plain retry after `ocr_status =
/// 'failed'`. The guarded `update ... where ocr_status not in ('pending',
/// 'processing') returning ...` both makes the "don't queue a second job
/// on an in-flight document" guarantee (AC-4) and the state transition
/// atomic in a single round-trip — same idiom `accept_suggested_date`
/// above uses for its own guard. No row back means either the document
/// doesn't exist for this tenant (404) or it's already `pending`/
/// `processing` (no-op redirect); the existence check only runs as a
/// fallback to tell those two apart, same as `accept_suggested_date`.
#[tracing::instrument(skip(state, tenancy))]
pub async fn reprocess_ocr(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppWebError> {
    let row = sqlx::query_as!(
        ReprocessRow,
        // ELIGIBILITY GUARD: keep this predicate byte-for-byte identical
        // to `bulk_reprocess_ocr`'s below (sqlx's compile-time `query_as!`
        // needs a literal string, so this can't be a shared `const`/fn —
        // both are exercised by `tests/documents_bulk_actions.rs`'s
        // eligibility test and `tests/documents_reprocess_ocr.rs`'s, so a
        // drift here fails a test, not just a review).
        "update documents set ocr_status = 'pending', updated_at = now()
         where id = $1 and tenant_id = $2 and ocr_status not in ('pending', 'processing')
         returning blob_key, content_type",
        id,
        tenancy.tenant_id.0,
    )
    .fetch_optional(&state.pool)
    .await?;

    let Some(row) = row else {
        if !document_exists(&state, id, tenancy.tenant_id.0).await? {
            return Err(AppWebError::NotFound);
        }

        return Ok(Redirect::to(&format!("/documents/{id}")).into_response());
    };

    let ocr_state = state.clone();
    tokio::spawn(
        run_ocr(ocr_state, id, tenancy.tenant_id.0, row.blob_key, row.content_type).instrument(tracing::Span::current()),
    );

    Ok(Redirect::to(&format!("/documents/{id}?reprocessing=true")).into_response())
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

/// Best-effort removal of a document's blob and its generated thumbnail
/// (feature 025) — shared by single-document `delete` and bulk
/// `bulk_delete` so the two paths can't drift on which blobs get cleaned
/// up (they briefly had: `bulk_delete` cleaned up the thumbnail,
/// single-document `delete` didn't, leaking an orphaned thumbnail object
/// per single delete). Failures are logged, not bubbled: by the time this
/// runs, the DB delete has already committed, so from the user's side the
/// document is already gone — surfacing an error here would report
/// failure for an action that, as far as they can tell, already
/// succeeded.
async fn delete_document_blobs(blob: &crate::blob::BlobStore, blob_key: &str) {
    if let Err(error) = blob.delete_object(blob_key).await {
        tracing::warn!(%error, "failed to delete blob for an already-deleted document row");
    }
    if let Err(error) = blob.delete_object(&thumbnail_blob_key(blob_key)).await {
        tracing::warn!(%error, "failed to delete thumbnail blob for an already-deleted document row");
    }
}

/// Deletes the DB row first (tenant-scoped `DELETE ... RETURNING` both
/// checks ownership and hands back the `blob_key` in one query), then the
/// blob — so a blob-delete failure never leaves a row pointing at storage
/// that's already gone, only the reverse (an orphaned blob with no row,
/// which is invisible to the user and harmless beyond wasted storage).
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

    delete_document_blobs(&state.blob, &row.blob_key).await;

    Ok(Redirect::to("/documents?deleted=true").into_response())
}

/// Shared by every bulk handler below (feature 026): builds a redirect
/// target from the dashboard's `return_to` hidden field (the current
/// filtered/sorted query string, or empty for "no filters active") plus
/// an optional flash-flag query param — so a bulk action lands the user
/// back where they were, not reset to the unfiltered dashboard.
fn bulk_redirect_target(return_to: &str, extra_flag: Option<&str>) -> String {
    let mut target = if return_to.is_empty() { "/documents".to_string() } else { format!("/documents?{return_to}") };
    if let Some(flag) = extra_flag {
        target.push(if target.contains('?') { '&' } else { '?' });
        target.push_str(flag);
    }
    target
}

#[derive(Debug, Deserialize)]
pub struct BulkActionForm {
    #[serde(default)]
    pub doc_ids: Vec<Uuid>,
    #[serde(default)]
    pub return_to: String,
}

struct BulkDeleteRow {
    id: Uuid,
    title: Option<String>,
    original_filename: String,
}

/// Renders the "delete N documents?" confirm page (feature 026) — same
/// confirm-before-destroy precedent single-document `delete` already
/// established (unlike `delete_collection`'s no-confirm exception, a
/// document isn't trivially re-creatable).
#[tracing::instrument(skip(state, tenancy, form))]
pub async fn bulk_delete_confirm(
    tenancy: TenantContext,
    State(state): State<AppState>,
    MultiForm(form): MultiForm<BulkActionForm>,
) -> Result<DocumentBulkDeleteConfirmTemplate, AppWebError> {
    let nav_avatar_url = nav::avatar_url(&state.pool, &state.blob, tenancy.user_id.0).await?;

    let rows = sqlx::query_as!(
        BulkDeleteRow,
        "select id, title, original_filename from documents where id = any($1) and tenant_id = $2",
        &form.doc_ids,
        tenancy.tenant_id.0,
    )
    .fetch_all(&state.pool)
    .await?;

    let documents = rows
        .into_iter()
        .map(|row| BulkDeleteDocumentSummary {
            id: row.id,
            title: row.title.unwrap_or_else(|| row.original_filename.clone()),
            original_filename: row.original_filename,
        })
        .collect();

    Ok(DocumentBulkDeleteConfirmTemplate { active_tab: "documents", authenticated: true, nav_avatar_url, documents, return_to: form.return_to })
}

/// Executes the bulk delete confirmed by `bulk_delete_confirm` above —
/// same `DELETE ... RETURNING blob_key` shape as single-document `delete`,
/// just over every id in one round trip, then `delete_document_blobs` per
/// row concurrently (independent per-document blob/thumbnail deletes,
/// no reason to await them one at a time). Ids belonging to another
/// tenant simply don't match the `tenant_id` guard and are silently
/// excluded, the same isolation every other mutating query in this file
/// already relies on.
#[tracing::instrument(skip(state, tenancy, form))]
pub async fn bulk_delete(
    tenancy: TenantContext,
    State(state): State<AppState>,
    MultiForm(form): MultiForm<BulkActionForm>,
) -> Result<Response, AppWebError> {
    let rows = sqlx::query!(
        "delete from documents where id = any($1) and tenant_id = $2 returning blob_key",
        &form.doc_ids,
        tenancy.tenant_id.0,
    )
    .fetch_all(&state.pool)
    .await?;

    futures_util::future::join_all(rows.iter().map(|row| delete_document_blobs(&state.blob, &row.blob_key))).await;

    Ok(Redirect::to(&bulk_redirect_target(&form.return_to, Some("deleted=true"))).into_response())
}

#[derive(Debug, Deserialize)]
pub struct BulkTagForm {
    #[serde(default)]
    pub doc_ids: Vec<Uuid>,
    #[serde(default)]
    pub tag: Tags,
    #[serde(default)]
    pub return_to: String,
}

/// Adds every tag parsed from the bulk toolbar's tag input to every
/// selected document, deduplicating against tags a document already has
/// — no confirmation needed (additive, reversible), unlike bulk delete.
/// Appends only the genuinely-new tags after a document's existing array
/// rather than `select distinct unnest`-ing the whole union: `DISTINCT`
/// gives no order guarantee, so that would silently reorder tags a
/// document already had (visible in chip order and the "Tags (A-Z)"
/// sort, which joins the array in place) even though no tag's content
/// actually changed.
#[tracing::instrument(skip(state, tenancy, form))]
pub async fn bulk_tag(
    tenancy: TenantContext,
    State(state): State<AppState>,
    MultiForm(form): MultiForm<BulkTagForm>,
) -> Result<Response, AppWebError> {
    let new_tags = form.tag.into_vec();
    if !new_tags.is_empty() {
        sqlx::query!(
            "update documents set tags = tags || coalesce(
                 (select array_agg(distinct t) from unnest($1::text[]) as t where t <> all(tags)),
                 '{}'::text[]
             ), updated_at = now()
             where id = any($2) and tenant_id = $3",
            &new_tags,
            &form.doc_ids,
            tenancy.tenant_id.0,
        )
        .execute(&state.pool)
        .await?;
    }

    Ok(Redirect::to(&bulk_redirect_target(&form.return_to, None)).into_response())
}

struct BulkReprocessRow {
    id: Uuid,
    blob_key: String,
    content_type: String,
}

/// Bulk "reprocess all eligible OCR" (the deferred action named in
/// ARCHITECTURE.md §8, closing that gap) — reuses `reprocess_ocr`'s exact
/// eligibility guard (`ocr_status not in ('pending', 'processing')`) over
/// every selected id in one round trip, then spawns the same `run_ocr`
/// task per eligible row. `state.ocr_semaphore` (already acquired inside
/// `run_ocr`) naturally bounds how many run concurrently regardless of
/// how many are spawned at once — no new batching/throttling logic
/// needed.
#[tracing::instrument(skip(state, tenancy, form))]
pub async fn bulk_reprocess_ocr(
    tenancy: TenantContext,
    State(state): State<AppState>,
    MultiForm(form): MultiForm<BulkActionForm>,
) -> Result<Response, AppWebError> {
    let rows = sqlx::query_as!(
        BulkReprocessRow,
        // ELIGIBILITY GUARD: keep this predicate byte-for-byte identical
        // to `reprocess_ocr`'s above — see the comment there for why this
        // can't be a shared compile-time constant.
        "update documents set ocr_status = 'pending', updated_at = now()
         where id = any($1) and tenant_id = $2 and ocr_status not in ('pending', 'processing')
         returning id, blob_key, content_type",
        &form.doc_ids,
        tenancy.tenant_id.0,
    )
    .fetch_all(&state.pool)
    .await?;

    for row in rows {
        let ocr_state = state.clone();
        tokio::spawn(run_ocr(ocr_state, row.id, tenancy.tenant_id.0, row.blob_key, row.content_type).instrument(tracing::Span::current()));
    }

    Ok(Redirect::to(&bulk_redirect_target(&form.return_to, None)).into_response())
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

/// Generates and uploads a small preview JPEG for the dashboard (feature
/// 025), reusing whichever raster bytes `run_ocr` already produced — the
/// original image bytes for a direct upload, or the PDF's rasterized page
/// 1 (`ocr::extract`'s second return value) — rather than a second blob
/// fetch or a second PDF rasterization pass. Best-effort: a thumbnail
/// failure never fails the OCR pass itself (returns `"failed"`, skip
/// tracked in `documents.thumbnail_status`); the dashboard just falls back
/// to its pre-025 rendering for that document.
#[tracing::instrument(skip(state, thumbnail_source))]
async fn generate_and_store_thumbnail(state: &AppState, blob_key: &str, thumbnail_source: &[u8]) -> &'static str {
    let jpeg_bytes = match crate::thumbnail::generate(thumbnail_source) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::error!(%error, "thumbnail generation failed");
            return "failed";
        }
    };

    match state.blob.upload_bytes(&thumbnail_blob_key(blob_key), "image/jpeg", jpeg_bytes).await {
        Ok(_) => "done",
        Err(error) => {
            tracing::error!(%error, "thumbnail upload failed");
            "failed"
        }
    }
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

    // Kept alongside `text_result` (not consumed inside the match that
    // produces it) so a successful OCR pass can also fall back to an
    // EXIF-sourced date suggestion (feature 019), and thumbnailing can
    // fall back to the original upload's bytes (feature 025), without a
    // second blob fetch.
    let blob_bytes = state.blob.get_object(&blob_key).await;
    let (text_result, thumbnail_source): (Result<String, String>, Option<Vec<u8>>) = match &blob_bytes {
        Ok(bytes) => {
            let (text_result, thumbnail_source) = crate::ocr::extract(&content_type, bytes).await;
            (text_result.map_err(|e| e.to_string()), thumbnail_source)
        }
        Err(error) => (Err(error.to_string()), None),
    };

    // Thumbnail generation runs regardless of whether OCR text extraction
    // succeeded — a thumbnail only needs pixels, not legible text, so a
    // failed OCR pass must not also mean a permanently missing thumbnail
    // for a document whose image bytes are perfectly valid. Prefers
    // `thumbnail_source` (a PDF's rasterized page 1, itself independent of
    // whether `tesseract` succeeded on it or a later page — see
    // `ocr::extract_text_from_pdf`); falls back to the original upload's
    // bytes for a direct image, which are thumbnail-source and OCR-source
    // in one. `None` only when the blob fetch itself failed — nothing to
    // thumbnail at all.
    let thumbnail_bytes = thumbnail_source.as_deref().or_else(|| blob_bytes.as_deref().ok());
    let thumbnail_status = match thumbnail_bytes {
        Some(bytes) => generate_and_store_thumbnail(&state, &blob_key, bytes).await,
        None => "failed",
    };

    // Backfills `content_hash` for a document uploaded before feature 029
    // (or reprocessed for any other reason) — `None` only when the blob
    // fetch itself failed, in which case `coalesce` below preserves
    // whatever hash, if any, was already recorded rather than clobbering
    // it with null over a transient fetch error.
    let content_hash = blob_bytes.as_ref().ok().map(|bytes| crate::content_hash::hash_bytes(bytes));

    let update_result = match text_result {
        Ok(text) => {
            let ocr_suggested_date_issued = crate::date_extract::extract_issued_date(&text);
            let exif_suggested_date_issued =
                blob_bytes.as_ref().ok().and_then(|bytes| crate::exif_extract::extract_issued_date(bytes));
            let suggested_date_issued = ocr_suggested_date_issued.or(exif_suggested_date_issued);
            let language = crate::language_detect::detect(&text);
            let suggested_doc_type = crate::doc_type_extract::extract_doc_type(&text).map(|dt| dt.as_str());
            sqlx::query!(
                "update documents set ocr_status = 'done', ocr_text = $3, ocr_suggested_date_issued = $4,
                        language = coalesce(language, $5), ocr_suggested_doc_type = $6, thumbnail_status = $7,
                        content_hash = coalesce($8, content_hash)
                 where id = $1 and tenant_id = $2",
                document_id,
                tenant_id,
                text,
                suggested_date_issued,
                language,
                suggested_doc_type,
                thumbnail_status,
                content_hash,
            )
            .execute(&state.pool)
            .await
        }
        Err(error_message) => {
            tracing::error!(error = %error_message, "ocr extraction failed");
            sqlx::query!(
                "update documents set ocr_status = 'failed', ocr_error = $3, thumbnail_status = $4,
                        content_hash = coalesce($5, content_hash)
                 where id = $1 and tenant_id = $2",
                document_id,
                tenant_id,
                error_message,
                thumbnail_status,
                content_hash,
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
/// and returns the resulting byte count plus a hex-encoded SHA-256 of its
/// content (feature 029's duplicate detection — computed in the same pass
/// `BlobStore::stream_upload_with_hash` already reads chunks in for its
/// byte-count check, no second read). Split out from
/// `insert_document_and_queue_ocr` below specifically so `create`'s
/// "metadata fields must arrive before the file field" check can still
/// reject a request (with no document row ever created) even *after* this
/// stream has already run — the file field is necessarily read as it's
/// encountered (multipart bodies can't be rewound to check what follows
/// first), so only the upload is unavoidable in that rejected case, never
/// the DB row; the S3 object it leaves behind is an accepted, harmless
/// orphan (nothing ever references it).
pub(crate) async fn stream_document_to_blob(
    state: &AppState,
    user_id: Uuid,
    document_id: Uuid,
    content_type: &str,
    field: axum::extract::multipart::Field<'_>,
) -> Result<(String, i64, String), AppWebError> {
    let blob_key = format!("documents/{user_id}/{document_id}");
    let (file_size_bytes, content_hash) =
        state.blob.stream_upload_with_hash(&blob_key, content_type, field, MAX_DOCUMENT_BYTES).await?;
    Ok((blob_key, file_size_bytes as i64, content_hash))
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
    content_hash: String,
) -> Result<(), AppWebError> {
    let is_ocr_eligible = ocr_eligible(&content_type);
    let initial_ocr_status = if is_ocr_eligible { "pending" } else { "skipped" };

    sqlx::query!(
        "insert into documents
            (id, tenant_id, user_id, original_filename, title, content_type, file_size_bytes, blob_key, tags, date_issued, ocr_status, content_hash)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
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
        content_hash,
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
    // (blob_key, original_filename, content_type, file_size_bytes,
    // content_hash) — the DB insert is deferred until after the whole
    // multipart body has validated (see `stream_document_to_blob`'s doc
    // comment), so this just remembers what to insert once we know no
    // later field will reject the request.
    let mut uploaded: Option<(String, String, String, i64, String)> = None;
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
                let (blob_key, file_size_bytes, content_hash) =
                    stream_document_to_blob(&state, tenancy.user_id.0, document_id, &content_type, field).await?;
                uploaded = Some((blob_key, original_filename, content_type, file_size_bytes, content_hash));
            }
            _ => {}
        }
    }

    let Some((blob_key, original_filename, content_type, file_size_bytes, content_hash)) = uploaded else {
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
        content_hash,
    )
    .await?;

    Ok(Redirect::to(&format!("/documents/{document_id}?uploaded=true")).into_response())
}

#[derive(Debug, Deserialize)]
pub struct SaveCollectionForm {
    pub name: CollectionName,
    #[serde(default)]
    pub query: String,
}

/// Saves the current `/documents` filter state as a named collection
/// (TDR 016). `form.query` is the exact query string `list` already
/// builds for its own applied-filter chips (see `build_query_string`),
/// submitted as a hidden field — this handler never re-derives filter
/// state itself, only validates that the submitted state is a *real*
/// filter (AC-4): a crafted request with an empty/no-op `query` is
/// rejected server-side, not just hidden client-side by the "Save this
/// search" control's own visibility rule.
#[tracing::instrument(skip(state, tenancy, form))]
pub async fn save_collection(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Form(form): Form<SaveCollectionForm>,
) -> Result<Response, AppWebError> {
    let filters: ListQuery = serde_html_form::from_str(&form.query).unwrap_or_default();
    if !active_filters(&filters).has_active_filters() {
        return Ok(bad_request("cannot save a search with no active filter"));
    }

    let id = Uuid::new_v4();
    sqlx::query!(
        "insert into smart_collections (id, tenant_id, name, query) values ($1, $2, $3, $4)",
        id,
        tenancy.tenant_id.0,
        form.name.as_str(),
        form.query,
    )
    .execute(&state.pool)
    .await?;

    Ok(Redirect::to("/documents").into_response())
}

/// Tenant-scoped delete, no confirmation step — unlike document deletion,
/// removing a saved collection touches nothing but a bookmark (TDR 016
/// §2 Alternative C).
#[tracing::instrument(skip(state, tenancy))]
pub async fn delete_collection(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppWebError> {
    sqlx::query!(
        "delete from smart_collections where id = $1 and tenant_id = $2 returning id",
        id,
        tenancy.tenant_id.0,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppWebError::NotFound)?;

    Ok(Redirect::to("/documents").into_response())
}

#[derive(Debug, Deserialize)]
pub struct RenameCollectionForm {
    pub name: CollectionName,
}

/// Updates only `name` — `query`/`created_at` are untouched, so a rename
/// never changes what a collection filters or its position in the
/// newest-first list (TDR 017 AC-2).
#[tracing::instrument(skip(state, tenancy, form))]
pub async fn rename_collection(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(form): Form<RenameCollectionForm>,
) -> Result<Response, AppWebError> {
    sqlx::query!(
        "update smart_collections set name = $3 where id = $1 and tenant_id = $2 returning id",
        id,
        tenancy.tenant_id.0,
        form.name.as_str(),
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppWebError::NotFound)?;

    Ok(Redirect::to("/documents").into_response())
}
