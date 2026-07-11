use axum::extract::Query;
use serde::Deserialize;

use crate::web::tenancy::MaybeTenantContext;
use crate::web::templates::{LandingTemplate, WelcomeTemplate};

#[tracing::instrument(skip(tenancy))]
pub async fn show(MaybeTenantContext(tenancy): MaybeTenantContext) -> LandingTemplate {
    LandingTemplate {
        active_tab: "",
        authenticated: tenancy.is_some(),
    }
}

#[derive(Debug, Deserialize)]
pub struct WelcomeQuery {
    #[serde(default)]
    returning: bool,
}

#[tracing::instrument(skip(tenancy))]
pub async fn welcome(
    MaybeTenantContext(tenancy): MaybeTenantContext,
    Query(query): Query<WelcomeQuery>,
) -> WelcomeTemplate {
    WelcomeTemplate {
        active_tab: "",
        authenticated: tenancy.is_some(),
        returning: query.returning,
    }
}
