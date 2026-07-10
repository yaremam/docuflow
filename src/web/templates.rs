#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "landing.html")]
pub struct LandingTemplate {
    pub active_tab: &'static str,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "signup.html")]
pub struct SignupTemplate {
    pub active_tab: &'static str,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub active_tab: &'static str,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "coming_soon.html")]
pub struct ComingSoonTemplate {
    pub feature_name: &'static str,
    pub active_tab: &'static str,
}
