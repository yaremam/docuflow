mod common;

/// Every test below uploads a real image and waits for the real OCR pass
/// (which is also where thumbnail generation happens) — soft-skip on a box
/// without `tesseract`, same convention as `documents_upload.rs`.
fn tesseract_available() -> bool {
    common::command_on_path("tesseract")
}

async fn thumbnail_status(app: &common::TestApp, id: uuid::Uuid) -> Option<String> {
    sqlx::query_scalar!("select thumbnail_status from documents where id = $1", id).fetch_one(&app.state.pool).await.unwrap()
}

async fn blob_key(app: &common::TestApp, id: uuid::Uuid) -> String {
    sqlx::query_scalar!("select blob_key from documents where id = $1", id).fetch_one(&app.state.pool).await.unwrap()
}

#[tokio::test]
async fn an_uploaded_image_gets_a_generated_thumbnail() {
    if !tesseract_available() {
        eprintln!("skipping an_uploaded_image_gets_a_generated_thumbnail: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "thumbimage.docs@example.com",
        "tests/fixtures/english_sample.png",
        "english_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");
    assert_eq!(thumbnail_status(&app, uploaded.id).await.as_deref(), Some("done"));
}

#[tokio::test]
async fn a_pdf_upload_gets_a_thumbnail_from_its_first_page() {
    if !tesseract_available() || !common::command_on_path("pdftoppm") {
        eprintln!("skipping a_pdf_upload_gets_a_thumbnail_from_its_first_page: `tesseract`/`pdftoppm` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded =
        common::upload_and_wait_for_ocr(&app, "thumbpdf.docs@example.com", "tests/fixtures/ocr_sample.pdf", "ocr_sample.pdf", "application/pdf")
            .await;
    assert_eq!(uploaded.outcome.status, "done");
    assert_eq!(thumbnail_status(&app, uploaded.id).await.as_deref(), Some("done"));
}

#[tokio::test]
async fn the_dashboard_row_uses_the_generated_thumbnail_url_once_it_is_ready() {
    if !tesseract_available() {
        eprintln!("skipping the_dashboard_row_uses_the_generated_thumbnail_url_once_it_is_ready: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "thumbdashboard.docs@example.com",
        "tests/fixtures/english_sample.png",
        "english_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");
    assert_eq!(thumbnail_status(&app, uploaded.id).await.as_deref(), Some("done"));

    let key = blob_key(&app, uploaded.id).await;
    let response = common::get_with_cookie(&app, "/documents", &uploaded.cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains(&format!("{key}-thumb")), "expected the dashboard row's <img> to point at the generated thumbnail key, got: {body}");
}
