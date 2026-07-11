mod common;

#[tokio::test]
async fn get_health_returns_json_status() {
    let app = common::test_state().await;
    let response = common::get(&app, "/health").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(common::content_type(&response).starts_with("application/json"));

    let body = common::body_string(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["name"], "DocuFlow");
    assert_eq!(json["status"], "healthy");
    assert!(json["version"].is_string());
}
