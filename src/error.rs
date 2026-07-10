use std::net::SocketAddr;

use crate::telemetry;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("telemetry initialization failed: {0}")]
    Telemetry(#[from] telemetry::TelemetryError),
    #[error("failed to bind listener on {0}: {1}")]
    Bind(SocketAddr, #[source] std::io::Error),
    #[error("HTTP server error: {0}")]
    Serve(#[from] std::io::Error),
}
