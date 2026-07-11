mod common;

#[tokio::test]
async fn get_profile_without_session_redirects_to_login() {
    let app = common::test_state().await;
    let response = common::get(&app, "/profile").await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response), Some("/login".to_string()));
}

#[tokio::test]
async fn get_profile_with_session_renders_empty_fields_for_a_new_user() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "profile.viewer@example.com", "profileviewerpw").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::get_with_cookie(&app, "/profile", &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("name=\"first_name\""));
    assert!(body.contains("name=\"street_address\""));
}

#[tokio::test]
async fn post_profile_saves_fields_and_they_persist_on_reload() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "profile.editor@example.com", "profileeditorpw").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let form_body = "first_name=Ada&last_name=Lovelace&street_address=123+Analytical+Engine+Ave&\
        city=London&postcode=SW1A+1AA&country=UK&phone=%2B441234567890";
    let update = common::post_form_with_cookie(&app, "/profile", &cookie, form_body).await;
    assert_eq!(update.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        common::location(&update),
        Some("/profile?saved=true".to_string())
    );

    let response = common::get_with_cookie(&app, "/profile?saved=true", &cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("Ada"));
    assert!(body.contains("Lovelace"));
    assert!(body.contains("London"));
    assert!(body.contains("Saved."));
}
