mod common;

use common::MultipartPart;

async fn user_id(app: &common::TestApp, email: &str) -> uuid::Uuid {
    sqlx::query_scalar!("select id from users where email = $1", email)
        .fetch_one(&app.state.pool)
        .await
        .unwrap()
}

struct DocumentRow {
    tenant_id: uuid::Uuid,
    original_filename: String,
    file_size_bytes: i64,
    blob_key: String,
    ocr_status: String,
}

async fn find_document(app: &common::TestApp, id: uuid::Uuid) -> Option<DocumentRow> {
    sqlx::query_as!(
        DocumentRow,
        "select tenant_id, original_filename, file_size_bytes, blob_key, ocr_status from documents where id = $1",
        id,
    )
    .fetch_optional(&app.state.pool)
    .await
    .unwrap()
}

/// Scoped to one tenant rather than a bare `select count(*) from documents`
/// — tests in this suite run concurrently against the same
/// `doc_manager_db_test` database (see `tests/common/mod.rs`), so an
/// unscoped count would flake by picking up rows other tests inserted at
/// the same time.
async fn document_count_for_tenant(app: &common::TestApp, tenant_id: uuid::Uuid) -> i64 {
    sqlx::query_scalar!("select count(*) from documents where tenant_id = $1", tenant_id)
        .fetch_one(&app.state.pool)
        .await
        .unwrap()
        .unwrap_or(0)
}

#[tokio::test]
async fn successful_image_upload_creates_row_and_redirects() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "upload.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "upload.docs@example.com").await;

    let bytes = std::fs::read("tests/fixtures/ocr_sample.png").unwrap();
    let response = common::post_multipart_parts_with_cookie(
        &app,
        "/documents",
        &cookie,
        &[
            MultipartPart::Text { name: "title", value: "Sample scan" },
            MultipartPart::Text { name: "tags", value: "test, sample" },
            MultipartPart::Text { name: "date_issued", value: "" },
            MultipartPart::File {
                name: "file",
                filename: "ocr_sample.png",
                content_type: "image/png",
                bytes: &bytes,
            },
        ],
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let location = common::location(&response).expect("upload should redirect");
    assert!(location.ends_with("?uploaded=true"));
    let id = common::document_id_from_location(&location);

    let row = find_document(&app, id).await.expect("row should exist");
    assert_eq!(row.tenant_id, user);
    assert_eq!(row.original_filename, "ocr_sample.png");
    assert_eq!(row.file_size_bytes as u64, bytes.len() as u64);
    assert_eq!(row.blob_key, format!("documents/{user}/{id}"));
    assert!(row.ocr_status == "pending" || row.ocr_status == "processing" || row.ocr_status == "done");
}

#[tokio::test]
async fn oversized_upload_is_rejected() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "oversized.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "oversized.docs@example.com").await;

    let bytes = vec![0u8; 21 * 1024 * 1024];
    let response = common::post_multipart_parts_with_cookie(
        &app,
        "/documents",
        &cookie,
        &[MultipartPart::File {
            name: "file",
            filename: "huge.png",
            content_type: "image/png",
            bytes: &bytes,
        }],
    )
    .await;

    // `DefaultBodyLimit`'s rejection surfaces through `Multipart::next_field`
    // as a `MultipartError`, which `AppWebError`'s `#[from]` conversion maps
    // to its generic 500 branch — the same treatment `profile::upload_picture`
    // already gives an oversized picture upload (see AppWebError::Multipart);
    // not a regression introduced here, just the existing project-wide
    // convention for this error type.
    assert_eq!(response.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(document_count_for_tenant(&app, user).await, 0);
}

#[tokio::test]
async fn disallowed_content_type_is_rejected() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "badtype.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "badtype.docs@example.com").await;

    let response = common::post_multipart_parts_with_cookie(
        &app,
        "/documents",
        &cookie,
        &[MultipartPart::File {
            name: "file",
            filename: "malware.exe",
            content_type: "application/x-msdownload",
            bytes: b"not a real document",
        }],
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(document_count_for_tenant(&app, user).await, 0);
}

#[tokio::test]
async fn pdf_upload_is_ocr_eligible_not_skipped() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "pdf.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::post_multipart_parts_with_cookie(
        &app,
        "/documents",
        &cookie,
        &[MultipartPart::File {
            name: "file",
            filename: "statement.pdf",
            content_type: "application/pdf",
            bytes: b"%PDF-1.4 fake but good enough, content is never parsed",
        }],
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let id = common::document_id_from_location(&common::location(&response).unwrap());
    let row = find_document(&app, id).await.expect("row should exist");
    assert!(
        row.ocr_status == "pending" || row.ocr_status == "processing" || row.ocr_status == "done" || row.ocr_status == "failed",
        "PDF uploads are OCR-eligible now, never inserted as 'skipped': got {}",
        row.ocr_status
    );
}

#[tokio::test]
async fn corrupt_pdf_fails_ocr_gracefully_instead_of_panicking() {
    if !common::command_on_path("pdftoppm") {
        eprintln!("skipping corrupt_pdf_fails_ocr_gracefully_instead_of_panicking: `pdftoppm` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "corruptpdf.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::post_multipart_parts_with_cookie(
        &app,
        "/documents",
        &cookie,
        &[MultipartPart::File {
            name: "file",
            filename: "not_really_a_pdf.pdf",
            content_type: "application/pdf",
            bytes: b"%PDF-1.4 this is not a well-formed pdf and pdftoppm cannot rasterize it",
        }],
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let id = common::document_id_from_location(&common::location(&response).unwrap());

    let outcome = common::wait_for_ocr_outcome(&app, id, std::time::Duration::from_secs(15)).await;
    assert_eq!(outcome.status, "failed", "an unrasterizable pdf should end up failed, not stuck or panicking");
    assert!(outcome.error.is_some(), "a failed PDF OCR pass should record ocr_error");
}

#[tokio::test]
async fn invalid_metadata_field_is_rejected() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "badmeta.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "badmeta.docs@example.com").await;

    let too_long_title = "x".repeat(500);
    let response = common::post_multipart_parts_with_cookie(
        &app,
        "/documents",
        &cookie,
        &[
            MultipartPart::Text { name: "title", value: &too_long_title },
            MultipartPart::File {
                name: "file",
                filename: "doc.png",
                content_type: "image/png",
                bytes: b"irrelevant",
            },
        ],
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(document_count_for_tenant(&app, user).await, 0);
}

#[tokio::test]
async fn metadata_field_after_file_is_rejected() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "outoforder.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "outoforder.docs@example.com").await;

    let response = common::post_multipart_parts_with_cookie(
        &app,
        "/documents",
        &cookie,
        &[
            MultipartPart::File {
                name: "file",
                filename: "doc.png",
                content_type: "image/png",
                bytes: b"irrelevant",
            },
            MultipartPart::Text { name: "title", value: "Too late" },
        ],
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(document_count_for_tenant(&app, user).await, 0);
}

#[tokio::test]
async fn uploads_are_isolated_by_tenant() {
    let app = common::test_state().await;

    let login_a = common::signup_and_login(&app, "tenant.upload.a@example.com", "documentspassword").await;
    let cookie_a = common::session_cookie(&login_a).expect("login should set a session cookie");

    common::post_multipart_parts_with_cookie(
        &app,
        "/documents",
        &cookie_a,
        &[MultipartPart::File {
            name: "file",
            filename: "tenant_a_only.pdf",
            content_type: "application/pdf",
            bytes: b"fake pdf",
        }],
    )
    .await;

    let login_b = common::signup_and_login(&app, "tenant.upload.b@example.com", "documentspassword").await;
    let cookie_b = common::session_cookie(&login_b).expect("login should set a session cookie");

    let response_b = common::get_with_cookie(&app, "/documents", &cookie_b).await;
    let body_b = common::body_string(response_b).await;
    assert!(!body_b.contains("tenant_a_only.pdf"));

    let response_a = common::get_with_cookie(&app, "/documents", &cookie_a).await;
    let body_a = common::body_string(response_a).await;
    assert!(body_a.contains("tenant_a_only.pdf"));
}

#[tokio::test]
async fn get_documents_new_reaches_the_new_document_form_not_show() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "newform.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::get_with_cookie(&app, "/documents/new", &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("Add a document"));
    assert!(body.contains("action=\"/documents\""));
}

#[tokio::test]
async fn image_upload_is_actually_ocrd_by_tesseract() {
    if !common::command_on_path("tesseract") {
        eprintln!("skipping image_upload_is_actually_ocrd_by_tesseract: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let outcome =
        common::upload_and_wait_for_ocr(&app, "realocr.docs@example.com", "tests/fixtures/ocr_sample.png", "ocr_sample.png", "image/png")
            .await
            .outcome;
    assert_eq!(outcome.status, "done", "ocr should complete within the timeout");
    assert!(
        outcome.text.as_deref().unwrap_or("").contains("DOCUFLOW OCR SAMPLE"),
        "expected extracted text to contain the fixture's text, got: {:?}",
        outcome.text
    );
}

#[tokio::test]
async fn cyrillic_image_upload_is_correctly_ocrd() {
    if !common::command_on_path("tesseract") {
        eprintln!("skipping cyrillic_image_upload_is_correctly_ocrd: `tesseract` not found on PATH");
        return;
    }
    if !common::tesseract_has_lang("rus") {
        eprintln!("skipping cyrillic_image_upload_is_correctly_ocrd: tesseract-ocr-rus (rus.traineddata) not installed");
        return;
    }

    let app = common::test_state().await;
    let outcome = common::upload_and_wait_for_ocr(
        &app,
        "cyrillicocr.docs@example.com",
        "tests/fixtures/cyrillic_sample.png",
        "cyrillic_sample.png",
        "image/png",
    )
    .await
    .outcome;
    assert_eq!(outcome.status, "done", "ocr should complete within the timeout");
    assert!(
        outcome.text.as_deref().unwrap_or("").contains("ДОКУФЛОВ"),
        "expected extracted text to contain the fixture's Cyrillic text, got: {:?}",
        outcome.text
    );
}

#[tokio::test]
async fn pdf_upload_is_rasterized_and_ocrd_by_tesseract() {
    if !common::command_on_path("tesseract") || !common::command_on_path("pdftoppm") {
        eprintln!("skipping pdf_upload_is_rasterized_and_ocrd_by_tesseract: `tesseract` and/or `pdftoppm` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let outcome = common::upload_and_wait_for_ocr(
        &app,
        "realpdfocr.docs@example.com",
        "tests/fixtures/ocr_sample.pdf",
        "ocr_sample.pdf",
        "application/pdf",
    )
    .await
    .outcome;
    assert_eq!(outcome.status, "done", "pdf ocr should complete within the timeout, got text: {:?}", outcome.text);
    assert!(
        outcome.text.as_deref().unwrap_or("").contains("DOCUFLOW OCR SAMPLE PDF"),
        "expected extracted text to contain the fixture's text, got: {:?}",
        outcome.text
    );
}
