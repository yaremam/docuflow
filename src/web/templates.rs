#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "landing.html")]
pub struct LandingTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "signup.html")]
pub struct SignupTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub error: Option<&'static str>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub error: Option<&'static str>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "welcome.html")]
pub struct WelcomeTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub returning: bool,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "profile.html")]
pub struct ProfileTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub saved: bool,
    pub first_name: String,
    pub last_name: String,
    pub street_address: String,
    pub city: String,
    pub postcode: String,
    pub country: String,
    pub phone: String,
    pub picture_url: Option<String>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "forgot_password.html")]
pub struct ForgotPasswordTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub sent: bool,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "reset_password.html")]
pub struct ResetPasswordTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub valid: bool,
    pub token: String,
}

/// A single row in the documents list — a display-ready projection of a
/// `documents` row, not the row itself (dates/tags are pre-formatted in the
/// handler since no date-formatting Askama filter is set up in this project).
pub struct DocumentListItem {
    pub id: uuid::Uuid,
    pub title: String,
    pub original_filename: String,
    pub file_url: String,
    pub is_image: bool,
    /// A generated, page-1/preview-sized thumbnail (feature 025) — `None`
    /// until `run_ocr` produces one (or if generation failed), in which
    /// case the template falls back to the pre-025 `is_image`/`file_url`
    /// rendering unchanged.
    pub thumbnail_url: Option<String>,
    pub tags: Vec<String>,
    pub date_issued: Option<String>,
    pub uploaded_at: String,
    pub ocr_status: String,
    pub doc_type_label: Option<&'static str>,
    /// Pre-rendered safe HTML (feature 027) — `Some` only when this row's
    /// OCR text, not merely its tags, matched the active free-text
    /// search; already escaped and `<mark>`-wrapped by
    /// `highlight::render_marked`, rendered with `|safe`.
    pub ocr_snippet_html: Option<String>,
}

/// One checkbox in the Tags facet group (see TDR 015 §3) — `count` is
/// against the tenant's full document set, not narrowed by any other
/// currently-active facet (an accepted v1 simplification, AC-10).
pub struct TagFacetOption {
    pub name: String,
    pub count: i64,
    pub checked: bool,
}

/// One month row nested under a `YearFacetOption` — only ever populated
/// for the single currently-selected year, since the date-issued facet
/// has at most one active year at a time (TDR 015 §3).
pub struct MonthFacetOption {
    pub label: &'static str,
    pub value: u8,
    pub count: i64,
    pub checked: bool,
}

pub struct YearFacetOption {
    pub year: i32,
    pub count: i64,
    pub checked: bool,
    pub months: Vec<MonthFacetOption>,
}

/// One checkbox in the Language facet group. `value` is either a real ISO
/// 639-1 code actually present among the tenant's documents, or the
/// `unset` sentinel meaning `language is null` — since feature 020 opened
/// `language` up to any ISO 639-1 code, this list is discovered per-tenant
/// (see `list`'s facet-discovery query) rather than a fixed 3-option set.
pub struct LanguageFacetOption {
    pub value: String,
    pub label: String,
    pub count: i64,
    pub checked: bool,
}

/// One checkbox in the Document type facet group (feature 024) — same
/// shape/semantics as `LanguageFacetOption` (open string values, an
/// `"unset"` sentinel, OR-within-facet), since `doc_type` is structurally
/// the same kind of facet as `language`, not the single-active-value date
/// facet.
pub struct DocTypeFacetOption {
    pub value: String,
    pub label: String,
    pub count: i64,
    pub checked: bool,
}

/// One checkbox in the Expiry status facet group (feature 031) — same
/// OR-within-facet interaction as tags/language/doc_type, but the four
/// candidates (`"expired"`/`"soon"`/`"later"`/`"unset"`) are fixed status
/// buckets computed from `date_expires`, not distinct stored column
/// values, so there's no "discover candidates from the DB" step behind
/// this one (see TDR 031 §3).
pub struct ExpiryStatusFacetOption {
    pub value: &'static str,
    pub label: &'static str,
    pub count: i64,
    pub checked: bool,
}

/// A single removable chip above the results — `remove_href` is a
/// pre-built `/documents?...` URL with just this one filter dropped,
/// computed in the handler (see `build_documents_url`) since Askama has
/// no query-string-building filter of its own.
pub struct AppliedFilterChip {
    pub label: String,
    pub remove_href: String,
}

/// One row in the "My collections" panel (feature 016) — `count` is the
/// live number of documents currently matching this collection's saved
/// filters, recomputed on every render (TDR 016 §3), never a snapshot
/// frozen at save time.
pub struct CollectionOption {
    pub id: uuid::Uuid,
    pub name: String,
    pub href: String,
    pub count: i64,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "documents_list.html")]
pub struct DocumentsListTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub q: String,
    pub sort: &'static str,
    pub deleted: bool,
    pub expiring_documents: Vec<ExpiringDocument>,
    pub documents: Vec<DocumentListItem>,
    pub tag_facets: Vec<TagFacetOption>,
    pub year_facets: Vec<YearFacetOption>,
    pub undated_count: i64,
    pub undated_checked: bool,
    pub language_facets: Vec<LanguageFacetOption>,
    pub doc_type_facets: Vec<DocTypeFacetOption>,
    pub expiry_status_facets: Vec<ExpiryStatusFacetOption>,
    pub applied_filters: Vec<AppliedFilterChip>,
    /// `None` when no facet is active — the template uses this both to
    /// decide whether to render "Clear all" and to pick between the
    /// true first-run empty state and the filtered-to-zero one (AC-8).
    pub clear_filters_href: Option<String>,
    pub collections: Vec<CollectionOption>,
    /// Whether *any* filter — a facet or the free-text search box — is
    /// active, gating the "Save this search" control (feature 016 AC-3).
    /// Broader than `clear_filters_href.is_some()`, which only tracks
    /// facets, not `q`.
    pub can_save_search: bool,
    /// The query string (no leading `/documents?`) to embed as the
    /// "Save this search" form's hidden `query` field — the exact state
    /// a saved collection will bookmark.
    pub save_search_query: String,
    /// `"?q=<url-encoded search text>"`, or `""` when no free-text search
    /// is active (feature 027) — appended to every row's link into
    /// `/documents/{id}` so the detail page knows what to highlight.
    pub detail_link_query: String,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "document_show.html")]
pub struct DocumentShowTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub saved: bool,
    pub uploaded: bool,
    pub reprocessing: bool,
    pub id: uuid::Uuid,
    pub title: String,
    pub original_filename: String,
    pub content_type: String,
    pub file_size_bytes: i64,
    pub file_url: String,
    pub is_image: bool,
    pub tags_input_value: String,
    pub date_issued_input_value: String,
    /// `Some(formatted date)` only when OCR found a plausible issued date
    /// *and* `date_issued` is still empty — `None` covers both "no
    /// suggestion was found" and "a date is already set," so the template
    /// only needs one condition to decide whether to show the suggestion
    /// box (see TDR 012).
    pub suggested_date_issued_display: Option<String>,
    pub uploaded_at: String,
    pub ocr_status: String,
    /// Pre-rendered safe HTML (feature 027): the extracted text, escaped,
    /// with any matches for `highlighting_query` wrapped in `<mark>` —
    /// rendered with `|safe`. With no active `q`, this is just the escaped
    /// text, byte-identical to what auto-escaping produced pre-027.
    pub ocr_text_html: Option<String>,
    /// `Some(search text)` only once something in *this* document's OCR
    /// text actually matched it — gates the "Highlighting matches for
    /// ..." indicator so it's never shown for a `q` that doesn't appear
    /// here (feature 027, TDR 027 §3 AC-6).
    pub highlighting_query: Option<String>,
    /// `""` or a real ISO 639-1 code — matches the `<select>`'s option
    /// values directly (see TDR 014, generalized by TDR 020), so the
    /// template can compare with `==` rather than needing an `Option` +
    /// extra branch.
    pub language: String,
    /// The dropdown's first `<optgroup>` — the 4 languages OCR is actually
    /// tuned for (`crate::languages::OCR_SUPPORTED`).
    pub supported_language_options: Vec<crate::languages::LanguageOption>,
    /// The dropdown's second `<optgroup>` — every other ISO 639-1 language,
    /// alphabetical, manual-tagging only (see TDR 020).
    pub other_language_options: Vec<crate::languages::LanguageOption>,
    /// `""` or one of `doc_type_options`' values — same "matches the
    /// `<select>` directly" convention as `language` above (feature 024).
    pub doc_type: String,
    pub doc_type_options: &'static [crate::doc_type_extract::DocTypeOption],
    /// `Some(label)` only when OCR suggested a type *and* `doc_type` is
    /// still empty — same "one condition, not two" shape as
    /// `suggested_date_issued_display` (TDR 024, mirroring TDR 012).
    pub suggested_doc_type_display: Option<&'static str>,
    /// `Some` only immediately after upload (`uploaded == true`) when this
    /// document's content exactly matches an earlier one — a one-shot
    /// warning, never shown again on a later plain visit (feature 029,
    /// TDR 029 §3 Alternative E).
    pub duplicate_of: Option<DuplicateMatch>,
    /// Whether the *confirmed* `doc_type` structurally has an expiry
    /// date — gates the Expires field's visibility entirely, not just
    /// its suggestion box (feature 031, TDR 031 §3, AC-1).
    pub is_expiry_eligible: bool,
    pub date_expires_input_value: String,
    /// Same "only when unset" rule as `suggested_date_issued_display`
    /// (feature 031, mirroring TDR 012).
    pub suggested_date_expires_display: Option<String>,
}

/// The oldest earlier document sharing this one's exact content hash —
/// see `DocumentShowTemplate::duplicate_of`.
pub struct DuplicateMatch {
    pub id: uuid::Uuid,
    pub title: String,
    pub uploaded_at: String,
}

/// One row in the dashboard's "expiring soon" strip (feature 031) —
/// computed from the tenant's full expiry-eligible set regardless of
/// whatever facets/search are currently active on the results below it
/// (TDR 031 §3/§4).
pub struct ExpiringDocument {
    pub id: uuid::Uuid,
    pub title: String,
    pub status_label: String,
    /// Distinguishes the "already expired" (red) vs. "expiring soon"
    /// (stamp-green) styling the mockup gives these rows.
    pub is_expired: bool,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "document_delete.html")]
pub struct DocumentDeleteTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub id: uuid::Uuid,
    pub title: String,
    pub original_filename: String,
}

/// One row in the bulk-delete confirm list (feature 026) — same fields as
/// `DocumentDeleteTemplate`'s single-document shape, just collected.
pub struct BulkDeleteDocumentSummary {
    pub id: uuid::Uuid,
    pub title: String,
    pub original_filename: String,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "document_bulk_delete_confirm.html")]
pub struct DocumentBulkDeleteConfirmTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub documents: Vec<BulkDeleteDocumentSummary>,
    /// Threaded straight back through as a hidden field so the real
    /// "Delete N documents" button's POST still carries the dashboard's
    /// filter state (see `bulk_redirect_target`).
    pub return_to: String,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "document_new.html")]
pub struct DocumentNewTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
}

/// The desktop side of feature 009's phone-camera scan handoff: a QR code
/// and a "waiting for your phone" state, polled via the same
/// `<meta http-equiv="refresh">` idiom `document_show.html` already uses for
/// OCR-status polling (see `docs/tdr/009_phone_camera_scan_design.md`
/// Alternative F).
#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "scan_new.html")]
pub struct ScanNewTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub scan_url: String,
    /// Raw `<svg>...</svg>` markup (no XML prolog), rendered with
    /// `|safe` — colored via `var(--ink)`/`var(--paper-raised)` so it
    /// follows the page's active light/dark theme like everything else.
    pub qr_svg: String,
}

/// The mutually-exclusive states `scan_phone.html` can render — an enum for
/// the same illegal-states-unrepresentable reason as feature 009's original
/// three-variant version; feature 022 added `Capturing` and gave the two
/// page-carrying variants their counts.
pub enum ScanPhoneState {
    /// Finalized — one document created from the session's pages. The count
    /// is 0 only for sessions captured before feature 022 existed (no
    /// `scan_pages` rows) — the template shows the count only when > 0.
    Captured(i64),
    /// Unexpired token, no pages yet — show the first capture form.
    Capture,
    /// ≥1 page uploaded, not finished — the capture-next / finish decision
    /// screen, with the running page count (feature 022).
    Capturing(i64),
    /// Unknown, expired, or already-finalized-and-gone token.
    Invalid,
}

/// The desktop `GET /scan` page once the phone has started capturing
/// (feature 022): the QR is deliberately gone — spent, and hiding it stops
/// a second device joining mid-session — replaced by a live page count on
/// the same meta-refresh poll.
#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "scan_progress.html")]
pub struct ScanProgressTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub page_count: i64,
}

/// The phone side of the scan handoff — public, unauthenticated (the phone
/// never logs in; the token itself is the credential).
#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "scan_phone.html")]
pub struct ScanPhoneTemplate {
    /// Always `false` — the phone is never logged in — but `base.html`'s
    /// nav block still needs it, like every other template extending it.
    pub authenticated: bool,
    pub active_tab: &'static str,
    pub nav_avatar_url: Option<String>,
    pub state: ScanPhoneState,
    pub token: String,
}
