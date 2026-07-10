use std::net::SocketAddr;

use docuflow::error::AppError;
use docuflow::{telemetry, web};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let telemetry_guard = telemetry::init_telemetry("http://localhost:4317")?;
    tracing::info!("server booting");

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| AppError::Bind(addr, e))?;

    tracing::info!(%addr, "listening");

    axum::serve(listener, web::router::app())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    telemetry_guard.shutdown().await;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
