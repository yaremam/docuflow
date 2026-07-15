//! Deliberately its own test binary, separate from `telemetry_bootstrap.rs`:
//! `init_telemetry` installs the process-global `tracing` subscriber, which
//! can only succeed once per process — a second init in the same binary
//! would fail with `SubscriberInit` no matter what, masking what's actually
//! under test. One binary per init keeps each test observing a fresh
//! process. (Feature 021 — no-collector deployments, e.g. the NAS.)

#[tokio::test]
async fn telemetry_initializes_without_otlp_endpoint() {
    let result = docuflow::telemetry::init_telemetry(None);
    assert!(
        result.is_ok(),
        "expected telemetry init to succeed with no OTLP endpoint configured \
         (stdout-logging-only mode for deployments without a collector), got: {:?}",
        result.err()
    );
    if let Ok(guard) = result {
        guard.shutdown().await;
    }
}
