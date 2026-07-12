mod common;

#[tokio::test]
async fn public_pages_include_the_theme_toggle_button() {
    let app = common::test_state().await;

    let response = common::get(&app, "/").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains(r#"id="theme-toggle""#), "expected the theme toggle button, got: {body}");
    assert!(body.contains("aria-label="), "expected the toggle to have an aria-label, got: {body}");
}

#[tokio::test]
async fn authenticated_pages_include_the_theme_toggle_button() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "themetoggle.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains(r#"id="theme-toggle""#), "expected the theme toggle button, got: {body}");
}

#[tokio::test]
async fn the_anti_flash_theme_script_runs_before_the_stylesheet_loads() {
    let app = common::test_state().await;

    let response = common::get(&app, "/").await;

    let body = common::body_string(response).await;
    let script_pos = body.find("localStorage.getItem('theme')").expect("expected the anti-flash theme script");
    let stylesheet_pos = body.find(r#"<link rel="stylesheet""#).expect("expected the stylesheet link");
    assert!(
        script_pos < stylesheet_pos,
        "the anti-flash theme script must run before the stylesheet is applied, to avoid a flash of the wrong theme"
    );
}
