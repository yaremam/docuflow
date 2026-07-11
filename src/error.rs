use std::net::SocketAddr;

use crate::telemetry;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("telemetry initialization failed: {0}")]
    Telemetry(#[from] telemetry::TelemetryError),
    #[error("missing required environment variable: {0}")]
    MissingConfig(&'static str),
    #[error("invalid value for environment variable {0}: {1}")]
    InvalidConfig(&'static str, String),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("database migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("failed to bind listener on {0}: {1}")]
    Bind(SocketAddr, #[source] std::io::Error),
    #[error("HTTP server error: {0}")]
    Serve(#[from] std::io::Error),
}
