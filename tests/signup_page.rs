mod common;

#[tokio::test]
async fn get_signup_renders_form() {
    let app = common::test_state().await;
    let response = common::get(&app, "/signup").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("<form"));
    assert!(body.contains("action=\"/signup\""));
    assert!(body.contains("name=\"email\""));
    assert!(body.contains("name=\"password\""));
}

#[tokio::test]
async fn post_signup_with_valid_data_creates_user_and_establishes_session() {
    let app = common::test_state().await;
    let response = common::post_form(
        &app,
        "/signup",
        "email=new.user%40example.com&password=hunter2word",
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response), Some("/welcome".to_string()));
    assert!(
        common::session_cookie(&response).is_some(),
        "signup should establish an authenticated session"
    );

    let row = sqlx::query!(
        "select password_hash from users where email = $1",
        "new.user@example.com",
    )
    .fetch_one(&app.state.pool)
    .await
    .expect("exactly one user row should have been created");

    assert!(row.password_hash.starts_with("$argon2"));
    assert_ne!(row.password_hash, "hunter2word");
}

#[tokio::test]
async fn post_signup_with_duplicate_email_is_rejected_without_creating_a_second_row() {
    let app = common::test_state().await;
    let form_body = "email=dup.user%40example.com&password=hunter2word";

    let first = common::post_form(&app, "/signup", form_body).await;
    assert_eq!(first.status(), axum::http::StatusCode::SEE_OTHER);

    let second = common::post_form(&app, "/signup", form_body).await;
    assert_eq!(second.status(), axum::http::StatusCode::CONFLICT);

    let count = sqlx::query_scalar!(
        "select count(*) from users where email = $1",
        "dup.user@example.com",
    )
    .fetch_one(&app.state.pool)
    .await
    .unwrap();
    assert_eq!(count, Some(1));
}

#[tokio::test]
async fn post_signup_with_malformed_email_is_bad_request_not_a_panic() {
    let app = common::test_state().await;
    let response = common::post_form(&app, "/signup", "email=not-an-email&password=hunter2word").await;

    // Axum's `Form` extractor rejects a failed deserialize with 422 (well-formed
    // request, semantically invalid field) rather than 400 — still a proper
    // 4xx client error, not a panic.
    assert_eq!(response.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn post_signup_with_short_password_is_bad_request_not_a_panic() {
    let app = common::test_state().await;
    let response =
        common::post_form(&app, "/signup", "email=short.pw%40example.com&password=short").await;

    assert_eq!(response.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);

    let count = sqlx::query_scalar!(
        "select count(*) from users where email = $1",
        "short.pw@example.com",
    )
    .fetch_one(&app.state.pool)
    .await
    .unwrap();
    assert_eq!(count, Some(0));
}
