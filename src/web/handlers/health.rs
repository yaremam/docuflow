use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthStatus {
    name: &'static str,
    version: &'static str,
    status: &'static str,
}

#[tracing::instrument]
pub async fn show() -> Json<HealthStatus> {
    Json(HealthStatus {
        name: "DocuFlow",
        version: env!("CARGO_PKG_VERSION"),
        status: "healthy",
    })
}
