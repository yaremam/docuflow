#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "landing.html")]
pub struct LandingTemplate {
    pub active_tab: &'static str,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "signup.html")]
pub struct SignupTemplate {
    pub active_tab: &'static str,
    pub error: Option<&'static str>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub active_tab: &'static str,
    pub error: Option<&'static str>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "welcome.html")]
pub struct WelcomeTemplate {
    pub active_tab: &'static str,
    pub returning: bool,
}
