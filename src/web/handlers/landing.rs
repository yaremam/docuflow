use crate::web::templates::LandingTemplate;

#[tracing::instrument]
pub async fn show() -> LandingTemplate {
    LandingTemplate { active_tab: "" }
}
