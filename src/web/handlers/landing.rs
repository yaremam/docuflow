use axum::extract::{Query, State};
use serde::Deserialize;

use crate::web::error::AppWebError;
use crate::web::nav;
use crate::web::state::AppState;
use crate::web::tenancy::MaybeTenantContext;
use crate::web::templates::{LandingTemplate, WelcomeTemplate};

#[tracing::instrument(skip(tenancy, state))]
pub async fn show(
    MaybeTenantContext(tenancy): MaybeTenantContext,
    State(state): State<AppState>,
) -> Result<LandingTemplate, AppWebError> {
    let nav_avatar_url = match &tenancy {
        Some(t) => nav::avatar_url(&state.pool, &state.blob, t.user_id.0).await?,
        None => None,
    };

    Ok(LandingTemplate {
        active_tab: "",
        authenticated: tenancy.is_some(),
        nav_avatar_url,
    })
}

#[derive(Debug, Deserialize)]
pub struct WelcomeQuery {
    #[serde(default)]
    returning: bool,
}

#[tracing::instrument(skip(tenancy, state))]
pub async fn welcome(
    MaybeTenantContext(tenancy): MaybeTenantContext,
    State(state): State<AppState>,
    Query(query): Query<WelcomeQuery>,
) -> Result<WelcomeTemplate, AppWebError> {
    let nav_avatar_url = match &tenancy {
        Some(t) => nav::avatar_url(&state.pool, &state.blob, t.user_id.0).await?,
        None => None,
    };

    Ok(WelcomeTemplate {
        active_tab: "",
        authenticated: tenancy.is_some(),
        nav_avatar_url,
        returning: query.returning,
    })
}
