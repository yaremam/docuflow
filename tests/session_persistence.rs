mod common;

#[tokio::test]
async fn logout_invalidates_session_so_the_same_cookie_cannot_be_replayed() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "session.user@example.com", "hunter2word").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let logout = common::post_with_cookie(&app, "/logout", &cookie).await;
    assert_eq!(logout.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&logout), Some("/".to_string()));

    // Replaying the exact same cookie after logout must no longer be
    // accepted — the server-side session record was deleted, not just the
    // client-side cookie cleared. Both the success and rejection paths
    // redirect (303), so the distinguishing signal is *where* to: an
    // authenticated logout goes to "/", a rejected one is bounced to
    // "/login".
    let replay = common::post_with_cookie(&app, "/logout", &cookie).await;
    assert_eq!(common::location(&replay), Some("/login".to_string()));
}

#[tokio::test]
async fn logout_with_no_session_cookie_is_rejected() {
    let app = common::test_state().await;
    let response = common::post_form(&app, "/logout", "").await;

    assert_eq!(common::location(&response), Some("/login".to_string()));
}
