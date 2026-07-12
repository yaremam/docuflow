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
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "document_show.html")]
pub struct DocumentShowTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
    pub saved: bool,
    pub uploaded: bool,
    pub id: uuid::Uuid,
    pub title: String,
    pub original_filename: String,
    pub content_type: String,
    pub file_size_bytes: i64,
    pub file_url: String,
    pub is_image: bool,
    pub tags_input_value: String,
    pub date_issued_input_value: String,
    pub uploaded_at: String,
    pub ocr_status: String,
    pub ocr_text: Option<String>,
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
