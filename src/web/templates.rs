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
    pub tags_input_value: String,
    pub date_issued_input_value: String,
    pub uploaded_at: String,
    pub ocr_status: String,
    pub ocr_text: Option<String>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "document_new.html")]
pub struct DocumentNewTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub nav_avatar_url: Option<String>,
}
