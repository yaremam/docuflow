mod common;

/// Tests that need a real OCR pass to prove `reprocess_ocr` actually
/// overwrites derived OCR fields — soft-skip on a box without `tesseract`,
/// same convention as `documents_date_suggestion.rs`.
fn tesseract_available() -> bool {
    common::command_on_path("tesseract")
}

async fn user_id(app: &common::TestApp, email: &str) -> uuid::Uuid {
    sqlx::query_scalar!("select id from users where email = $1", email)
        .fetch_one(&app.state.pool)
        .await
        .unwrap()
}

/// Seeds a document row directly (no real blob) at a chosen `ocr_status` —
/// enough for tests that only need to observe status transitions/gating,
/// not a real OCR pass, matching `documents_show.rs`'s `seed_document`
/// convention.
async fn seed_document(pool: &sqlx::PgPool, user_id: uuid::Uuid, filename: &str, ocr_status: &str) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, '{}', $5)",
        id,
        user_id,
        filename,
        blob_key,
        ocr_status,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn reprocessing_overwrites_stale_ocr_text_and_suggested_date() {
    if !tesseract_available() {
        eprintln!("skipping reprocessing_overwrites_stale_ocr_text_and_suggested_date: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "reprocess.docs@example.com",
        "tests/fixtures/dated_sample.png",
        "dated_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");

    // Simulate a document whose OCR ran under an older, worse pipeline.
    sqlx::query!(
        "update documents set ocr_text = 'STALE TEXT FROM AN OLDER PIPELINE', ocr_suggested_date_issued = '2000-01-01' where id = $1",
        uploaded.id,
    )
    .execute(&app.state.pool)
    .await
    .unwrap();

    let response = common::post_with_cookie(&app, &format!("/documents/{}/reprocess_ocr", uploaded.id), &uploaded.cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response).unwrap(), format!("/documents/{}?reprocessing=true", uploaded.id));

    let outcome = common::wait_for_ocr_outcome(&app, uploaded.id, std::time::Duration::from_secs(10)).await;
    assert_eq!(outcome.status, "done");
    assert_ne!(
        outcome.text.as_deref(),
        Some("STALE TEXT FROM AN OLDER PIPELINE"),
        "reprocessing should overwrite stale ocr_text with a fresh OCR pass"
    );
    assert_eq!(
        outcome.suggested_date_issued,
        Some(time::Date::from_calendar_date(2024, time::Month::March, 15).unwrap()),
        "reprocessing should re-derive the suggested date from the fresh OCR text, not keep the stale one"
    );
}

#[tokio::test]
async fn reprocessing_a_skipped_document_runs_ocr_for_the_first_time() {
    if !tesseract_available() {
        eprintln!("skipping reprocessing_a_skipped_document_runs_ocr_for_the_first_time: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "reprocessskipped.docs@example.com",
        "tests/fixtures/ocr_sample.png",
        "ocr_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");

    // Simulate a document uploaded before its content type was OCR-eligible
    // (e.g. a PDF uploaded before feature 010) — never actually OCR'd.
    sqlx::query!("update documents set ocr_status = 'skipped', ocr_text = null where id = $1", uploaded.id)
        .execute(&app.state.pool)
        .await
        .unwrap();

    let response = common::post_with_cookie(&app, &format!("/documents/{}/reprocess_ocr", uploaded.id), &uploaded.cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let outcome = common::wait_for_ocr_outcome(&app, uploaded.id, std::time::Duration::from_secs(10)).await;
    assert_eq!(outcome.status, "done");
    assert!(
        outcome.text.is_some_and(|t| !t.trim().is_empty()),
        "a previously-skipped document should have real extracted text after reprocessing"
    );
}

#[tokio::test]
async fn reprocess_button_is_shown_for_done_failed_and_skipped_but_not_while_processing() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "reprocessbutton.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "reprocessbutton.docs@example.com").await;

    for status in ["done", "failed", "skipped"] {
        let doc_id = seed_document(&app.state.pool, user, &format!("{status}.pdf"), status).await;
        let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}"), &cookie).await;
        let body = common::body_string(response).await;
        assert!(body.contains("Reprocess OCR"), "expected a Reprocess OCR button for ocr_status = {status}, got: {body}");
    }

    let processing_id = seed_document(&app.state.pool, user, "processing.pdf", "processing").await;
    let response = common::get_with_cookie(&app, &format!("/documents/{processing_id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains("Reprocess OCR"), "a document mid-OCR shouldn't offer a reprocess button, got: {body}");
}

#[tokio::test]
async fn reprocessing_a_document_already_in_flight_is_a_no_op() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "reprocessnoop.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "reprocessnoop.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, user, "inflight.pdf", "processing").await;

    let response = common::post_with_cookie(&app, &format!("/documents/{doc_id}/reprocess_ocr"), &cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let row = sqlx::query!("select ocr_status from documents where id = $1", doc_id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(
        row.ocr_status, "processing",
        "a request while already processing should not queue a second job or reset the status"
    );
}

#[tokio::test]
async fn reprocessing_another_tenants_document_is_not_found() {
    let app = common::test_state().await;

    let owner_login = common::signup_and_login(&app, "reprocessowner.docs@example.com", "documentspassword").await;
    let owner = user_id(&app, "reprocessowner.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, owner, "not_yours.pdf", "done").await;
    drop(owner_login);

    let intruder_login = common::signup_and_login(&app, "reprocessintruder.docs@example.com", "documentspassword").await;
    let intruder_cookie = common::session_cookie(&intruder_login).expect("login should set a session cookie");

    let response = common::post_with_cookie(&app, &format!("/documents/{doc_id}/reprocess_ocr"), &intruder_cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reprocessing_a_nonexistent_document_is_not_found() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "reprocessmissing.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::post_with_cookie(&app, &format!("/documents/{}/reprocess_ocr", uuid::Uuid::new_v4()), &cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}
