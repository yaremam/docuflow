mod common;

#[tokio::test]
async fn get_login_renders_form() {
    let response = common::get("/login").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("<form"));
    assert!(body.contains("action=\"/login\""));
    assert!(body.contains("name=\"email\""));
    assert!(body.contains("name=\"password\""));
}

#[tokio::test]
async fn post_login_with_valid_data_is_stubbed_not_implemented() {
    let response =
        common::post_form("/login", "email=existing.user%40example.com&password=hunter2").await;

    assert_eq!(response.status(), axum::http::StatusCode::NOT_IMPLEMENTED);
    let body = common::body_string(response).await.to_lowercase();
    assert!(body.contains("coming soon") || body.contains("not yet"));
}
