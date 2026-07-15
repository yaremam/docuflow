use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthStatus {
    name: &'static str,
    version: &'static str,
    revision: String,
    status: &'static str,
}

#[tracing::instrument]
pub async fn show() -> Json<HealthStatus> {
    Json(HealthStatus {
        name: "DocuFlow",
        version: env!("CARGO_PKG_VERSION"),
        // Baked into the image as a runtime env var by the nightly pipeline
        // (Dockerfile `ARG GIT_SHA` → `ENV`, feature 021) rather than
        // compile-time — a runtime lookup can't invalidate build caches and
        // "dev" cleanly marks any binary that didn't come from the pipeline.
        revision: std::env::var("GIT_SHA").unwrap_or_else(|_| "dev".to_string()),
        status: "healthy",
    })
}
