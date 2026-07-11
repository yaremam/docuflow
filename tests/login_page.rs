mod common;

#[tokio::test]
async fn get_login_renders_form() {
    let app = common::test_state().await;
    let response = common::get(&app, "/login").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("<form"));
    assert!(body.contains("action=\"/login\""));
    assert!(body.contains("name=\"email\""));
    assert!(body.contains("name=\"password\""));
}

#[tokio::test]
async fn post_login_with_correct_password_establishes_session() {
    let app = common::test_state().await;
    let response =
        common::signup_and_login(&app, "existing.user@example.com", "hunter2word").await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        common::location(&response),
        Some("/welcome?returning=true".to_string())
    );
    assert!(common::session_cookie(&response).is_some());
}

#[tokio::test]
async fn post_login_with_wrong_password_is_rejected() {
    let app = common::test_state().await;
    common::signup(&app, "wrong.pw@example.com", "hunter2word").await;

    let response = common::login(&app, "wrong.pw@example.com", "not-the-password").await;

    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert!(common::session_cookie(&response).is_none());
    let body = common::body_string(response).await.to_lowercase();
    assert!(body.contains("invalid"));
}

#[tokio::test]
async fn post_login_with_unknown_email_is_rejected_identically_to_wrong_password() {
    let app = common::test_state().await;

    let unknown = common::login(&app, "nobody.here@example.com", "whatever").await;
    let unknown_status = unknown.status();
    let unknown_body = common::body_string(unknown).await;

    common::signup(&app, "known.user@example.com", "hunter2word").await;
    let wrong_password = common::login(&app, "known.user@example.com", "not-the-password").await;
    let wrong_password_status = wrong_password.status();
    let wrong_password_body = common::body_string(wrong_password).await;

    assert_eq!(unknown_status, wrong_password_status);
    assert_eq!(unknown_body, wrong_password_body);
}
