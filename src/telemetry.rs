use std::time::Duration;

use opentelemetry::trace::TraceError;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{runtime::Tokio, trace::Config as TraceConfig, Resource};
use tracing_subscriber::{
    filter::LevelFilter, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("failed to initialize OTLP tracer: {0}")]
    TracerInit(#[from] TraceError),
    #[error("failed to install global tracing subscriber: {0}")]
    SubscriberInit(#[from] tracing_subscriber::util::TryInitError),
}

/// Handle for the global OTel tracer provider set up by `init_telemetry`.
///
/// Call `shutdown().await` before the process exits to flush pending spans.
pub struct TelemetryGuard;

impl TelemetryGuard {
    /// Flushes and shuts down the global tracer provider.
    ///
    /// The underlying OTel shutdown call is synchronous and blocks on the batch
    /// exporter's background task. Running it via `spawn_blocking` keeps it off
    /// the async worker thread(s), so this is safe to await on any Tokio runtime
    /// flavor, including the single-threaded `#[tokio::test]` default — calling
    /// the blocking shutdown directly from a current-thread runtime would
    /// deadlock, since that runtime has no other thread left to poll the
    /// background flush task the shutdown call is waiting on.
    pub async fn shutdown(self) {
        let _ = tokio::task::spawn_blocking(opentelemetry::global::shutdown_tracer_provider).await;
    }
}

/// Initializes the global `tracing` subscriber: a stdout `fmt` layer always
/// (so `docker logs` is useful on deployments with no collector at all —
/// feature 021), plus an OpenTelemetry OTLP/gRPC layer exporting to
/// `otlp_endpoint` when one is configured. With `None`, no exporter is
/// created and no export is ever attempted.
///
/// Must be called from within a Tokio runtime (required by the OTLP batch
/// exporter). The gRPC channel connects lazily, so this succeeds even if the
/// collector isn't reachable yet — spans are simply dropped by the batch
/// processor until it is.
pub fn init_telemetry(otlp_endpoint: Option<&str>) -> Result<TelemetryGuard, TelemetryError> {
    let filter_layer = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    let registry = tracing_subscriber::registry()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer());

    let Some(otlp_endpoint) = otlp_endpoint else {
        registry.try_init()?;
        return Ok(TelemetryGuard);
    };

    let resource = Resource::new(vec![
        KeyValue::new("service.name", env!("CARGO_PKG_NAME")),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
    ]);

    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(otlp_endpoint)
                .with_timeout(Duration::from_secs(3)),
        )
        .with_trace_config(TraceConfig::default().with_resource(resource))
        .install_batch(Tokio)?;

    registry
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .try_init()?;

    Ok(TelemetryGuard)
}
