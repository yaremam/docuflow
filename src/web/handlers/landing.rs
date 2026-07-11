use axum::extract::Query;
use serde::Deserialize;

use crate::web::templates::{LandingTemplate, WelcomeTemplate};

#[tracing::instrument]
pub async fn show() -> LandingTemplate {
    LandingTemplate { active_tab: "" }
}

#[derive(Debug, Deserialize)]
pub struct WelcomeQuery {
    #[serde(default)]
    returning: bool,
}

#[tracing::instrument]
pub async fn welcome(Query(query): Query<WelcomeQuery>) -> WelcomeTemplate {
    WelcomeTemplate {
        active_tab: "",
        returning: query.returning,
    }
}
