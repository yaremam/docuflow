mod common;

#[tokio::test]
async fn get_static_stylesheet_is_served() {
    let response = common::get("/static/style.css").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(common::content_type(&response).starts_with("text/css"));
}
