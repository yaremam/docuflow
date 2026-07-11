mod common;

#[tokio::test]
async fn get_forgot_password_renders_form() {
    let app = common::test_state().await;
    let response = common::get(&app, "/forgot-password").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("<form"));
    assert!(body.contains("action=\"/forgot-password\""));
    assert!(body.contains("name=\"email\""));
}

#[tokio::test]
async fn post_forgot_password_known_and_unknown_email_get_identical_response() {
    let app = common::test_state().await;
    common::signup(&app, "known.reset@example.com", "hunter2word").await;

    let unknown = common::post_form(&app, "/forgot-password", "email=nobody.here%40example.com").await;
    let unknown_status = unknown.status();
    let unknown_location = common::location(&unknown);

    let known = common::post_form(&app, "/forgot-password", "email=known.reset%40example.com").await;
    let known_status = known.status();
    let known_location = common::location(&known);

    assert_eq!(unknown_status, known_status);
    assert_eq!(unknown_location, known_location);
}

#[tokio::test]
async fn post_forgot_password_known_email_creates_a_reset_token_row() {
    let app = common::test_state().await;
    common::signup(&app, "gets.token@example.com", "hunter2word").await;

    let response = common::post_form(&app, "/forgot-password", "email=gets.token%40example.com").await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        common::location(&response),
        Some("/forgot-password?sent=true".to_string())
    );

    let row = sqlx::query!(
        "select prt.id from password_reset_tokens prt \
         join users u on u.id = prt.user_id \
         where u.email = $1",
        "gets.token@example.com",
    )
    .fetch_optional(&app.state.pool)
    .await
    .expect("query should succeed");

    assert!(row.is_some(), "expected a reset token row for the known email");
}

#[tokio::test]
async fn post_forgot_password_unknown_email_creates_no_reset_token_row() {
    let app = common::test_state().await;

    common::post_form(&app, "/forgot-password", "email=still.nobody%40example.com").await;

    let count = sqlx::query!("select count(*) as count from password_reset_tokens")
        .fetch_one(&app.state.pool)
        .await
        .expect("query should succeed")
        .count
        .unwrap_or(0);

    assert_eq!(count, 0);
}
