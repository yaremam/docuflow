use std::net::SocketAddr;

use docuflow::error::AppError;
use docuflow::{telemetry, web};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let _ = dotenvy::dotenv();

    // No silent localhost fallback: unset (or empty) means "no collector
    // here" — stdout logs only. Dev setups export to Jaeger by setting
    // OTLP_ENDPOINT explicitly (`.cargo/config.toml`, `.env`,
    // `docker-compose.yml`); a pulled image (feature 021) runs without one.
    let otlp_endpoint = std::env::var("OTLP_ENDPOINT")
        .ok()
        .filter(|endpoint| !endpoint.is_empty());
    let telemetry_guard = telemetry::init_telemetry(otlp_endpoint.as_deref())?;
    match &otlp_endpoint {
        Some(endpoint) => tracing::info!(otlp_endpoint = %endpoint, "server booting"),
        None => tracing::info!("server booting (trace export disabled: OTLP_ENDPOINT unset)"),
    }

    let database_url = std::env::var("DATABASE_URL").map_err(|_| AppError::MissingConfig("DATABASE_URL"))?;
    let (state, session_layer) = web::state::bootstrap(&database_url).await?;

    let port = match std::env::var("PORT") {
        Ok(port) => port
            .parse::<u16>()
            .map_err(|_| AppError::InvalidConfig("PORT", port))?,
        Err(_) => 8080,
    };
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| AppError::Bind(addr, e))?;

    tracing::info!(%addr, "listening");

    axum::serve(listener, web::router::app(state, session_layer))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    telemetry_guard.shutdown().await;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
