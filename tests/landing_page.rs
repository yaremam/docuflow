mod common;

#[tokio::test]
async fn get_root_renders_landing_page() {
    let app = common::test_state().await;
    let response = common::get(&app, "/").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(common::content_type(&response).starts_with("text/html"));

    let body = common::body_string(response).await;
    assert!(body.contains("DocuFlow"));
    assert!(body.contains("href=\"/signup\""));
    assert!(body.contains("href=\"/login\""));
}
