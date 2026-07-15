//! Feature 022: one scan session accumulates pages, Finish produces one PDF
//! document. See `docs/backlog/022_multipage_scan.md` and TDR 022.

mod common;

use docuflow::web::forms::ScanToken;

/// Real image bytes — the finish step decodes/embeds them into the PDF, so
/// unlike `scan_flow.rs`'s placeholder byte strings these must actually
/// parse as images.
const JPEG_PAGE: &[u8] = include_bytes!("fixtures/dated_sample_with_exif.jpg");
const PNG_PAGE: &[u8] = include_bytes!("fixtures/english_sample.png");

struct SignedUpUser {
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
    cookie: String,
}

async fn signed_up_user(app: &common::TestApp, email: &str) -> SignedUpUser {
    let login = common::signup_and_login(app, email, "multipagepassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let id = sqlx::query_scalar!("select id from users where email = $1", email)
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    SignedUpUser { tenant_id: id, user_id: id, cookie }
}

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

async fn capture_page(
    app: &common::TestApp,
    token: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> axum::http::Response<axum::body::Body> {
    common::post_multipart_with_cookie(
        app,
        &format!("/scan/{token}"),
        "", // phone side is never logged in
        "photo",
        filename,
        content_type,
        bytes,
    )
    .await
}

async fn finish(app: &common::TestApp, token: &str) -> axum::http::Response<axum::body::Body> {
    common::post_form(app, &format!("/scan/{token}/finish"), "").await
}

struct SessionRow {
    status: String,
    document_id: Option<uuid::Uuid>,
}

async fn session_by_token(app: &common::TestApp, token: &str) -> SessionRow {
    let hash = ScanToken::from(token.to_string()).hash();
    sqlx::query_as!(
        SessionRow,
        "select status, document_id from scan_sessions where token_hash = $1",
        hash,
    )
    .fetch_one(&app.state.pool)
    .await
    .expect("session should exist")
}

#[tokio::test]
async fn two_pages_then_finish_creates_one_pdf_document() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "multi.two.pages@example.com").await;
    let token = seed_scan_session(&app, &user, 10).await;

    let first = capture_page(&app, &token, "page1.jpg", "image/jpeg", JPEG_PAGE).await;
    assert_eq!(first.status(), axum::http::StatusCode::OK);
    let first_body = common::body_string(first).await.to_lowercase();
    assert!(first_body.contains("page 1 added"), "phone should confirm the page: {first_body:.300}");

    let second = capture_page(&app, &token, "page2.png", "image/png", PNG_PAGE).await;
    assert_eq!(second.status(), axum::http::StatusCode::OK);
    let second_body = common::body_string(second).await.to_lowercase();
    assert!(second_body.contains("page 2 added"));
    assert!(second_body.contains("finish"), "the decision screen should offer finishing");

    assert_eq!(session_by_token(&app, &token).await.status, "capturing");

    let finished = finish(&app, &token).await;
    assert_eq!(finished.status(), axum::http::StatusCode::OK);
    let finished_body = common::body_string(finished).await.to_lowercase();
    assert!(finished_body.contains("2 pages"));

    let session = session_by_token(&app, &token).await;
    assert_eq!(session.status, "captured");
    let document_id = session.document_id.expect("finished session should record its document");

    let doc = sqlx::query!(
        "select tenant_id, user_id, content_type, blob_key, ocr_status from documents where id = $1",
        document_id,
    )
    .fetch_one(&app.state.pool)
    .await
    .unwrap();
    assert_eq!(doc.tenant_id, user.tenant_id);
    assert_eq!(doc.user_id, user.user_id);
    assert_eq!(doc.content_type, "application/pdf");
    assert!(doc.ocr_status == "pending" || doc.ocr_status == "processing" || doc.ocr_status == "done");

    // Exactly one document came out of the whole session.
    let count = sqlx::query_scalar!(
        "select count(*) from documents where user_id = $1",
        user.user_id,
    )
    .fetch_one(&app.state.pool)
    .await
    .unwrap();
    assert_eq!(count, Some(1));

    // And the blob really is a PDF with both pages.
    let pdf_bytes = app.state.blob.get_object(&doc.blob_key).await.expect("document blob should exist");
    assert!(pdf_bytes.starts_with(b"%PDF"), "assembled blob should be a PDF");
    let parsed = lopdf::Document::load_mem(&pdf_bytes).expect("assembled PDF should reparse");
    assert_eq!(parsed.get_pages().len(), 2);
}

#[tokio::test]
async fn single_page_scan_still_works_end_to_end() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "multi.single.page@example.com").await;
    let token = seed_scan_session(&app, &user, 10).await;

    capture_page(&app, &token, "only.jpg", "image/jpeg", JPEG_PAGE).await;
    let finished = finish(&app, &token).await;
    assert_eq!(finished.status(), axum::http::StatusCode::OK);

    let session = session_by_token(&app, &token).await;
    assert_eq!(session.status, "captured");
    let document_id = session.document_id.expect("session should record its document");

    let doc = sqlx::query!("select content_type, blob_key from documents where id = $1", document_id)
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert_eq!(doc.content_type, "application/pdf");
    let pdf_bytes = app.state.blob.get_object(&doc.blob_key).await.expect("document blob should exist");
    let parsed = lopdf::Document::load_mem(&pdf_bytes).expect("assembled PDF should reparse");
    assert_eq!(parsed.get_pages().len(), 1);
}

#[tokio::test]
async fn finish_with_no_pages_is_rejected() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "multi.zero.pages@example.com").await;
    let token = seed_scan_session(&app, &user, 10).await;

    let response = finish(&app, &token).await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(session_by_token(&app, &token).await.status, "pending");
}

#[tokio::test]
async fn double_finish_does_not_create_a_second_document() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "multi.double.finish@example.com").await;
    let token = seed_scan_session(&app, &user, 10).await;

    capture_page(&app, &token, "page1.jpg", "image/jpeg", JPEG_PAGE).await;
    let first = finish(&app, &token).await;
    assert_eq!(first.status(), axum::http::StatusCode::OK);

    // The common double-tap: the second finish re-renders the captured
    // state rather than erroring or re-assembling (TDR 022 §3).
    let second = finish(&app, &token).await;
    assert_eq!(second.status(), axum::http::StatusCode::OK);

    let count = sqlx::query_scalar!(
        "select count(*) from documents where user_id = $1",
        user.user_id,
    )
    .fetch_one(&app.state.pool)
    .await
    .unwrap();
    assert_eq!(count, Some(1));
}

#[tokio::test]
async fn capturing_a_page_slides_the_session_expiry() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "multi.sliding.expiry@example.com").await;
    // Seeded with only 2 minutes left — a capture must push it back out to
    // the full 10-minute TTL (AC-5).
    let token = seed_scan_session(&app, &user, 2).await;

    capture_page(&app, &token, "page1.jpg", "image/jpeg", JPEG_PAGE).await;

    let hash = ScanToken::from(token.clone()).hash();
    let minutes_left = sqlx::query_scalar!(
        "select (extract(epoch from (expires_at - now())) / 60)::float8 from scan_sessions where token_hash = $1",
        hash,
    )
    .fetch_one(&app.state.pool)
    .await
    .unwrap()
    .expect("expiry interval should compute");
    assert!(minutes_left > 9.0, "expiry should slide to the full TTL, got {minutes_left} minutes");
}

#[tokio::test]
async fn phone_page_while_capturing_shows_count_and_finish_action() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "multi.phone.midway@example.com").await;
    let token = seed_scan_session(&app, &user, 10).await;

    capture_page(&app, &token, "page1.jpg", "image/jpeg", JPEG_PAGE).await;

    // A plain GET (e.g. the phone reloading) shows the same decision screen
    // the post-capture response rendered.
    let response = common::get(&app, &format!("/scan/{token}")).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains(&format!("action=\"/scan/{token}\"")), "capture-next form should be present");
    assert!(body.contains(&format!("action=\"/scan/{token}/finish\"")), "finish form should be present");
    assert!(body.to_lowercase().contains("1 page"));
}

#[tokio::test]
async fn desktop_scan_page_shows_progress_while_capturing() {
    let app = common::test_state().await;
    let user = signed_up_user(&app, "multi.desktop.progress@example.com").await;
    let token = seed_scan_session(&app, &user, 10).await;

    capture_page(&app, &token, "page1.jpg", "image/jpeg", JPEG_PAGE).await;
    capture_page(&app, &token, "page2.png", "image/png", PNG_PAGE).await;

    let response = common::get_with_cookie(&app, &format!("/scan?token={token}"), &user.cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.to_lowercase().contains("2 pages"));
    // The QR code is deliberately gone once pages exist (TDR 022 §3).
    assert!(!body.contains("qr-frame"), "QR should be hidden while capturing");
}

#[tokio::test]
async fn images_to_pdf_assembles_jpeg_and_png_pages_in_order() {
    let pages = vec![
        docuflow::pdf_assemble::PageImage {
            bytes: JPEG_PAGE.to_vec(),
            content_type: "image/jpeg".to_string(),
        },
        docuflow::pdf_assemble::PageImage {
            bytes: PNG_PAGE.to_vec(),
            content_type: "image/png".to_string(),
        },
    ];

    let pdf = docuflow::pdf_assemble::images_to_pdf(&pages).expect("assembly should succeed");

    assert!(pdf.starts_with(b"%PDF"));
    let parsed = lopdf::Document::load_mem(&pdf).expect("output should be a parseable PDF");
    assert_eq!(parsed.get_pages().len(), 2);
}
