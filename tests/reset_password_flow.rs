mod common;

use docuflow::web::forms::ResetToken;

/// Signs up a fresh account and inserts a reset token row directly (bypassing
/// SMTP/Mailpit, which this in-process test suite has no client for — see
/// the plan's rationale in `docs/tdr/006_forgot_password_design.md`).
/// Returns the raw token string, usable exactly once by the caller.
async fn seed_reset_token(app: &common::TestApp, email: &str) -> String {
    common::signup(app, email, "original-password").await;

    let user = sqlx::query!("select id from users where email = $1", email)
        .fetch_one(&app.state.pool)
        .await
        .expect("signed-up user should exist");

    let token = ResetToken::generate();
    sqlx::query!(
        "insert into password_reset_tokens (id, user_id, token_hash, expires_at) \
         values ($1, $2, $3, now() + interval '1 hour')",
        uuid::Uuid::new_v4(),
        user.id,
        token.hash(),
    )
    .execute(&app.state.pool)
    .await
    .expect("token insert should succeed");

    token.as_str().to_string()
}

#[tokio::test]
async fn get_reset_password_with_valid_token_renders_the_form() {
    let app = common::test_state().await;
    let token = seed_reset_token(&app, "valid.token@example.com").await;

    let response = common::get(&app, &format!("/reset-password?token={token}")).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("action=\"/reset-password\""));
    assert!(body.contains(&token));
}

#[tokio::test]
async fn get_reset_password_with_garbage_token_renders_invalid_state() {
    let app = common::test_state().await;

    let response = common::get(&app, "/reset-password?token=not-a-real-token").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await.to_lowercase();
    assert!(body.contains("invalid") || body.contains("expired"));
    assert!(!body.contains("action=\"/reset-password\""));
}

#[tokio::test]
async fn post_reset_password_with_valid_token_resets_password_and_logs_in() {
    let app = common::test_state().await;
    let token = seed_reset_token(&app, "resets.password@example.com").await;

    let response = common::post_form(
        &app,
        "/reset-password",
        &format!("token={token}&password=brand-new-password"),
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response), Some("/welcome".to_string()));
    assert!(common::session_cookie(&response).is_some());

    // Old password no longer works, new one does.
    let old_login = common::login(&app, "resets.password@example.com", "original-password").await;
    assert_eq!(old_login.status(), axum::http::StatusCode::UNAUTHORIZED);

    let new_login = common::login(&app, "resets.password@example.com", "brand-new-password").await;
    assert_eq!(new_login.status(), axum::http::StatusCode::SEE_OTHER);
    assert!(common::session_cookie(&new_login).is_some());
}

#[tokio::test]
async fn post_reset_password_with_already_used_token_is_rejected() {
    let app = common::test_state().await;
    let token = seed_reset_token(&app, "reused.token@example.com").await;

    let first = common::post_form(
        &app,
        "/reset-password",
        &format!("token={token}&password=first-new-password"),
    )
    .await;
    assert_eq!(first.status(), axum::http::StatusCode::SEE_OTHER);

    let second = common::post_form(
        &app,
        "/reset-password",
        &format!("token={token}&password=second-new-password"),
    )
    .await;
    assert_eq!(second.status(), axum::http::StatusCode::BAD_REQUEST);
}
