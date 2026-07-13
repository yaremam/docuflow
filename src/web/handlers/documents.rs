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
use serde::Deserialize;
use tracing::Instrument;
use uuid::Uuid;

use crate::web::error::AppWebError;
use crate::web::forms::{CollectionName, DateIssuedField, Language, ProfileField, Tags};
use crate::web::nav;
use crate::web::state::AppState;
use crate::web::tenancy::TenantContext;
use crate::web::templates::{
    AppliedFilterChip, CollectionOption, DocumentDeleteTemplate, DocumentListItem, DocumentNewTemplate,
    DocumentShowTemplate, DocumentsListTemplate, LanguageFacetOption, MonthFacetOption, TagFacetOption, YearFacetOption,
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
/// other filter link this page produces.
#[allow(clippy::too_many_arguments)]
fn build_query_string(
    q: &str,
    sort: &str,
    tags: &[String],
    date_year: Option<i32>,
    date_month: Option<i32>,
    undated: bool,
    lang: &[String],
) -> String {
    let mut params: Vec<(&str, String)> = Vec::new();
    if !q.is_empty() {
        params.push(("q", q.to_string()));
    }
    params.push(("sort", sort.to_string()));
    for tag in tags {
        params.push(("tags", tag.clone()));
    }
    if let Some(year) = date_year {
        params.push(("date_year", year.to_string()));
    }
    if let Some(month) = date_month {
        params.push(("date_month", month.to_string()));
    }
    if undated {
        params.push(("undated", "true".to_string()));
    }
    for value in lang {
        params.push(("lang", value.clone()));
    }

    params.iter().map(|(key, value)| format!("{key}={}", url_encode(value))).collect::<Vec<_>>().join("&")
}

/// Builds a full `/documents?...` URL from a full set of filter state —
/// shared by the applied-filter chips' individual "remove this one" links
/// and "Clear all", so both stay in sync with the query params `list`
/// itself parses (see TDR 015 §3).
#[allow(clippy::too_many_arguments)]
fn build_documents_url(
    q: &str,
    sort: &str,
    tags: &[String],
    date_year: Option<i32>,
    date_month: Option<i32>,
    undated: bool,
    lang: &[String],
) -> String {
    let query_string = build_query_string(q, sort, tags, date_year, date_month, undated, lang);
    if query_string.is_empty() {
        "/documents".to_string()
    } else {
        format!("/documents?{query_string}")
    }
}

/// Whether a parsed `ListQuery` has any real filter active — a facet or
/// non-empty free-text search. Shared by `list` (gating the "Save this
/// search" control) and `save_collection` (rejecting a no-op save server-
/// side, not just hiding the UI control — see TDR 016 AC-4).
fn query_has_active_filters(filters: &ListQuery) -> bool {
    !filters.tags.is_empty()
        || filters.date_year.is_some()
        || filters.undated
        || !filters.lang.is_empty()
        || !filters.q.trim().is_empty()
}

/// Counts documents matching an arbitrary filter set — the same
/// conditions `list`'s main query applies, minus any `ORDER BY` (a count
/// needs no ordering). Used both for a saved collection's live count
/// (feature 016) and could back a future "how many would this match"
/// preview; kept as its own query rather than folded into `list`'s
/// per-sort arms since it's needed once per collection, not once per
/// page load.
/// The raw eight-dimension `WHERE` clause shared by every "how many
/// documents match this filter combination" question on `/documents` —
/// a saved collection's full count (`count_matching_documents`) and,
/// since feature 018, each individual facet option's narrowed count
/// (`list` calls this directly, once per candidate, with that facet's
/// own dimension pinned to a single value and every other dimension left
/// at whatever's currently active — see TDR 018 §3).
#[allow(clippy::too_many_arguments)]
async fn count_documents(
    state: &AppState,
    tenant_id: Uuid,
    q_tags: Option<&[String]>,
    facet_tags: &[String],
    date_year: Option<i32>,
    date_month: Option<i32>,
    undated: bool,
    lang_values: &[String],
    lang_unset: bool,
) -> Result<i64, AppWebError> {
    let count = sqlx::query_scalar!(
        "select count(*) from documents
         where tenant_id = $1
           and ($2::text[] is null or tags && $2)
           and (cardinality($3::text[]) = 0 or tags @> $3)
           and (($4::int4 is null and $6 = false)
                or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                    and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                or ($6 and date_issued is null))
           and ((cardinality($7::text[]) = 0 and $8 = false)
                or (language = any($7))
                or ($8 and language is null))",
        tenant_id,
        q_tags,
        facet_tags,
        date_year,
        date_month,
        undated,
        lang_values,
        lang_unset,
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
    let tag_filter = parse_tag_search(&filters.q);
    let date_year = filters.date_year;
    let date_month = if date_year.is_some() { filters.date_month } else { None };
    let lang_values: Vec<String> = filters.lang.iter().filter(|value| value.as_str() != "unset").cloned().collect();
    let lang_unset = filters.lang.iter().any(|value| value == "unset");

    count_documents(state, tenant_id, tag_filter.as_deref(), &filters.tags, date_year, date_month, filters.undated, &lang_values, lang_unset).await
}

#[tracing::instrument(skip(state, tenancy, query))]
pub async fn list(
    tenancy: TenantContext,
    State(state): State<AppState>,
    MultiQuery(query): MultiQuery<ListQuery>,
) -> Result<DocumentsListTemplate, AppWebError> {
    let nav_avatar_url = nav::avatar_url(&state.pool, &state.blob, tenancy.user_id.0).await?;
    let tag_filter = parse_tag_search(&query.q);
    let sort = Sort::parse(query.sort.as_deref());

    let date_year = query.date_year;
    let date_month = if date_year.is_some() { query.date_month } else { None };
    let lang_values: Vec<String> = query.lang.iter().filter(|value| value.as_str() != "unset").cloned().collect();
    let lang_unset = query.lang.iter().any(|value| value == "unset");

    // Each arm differs only in `ORDER BY` — sqlx's compile-time `query_as!`
    // macro can't parameterize that clause, so the small, fixed set of sort
    // modes is spelled out literally rather than building the SQL string at
    // runtime (which would forgo compile-time verification for every query
    // on this page, not just the ordering). The facet conditions ($3-$8)
    // are identical across all five arms — see TDR 015 §3 for what each
    // one means.
    let rows = match sort {
        Sort::CreatedAtDesc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, created_at
                   from documents
                   where tenant_id = $1
                     and ($2::text[] is null or tags && $2)
                     and (cardinality($3::text[]) = 0 or tags @> $3)
                     and (($4::int4 is null and $6 = false)
                          or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                              and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                          or ($6 and date_issued is null))
                     and ((cardinality($7::text[]) = 0 and $8 = false)
                          or (language = any($7))
                          or ($8 and language is null))
                   order by created_at desc"#,
                tenancy.tenant_id.0,
                tag_filter.as_deref(),
                query.tags.as_slice(),
                date_year,
                date_month,
                query.undated,
                lang_values.as_slice(),
                lang_unset,
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::CreatedAtAsc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, created_at
                   from documents
                   where tenant_id = $1
                     and ($2::text[] is null or tags && $2)
                     and (cardinality($3::text[]) = 0 or tags @> $3)
                     and (($4::int4 is null and $6 = false)
                          or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                              and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                          or ($6 and date_issued is null))
                     and ((cardinality($7::text[]) = 0 and $8 = false)
                          or (language = any($7))
                          or ($8 and language is null))
                   order by created_at asc"#,
                tenancy.tenant_id.0,
                tag_filter.as_deref(),
                query.tags.as_slice(),
                date_year,
                date_month,
                query.undated,
                lang_values.as_slice(),
                lang_unset,
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::DateIssuedDesc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, created_at
                   from documents
                   where tenant_id = $1
                     and ($2::text[] is null or tags && $2)
                     and (cardinality($3::text[]) = 0 or tags @> $3)
                     and (($4::int4 is null and $6 = false)
                          or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                              and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                          or ($6 and date_issued is null))
                     and ((cardinality($7::text[]) = 0 and $8 = false)
                          or (language = any($7))
                          or ($8 and language is null))
                   order by date_issued desc nulls last"#,
                tenancy.tenant_id.0,
                tag_filter.as_deref(),
                query.tags.as_slice(),
                date_year,
                date_month,
                query.undated,
                lang_values.as_slice(),
                lang_unset,
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::DateIssuedAsc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, created_at
                   from documents
                   where tenant_id = $1
                     and ($2::text[] is null or tags && $2)
                     and (cardinality($3::text[]) = 0 or tags @> $3)
                     and (($4::int4 is null and $6 = false)
                          or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                              and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                          or ($6 and date_issued is null))
                     and ((cardinality($7::text[]) = 0 and $8 = false)
                          or (language = any($7))
                          or ($8 and language is null))
                   order by date_issued asc nulls last"#,
                tenancy.tenant_id.0,
                tag_filter.as_deref(),
                query.tags.as_slice(),
                date_year,
                date_month,
                query.undated,
                lang_values.as_slice(),
                lang_unset,
            )
            .fetch_all(&state.pool)
            .await?
        }
        Sort::TagsAsc => {
            sqlx::query_as!(
                DocumentListRow,
                r#"select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, created_at
                   from documents
                   where tenant_id = $1
                     and ($2::text[] is null or tags && $2)
                     and (cardinality($3::text[]) = 0 or tags @> $3)
                     and (($4::int4 is null and $6 = false)
                          or ($4::int4 is not null and extract(year from date_issued)::int4 = $4
                              and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
                          or ($6 and date_issued is null))
                     and ((cardinality($7::text[]) = 0 and $8 = false)
                          or (language = any($7))
                          or ($8 and language is null))
                   order by array_to_string(tags, ',') asc"#,
                tenancy.tenant_id.0,
                tag_filter.as_deref(),
                query.tags.as_slice(),
                date_year,
                date_month,
                query.undated,
                lang_values.as_slice(),
                lang_unset,
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
    let mut tag_facets = Vec::with_capacity(tag_rows.len());
    for row in tag_rows {
        let name = row.tag.unwrap_or_default();
        let checked = query.tags.iter().any(|tag| tag == &name);
        let count = count_documents(
            &state,
            tenancy.tenant_id.0,
            tag_filter.as_deref(),
            std::slice::from_ref(&name),
            date_year,
            date_month,
            query.undated,
            &lang_values,
            lang_unset,
        )
        .await?;
        tag_facets.push(TagFacetOption { name, count, checked });
    }

    let year_rows = sqlx::query!(
        "select extract(year from date_issued)::int4 as year, count(*) as count
         from documents where tenant_id = $1 and date_issued is not null
         group by year order by year desc",
        tenancy.tenant_id.0,
    )
    .fetch_all(&state.pool)
    .await?;
    let month_rows = if let Some(year) = date_year {
        sqlx::query!(
            "select extract(month from date_issued)::int4 as month, count(*) as count
             from documents where tenant_id = $1 and extract(year from date_issued)::int4 = $2
             group by month order by month",
            tenancy.tenant_id.0,
            year,
        )
        .fetch_all(&state.pool)
        .await?
    } else {
        Vec::new()
    };
    let mut year_facets = Vec::with_capacity(year_rows.len());
    for row in year_rows {
        let year = row.year.unwrap_or(0);
        let checked = date_year == Some(year);
        let count =
            count_documents(&state, tenancy.tenant_id.0, tag_filter.as_deref(), &query.tags, Some(year), None, false, &lang_values, lang_unset).await?;
        let months = if checked {
            let mut month_facets = Vec::with_capacity(month_rows.len());
            for month_row in &month_rows {
                let month = month_row.month.unwrap_or(0);
                let month_count =
                    count_documents(&state, tenancy.tenant_id.0, tag_filter.as_deref(), &query.tags, Some(year), Some(month), false, &lang_values, lang_unset)
                        .await?;
                month_facets.push(MonthFacetOption {
                    label: month_name(month),
                    value: month.clamp(0, 12) as u8,
                    count: month_count,
                    checked: date_month == Some(month),
                });
            }
            month_facets
        } else {
            Vec::new()
        };
        year_facets.push(YearFacetOption { year, count, checked, months });
    }

    let undated_count =
        count_documents(&state, tenancy.tenant_id.0, tag_filter.as_deref(), &query.tags, None, None, true, &lang_values, lang_unset).await?;

    let language_rows = sqlx::query!(
        "select distinct language from documents where tenant_id = $1 and language is not null order by language",
        tenancy.tenant_id.0,
    )
    .fetch_all(&state.pool)
    .await?;
    let mut language_facets = Vec::with_capacity(language_rows.len() + 1);
    for row in language_rows {
        let Some(code) = row.language else { continue };
        let count = count_documents(
            &state,
            tenancy.tenant_id.0,
            tag_filter.as_deref(),
            &query.tags,
            date_year,
            date_month,
            query.undated,
            std::slice::from_ref(&code),
            false,
        )
        .await?;
        let checked = lang_values.iter().any(|v| v == &code);
        let label = crate::languages::display_name(&code);
        language_facets.push(LanguageFacetOption { value: code, label, count, checked });
    }
    let unset_count = count_documents(&state, tenancy.tenant_id.0, tag_filter.as_deref(), &query.tags, date_year, date_month, query.undated, &[], true).await?;
    language_facets.push(LanguageFacetOption { value: "unset".to_string(), label: "Not set".to_string(), count: unset_count, checked: lang_unset });

    let mut applied_filters: Vec<AppliedFilterChip> = Vec::new();
    for tag in &query.tags {
        let remaining: Vec<String> = query.tags.iter().filter(|t| *t != tag).cloned().collect();
        applied_filters.push(AppliedFilterChip {
            label: tag.clone(),
            remove_href: build_documents_url(&query.q, sort.as_str(), &remaining, date_year, date_month, query.undated, &query.lang),
        });
    }
    if let Some(year) = date_year {
        let label = match date_month {
            Some(month) => format!("Issued {year}-{month:02}"),
            None => format!("Issued {year}"),
        };
        applied_filters.push(AppliedFilterChip {
            label,
            remove_href: build_documents_url(&query.q, sort.as_str(), &query.tags, None, None, query.undated, &query.lang),
        });
    }
    if query.undated {
        applied_filters.push(AppliedFilterChip {
            label: "Undated".to_string(),
            remove_href: build_documents_url(&query.q, sort.as_str(), &query.tags, date_year, date_month, false, &query.lang),
        });
    }
    for lang_value in &query.lang {
        let remaining: Vec<String> = query.lang.iter().filter(|v| *v != lang_value).cloned().collect();
        let label = if lang_value == "unset" { "Not set".to_string() } else { crate::languages::display_name(lang_value) };
        applied_filters.push(AppliedFilterChip {
            label,
            remove_href: build_documents_url(&query.q, sort.as_str(), &query.tags, date_year, date_month, query.undated, &remaining),
        });
    }

    let filters_active = !query.tags.is_empty() || date_year.is_some() || query.undated || !query.lang.is_empty();
    let clear_filters_href = filters_active.then(|| build_documents_url(&query.q, sort.as_str(), &[], None, None, false, &[]));

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

    let can_save_search = query_has_active_filters(&query);
    let save_search_query = build_query_string(&query.q, sort.as_str(), &query.tags, date_year, date_month, query.undated, &query.lang);

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
        applied_filters,
        clear_filters_href,
        collections,
        can_save_search,
        save_search_query,
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
    created_at: time::OffsetDateTime,
}

#[derive(Debug, Deserialize)]
pub struct ShowQuery {
    #[serde(default)]
    saved: bool,
    #[serde(default)]
    uploaded: bool,
    #[serde(default)]
    reprocessing: bool,
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
                  ocr_suggested_date_issued, ocr_status, ocr_text, language, created_at
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
        ocr_text: row.ocr_text,
        language: row.language.unwrap_or_default(),
        supported_language_options: crate::languages::supported_options(),
        other_language_options: crate::languages::other_options(),
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
}

#[tracing::instrument(skip(state, tenancy, form))]
pub async fn update(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(form): Form<DocumentMetadataForm>,
) -> Result<Response, AppWebError> {
    let result = sqlx::query!(
        "update documents set title = $3, tags = $4, date_issued = $5, language = $6, updated_at = now()
         where id = $1 and tenant_id = $2",
        id,
        tenancy.tenant_id.0,
        form.title.into_option(),
        &form.tags.into_vec(),
        form.date_issued.into_option(),
        form.language.into_option(),
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
        "update documents set ocr_status = 'pending', updated_at = now()
         where id = $1 and tenant_id = $2 and ocr_status not in ('pending', 'processing')
         returning blob_key, content_type",
        id,
        tenancy.tenant_id.0,
    )
    .fetch_optional(&state.pool)
    .await?;

    let Some(row) = row else {
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

    // Kept alongside `outcome` (not consumed inside the match that produces
    // it) so a successful OCR pass can also fall back to an EXIF-sourced
    // date suggestion (feature 019) without a second blob fetch.
    let blob_bytes = state.blob.get_object(&blob_key).await;
    let outcome = match &blob_bytes {
        Ok(bytes) => crate::ocr::extract(&content_type, bytes).await.map_err(|e| e.to_string()),
        Err(error) => Err(error.to_string()),
    };

    let update_result = match outcome {
        Ok(text) => {
            let ocr_suggested_date_issued = crate::date_extract::extract_issued_date(&text);
            let exif_suggested_date_issued =
                blob_bytes.as_ref().ok().and_then(|bytes| crate::exif_extract::extract_issued_date(bytes));
            let suggested_date_issued = ocr_suggested_date_issued.or(exif_suggested_date_issued);
            let language = crate::language_detect::detect(&text);
            sqlx::query!(
                "update documents set ocr_status = 'done', ocr_text = $3, ocr_suggested_date_issued = $4,
                        language = coalesce(language, $5)
                 where id = $1 and tenant_id = $2",
                document_id,
                tenant_id,
                text,
                suggested_date_issued,
                language,
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
    if !query_has_active_filters(&filters) {
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
