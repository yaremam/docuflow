mod common;

/// Every real-OCR test below needs `tesseract` (and, for the Cyrillic-script
/// case, the `rus` trained-data pack) — soft-skip on a box missing either,
/// same convention as `documents_upload.rs`/`documents_date_suggestion.rs`.
fn tesseract_available() -> bool {
    common::command_on_path("tesseract")
}

async fn user_id(app: &common::TestApp, email: &str) -> uuid::Uuid {
    sqlx::query_scalar!("select id from users where email = $1", email)
        .fetch_one(&app.state.pool)
        .await
        .unwrap()
}

/// Seeds a document row directly (no real blob/OCR) with a given
/// `language` already set — for tests that only need to observe the
/// "never overwrite" guarantee or form rendering, matching
/// `documents_show.rs`'s `seed_document` convention.
async fn seed_document_with_language(pool: &sqlx::PgPool, user_id: uuid::Uuid, filename: &str, language: Option<&str>) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status, language)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, '{}', 'done', $5)",
        id,
        user_id,
        filename,
        blob_key,
        language,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn english_text_is_detected_and_shown_after_ocr() {
    if !tesseract_available() {
        eprintln!("skipping english_text_is_detected_and_shown_after_ocr: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "englishlang.docs@example.com",
        "tests/fixtures/english_sample.png",
        "english_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");

    let row = sqlx::query!("select language from documents where id = $1", uploaded.id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.language.as_deref(), Some("en"));

    let response = common::get_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains(r#"<option value="en" selected>"#), "expected English pre-selected in the language field, got: {body}");
}

#[tokio::test]
async fn cyrillic_script_text_is_detected_and_shown_after_ocr() {
    if !tesseract_available() {
        eprintln!("skipping cyrillic_script_text_is_detected_and_shown_after_ocr: `tesseract` not found on PATH");
        return;
    }
    if !common::tesseract_has_lang("rus") {
        eprintln!("skipping cyrillic_script_text_is_detected_and_shown_after_ocr: tesseract-ocr-rus (rus.traineddata) not installed");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "cyrlang.docs@example.com",
        "tests/fixtures/ukrainian_sample.png",
        "ukrainian_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");

    let row = sqlx::query!("select language from documents where id = $1", uploaded.id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.language.as_deref(), Some("cyr"));

    let response = common::get_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains(r#"<option value="cyr" selected>"#), "expected Cyrillic pre-selected in the language field, got: {body}");
}

#[tokio::test]
async fn manually_set_language_is_never_overwritten_by_a_later_ocr_pass() {
    if !tesseract_available() {
        eprintln!("skipping manually_set_language_is_never_overwritten_by_a_later_ocr_pass: `tesseract` not found on PATH");
        return;
    }

    let app = common::test_state().await;
    let uploaded = common::upload_and_wait_for_ocr(
        &app,
        "manuallang.docs@example.com",
        "tests/fixtures/english_sample.png",
        "english_sample.png",
        "image/png",
    )
    .await;
    assert_eq!(uploaded.outcome.status, "done");
    // The fixture is genuinely English, so auto-detection would set "en" —
    // manually override it to prove a real user choice survives a later pass.
    let form_body = "title=Manually+set&tags=&date_issued=&language=cyr";
    let response = common::post_form_with_cookie(&app, &format!("/documents/{}", uploaded.id), &uploaded.cookie, form_body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let row = sqlx::query!("select language from documents where id = $1", uploaded.id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.language.as_deref(), Some("cyr"), "manual override should have taken effect");

    // Reprocessing (feature 013) re-runs run_ocr end to end, which would
    // auto-detect "en" again if the write weren't guarded.
    let response = common::post_with_cookie(&app, &format!("/documents/{}/reprocess_ocr", uploaded.id), &uploaded.cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let _ = common::wait_for_ocr_outcome(&app, uploaded.id, std::time::Duration::from_secs(10)).await;

    let row = sqlx::query!("select language from documents where id = $1", uploaded.id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.language.as_deref(), Some("cyr"), "a later OCR pass must never overwrite a manually-set language");
}

#[tokio::test]
async fn saving_metadata_never_blocks_when_language_is_blank() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "blanklang.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "blanklang.docs@example.com").await;
    let doc_id = seed_document_with_language(&app.state.pool, user, "untitled.pdf", None).await;

    let form_body = "title=Still+untitled&tags=&date_issued=&language=";
    let response = common::post_form_with_cookie(&app, &format!("/documents/{doc_id}"), &cookie, form_body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER, "saving with a blank language must not be rejected");

    let row = sqlx::query!("select language from documents where id = $1", doc_id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.language, None);
}

#[tokio::test]
async fn an_out_of_set_language_value_is_rejected() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "badlang.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "badlang.docs@example.com").await;
    let doc_id = seed_document_with_language(&app.state.pool, user, "untitled.pdf", None).await;

    let form_body = "title=x&tags=&date_issued=&language=fr";
    let response = common::post_form_with_cookie(&app, &format!("/documents/{doc_id}"), &cookie, form_body).await;
    // Axum's `Form` extractor rejects a failed deserialize with 422 (well-formed
    // request, semantically invalid), matching signup_page.rs's precedent.
    assert_eq!(response.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_upload_form_has_no_language_field() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "uploadformlang.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::get_with_cookie(&app, "/documents/new", &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains(r#"name="language""#), "the upload form shouldn't ask for a language up front, got: {body}");
}
