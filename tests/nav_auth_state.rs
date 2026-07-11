mod common;

#[tokio::test]
async fn logged_out_nav_shows_login_and_signup_not_profile() {
    let app = common::test_state().await;
    let response = common::get(&app, "/").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("href=\"/login\""));
    assert!(body.contains("href=\"/signup\""));
    assert!(!body.contains("href=\"/profile\""));
}

#[tokio::test]
async fn logged_in_nav_shows_profile_not_login_or_signup() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "nav.user@example.com", "navuserpassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::get_with_cookie(&app, "/", &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("href=\"/profile\""));
    assert!(!body.contains("href=\"/login\""));
    assert!(!body.contains("href=\"/signup\""));
}
