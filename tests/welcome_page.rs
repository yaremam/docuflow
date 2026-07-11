mod common;

#[tokio::test]
async fn get_welcome_renders_signup_confirmation_by_default() {
    let app = common::test_state().await;
    let response = common::get(&app, "/welcome").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await.to_lowercase();
    assert!(body.contains("workspace has been created"));
}

#[tokio::test]
async fn get_welcome_renders_login_confirmation_when_returning() {
    let app = common::test_state().await;
    let response = common::get(&app, "/welcome?returning=true").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await.to_lowercase();
    assert!(body.contains("welcome back"));
}
