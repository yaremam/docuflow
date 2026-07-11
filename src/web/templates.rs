#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "landing.html")]
pub struct LandingTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "signup.html")]
pub struct SignupTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub error: Option<&'static str>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub error: Option<&'static str>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "welcome.html")]
pub struct WelcomeTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub returning: bool,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "profile.html")]
pub struct ProfileTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
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
    pub sent: bool,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "reset_password.html")]
pub struct ResetPasswordTemplate {
    pub active_tab: &'static str,
    pub authenticated: bool,
    pub valid: bool,
    pub token: String,
}
