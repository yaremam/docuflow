#[tokio::test]
async fn telemetry_initializes_without_panicking() {
    let result = docuflow::telemetry::init_telemetry("http://localhost:4317");
    assert!(
        result.is_ok(),
        "expected telemetry init to succeed even if the collector is unreachable, got: {:?}",
        result.err()
    );
    if let Ok(guard) = result {
        guard.shutdown().await;
    }
}
