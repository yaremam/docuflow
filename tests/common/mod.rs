//! Shared request-building helpers for the integration tests. Each test file
//! compiles as its own crate, so any given binary only uses a subset of these
//! — the resulting dead-code warnings are an accepted characteristic of the
//! `tests/common/mod.rs` pattern, suppressed here rather than per call site.
#![allow(dead_code)]

use http_body_util::BodyExt;
use tower::ServiceExt;

pub async fn get(uri: &str) -> axum::http::Response<axum::body::Body> {
    docuflow::web::router::app()
        .oneshot(
            axum::http::Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

pub async fn post_form(uri: &str, form_body: &str) -> axum::http::Response<axum::body::Body> {
    docuflow::web::router::app()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(uri)
                .header(
                    axum::http::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(axum::body::Body::from(form_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

pub async fn body_string(response: axum::http::Response<axum::body::Body>) -> String {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(body.to_vec()).unwrap()
}

pub fn content_type(response: &axum::http::Response<axum::body::Body>) -> String {
    response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}
