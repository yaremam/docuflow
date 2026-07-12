mod common;

use docuflow::web::forms::ScanToken;

struct SignedUpUser {
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
    cookie: String,
}

async fn signed_up_user(app: &common::TestApp, email: &str) -> SignedUpUser {
    let login = common::signup_and_login(app, email, "scanflowpassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let id = sqlx::query_scalar!("select id from users where email = $1", email)
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    SignedUpUser { tenant_id: id, user_id: id, cookie }
}

/// Inserts a `scan_sessions` row directly, bypassing `GET /scan` — mirrors
/// `reset_password_flow.rs::seed_reset_token`'s approach for
/// `password_reset_tokens`. `ttl_minutes` lets tests seed an already-expired
/// row by passing a negative value.
async fn seed_scan_session(app: &common::TestApp, user: &SignedUpUser, ttl_minutes: i32) -> String {
    let token = ScanToken::generate();
    sqlx::query!(
        "insert into scan_sessions (id, tenant_id, user_id, token_hash, expires_at)
         values ($1, $2, $3, $4, now() + make_interval(mins => $5))",
        uuid::Uuid::new_v4(),
        user.tenant_id,
        user.user_id,
        token.hash(),
        ttl_minutes,
    )
    .execute(&app.state.pool)
    .await
    .expect("scan session insert should succeed");
    token.as_str().to_string()
}

struct ScanSessionRow {
    status: String,
    document_id: Option<uuid::Uuid>,
}

async fn find_scan_session_by_token(app: &common::TestApp, token: &str) -> Option<ScanSessionRow> {
    let hash = ScanToken::from(token.to_string()).hash();
    sqlx::query_as!(
        ScanSessionRow,
        "select status, document_id from scan_sessions where token_hash = $1",
        hash,
    )
    .fetch_optional(&app.state.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn get_scan_without_session_redirects_to_login() {
    let app = common::test_state().await;

    let response = common::get(&app, "/scan").await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response), Some("/login".to_string()));
}

#[tokio::test]
async fn get_scan_mints_a_session_and_redirects_to_the_qr_page() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "scan.mint@example.com").await;

    let response = common::get_with_cookie(&app, "/scan", &user.cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let location = common::location(&response).expect("should redirect to the QR page");
    assert!(location.starts_with("/scan?token="));

    let qr_page = common::get_with_cookie(&app, &location, &user.cookie).await;
    assert_eq!(qr_page.status(), axum::http::StatusCode::OK);
    let body = common::body_string(qr_page).await;
    assert!(body.contains("<svg"));
    assert!(body.contains("/scan/"));
    assert!(body.to_lowercase().contains("waiting"));
}

#[tokio::test]
async fn get_scan_phone_with_valid_token_shows_capture_form() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "scan.phone.valid@example.com").await;
    let token = seed_scan_session(&app, &user, 10).await;

    let response = common::get(&app, &format!("/scan/{token}")).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains(&format!("action=\"/scan/{token}\"")));
    assert!(body.contains("capture=\"environment\""));
}

#[tokio::test]
async fn get_scan_phone_with_unknown_token_shows_invalid_state() {
    let app = common::test_state().await;

    let response = common::get(&app, "/scan/not-a-real-token").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await.to_lowercase();
    assert!(body.contains("invalid") || body.contains("expired") || body.contains("valid"));
    assert!(!body.contains("action=\"/scan/not-a-real-token\""));
}

#[tokio::test]
async fn get_scan_phone_with_expired_token_shows_invalid_state() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "scan.phone.expired@example.com").await;
    let token = seed_scan_session(&app, &user, -1).await;

    let response = common::get(&app, &format!("/scan/{token}")).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(!body.contains(&format!("action=\"/scan/{token}\"")));
}

#[tokio::test]
async fn post_scan_phone_with_valid_image_creates_document_and_marks_session_captured() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "scan.phone.capture@example.com").await;
    let token = seed_scan_session(&app, &user, 10).await;

    let response = common::post_multipart_with_cookie(
        &app,
        &format!("/scan/{token}"),
        "", // phone side is never logged in — no cookie sent
        "photo",
        "capture.jpg",
        "image/jpeg",
        b"pretend this is jpeg bytes",
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await.to_lowercase();
    assert!(body.contains("received") || body.contains("close"));

    let session = find_scan_session_by_token(&app, &token).await.expect("session should still exist");
    assert_eq!(session.status, "captured");
    let document_id = session.document_id.expect("captured session should record a document id");

    let doc = sqlx::query!(
        "select tenant_id, user_id, original_filename, ocr_status from documents where id = $1",
        document_id,
    )
    .fetch_one(&app.state.pool)
    .await
    .unwrap();
    assert_eq!(doc.tenant_id, user.tenant_id);
    assert_eq!(doc.user_id, user.user_id);
    assert_eq!(doc.original_filename, "capture.jpg");
    assert!(doc.ocr_status == "pending" || doc.ocr_status == "processing" || doc.ocr_status == "done");
}

#[tokio::test]
async fn post_scan_phone_with_disallowed_content_type_is_rejected() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "scan.phone.badtype@example.com").await;
    let token = seed_scan_session(&app, &user, 10).await;

    // image/tiff is accepted on the desktop `POST /documents` upload but is
    // not one of the two types a phone camera actually produces (AC-3).
    let response = common::post_multipart_with_cookie(
        &app,
        &format!("/scan/{token}"),
        "",
        "photo",
        "capture.tiff",
        "image/tiff",
        b"irrelevant",
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let session = find_scan_session_by_token(&app, &token).await.expect("session should still exist");
    assert_eq!(session.status, "pending");
    assert!(session.document_id.is_none());
}

#[tokio::test]
async fn post_scan_phone_with_expired_token_is_rejected() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "scan.phone.postexpired@example.com").await;
    let token = seed_scan_session(&app, &user, -1).await;

    let response = common::post_multipart_with_cookie(
        &app,
        &format!("/scan/{token}"),
        "",
        "photo",
        "capture.jpg",
        "image/jpeg",
        b"irrelevant",
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_scan_phone_with_already_captured_token_is_rejected() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "scan.phone.reused@example.com").await;
    let token = seed_scan_session(&app, &user, 10).await;

    let first = common::post_multipart_with_cookie(
        &app,
        &format!("/scan/{token}"),
        "",
        "photo",
        "first.jpg",
        "image/jpeg",
        b"first capture",
    )
    .await;
    assert_eq!(first.status(), axum::http::StatusCode::OK);

    let second = common::post_multipart_with_cookie(
        &app,
        &format!("/scan/{token}"),
        "",
        "photo",
        "second.jpg",
        "image/jpeg",
        b"second capture",
    )
    .await;
    assert_eq!(second.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_scan_redirects_to_the_document_once_captured() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "scan.desktop.followup@example.com").await;
    let token = seed_scan_session(&app, &user, 10).await;

    common::post_multipart_with_cookie(
        &app,
        &format!("/scan/{token}"),
        "",
        "photo",
        "capture.jpg",
        "image/jpeg",
        b"pretend this is jpeg bytes",
    )
    .await;

    let response = common::get_with_cookie(&app, &format!("/scan?token={token}"), &user.cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let location = common::location(&response).expect("should redirect to the new document");
    assert!(location.starts_with("/documents/"));
    assert!(location.ends_with("?uploaded=true"));
}
