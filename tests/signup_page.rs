mod common;

#[tokio::test]
async fn get_signup_renders_form() {
    let response = common::get("/signup").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("<form"));
    assert!(body.contains("action=\"/signup\""));
    assert!(body.contains("name=\"email\""));
    assert!(body.contains("name=\"password\""));
}

#[tokio::test]
async fn post_signup_with_valid_data_is_stubbed_not_implemented() {
    let response = common::post_form("/signup", "email=new.user%40example.com&password=hunter2").await;

    assert_eq!(response.status(), axum::http::StatusCode::NOT_IMPLEMENTED);
    let body = common::body_string(response).await.to_lowercase();
    assert!(body.contains("coming soon") || body.contains("not yet"));
}

#[tokio::test]
async fn post_signup_with_malformed_email_is_bad_request_not_a_panic() {
    let response = common::post_form("/signup", "email=not-an-email&password=hunter2").await;

    // Axum's `Form` extractor rejects a failed deserialize with 422 (well-formed
    // request, semantically invalid field) rather than 400 — still a proper
    // 4xx client error, not a panic.
    assert_eq!(response.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);
}
