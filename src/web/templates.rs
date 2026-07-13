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
    pub tags: Vec<String>,
    pub date_issued: Option<String>,
    pub uploaded_at: String,
    pub ocr_status: String,
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
    pub documents: Vec<DocumentListItem>,
    pub tag_facets: Vec<TagFacetOption>,
    pub year_facets: Vec<YearFacetOption>,
    pub undated_count: i64,
    pub undated_checked: bool,
    pub language_facets: Vec<LanguageFacetOption>,
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
    pub ocr_text: Option<String>,
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

/// The three mutually-exclusive states `scan_phone.html` can render. A
/// `bool` pair (`valid`/`captured`) would allow four combinations when only
/// three are ever meaningful — this makes the illegal one unrepresentable
/// instead of just unexercised.
pub enum ScanPhoneState {
    /// Just uploaded — confirmation screen.
    Captured,
    /// Still-pending, unexpired token — show the capture form.
    Capture,
    /// Unknown, expired, or already-used token.
    Invalid,
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
