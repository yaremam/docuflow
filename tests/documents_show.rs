mod common;

async fn user_id(app: &common::TestApp, email: &str) -> uuid::Uuid {
    sqlx::query_scalar!("select id from users where email = $1", email)
        .fetch_one(&app.state.pool)
        .await
        .unwrap()
}

async fn seed_document(pool: &sqlx::PgPool, user_id: uuid::Uuid, filename: &str, ocr_text: Option<&str>) -> uuid::Uuid {
    seed_document_with_content_type(pool, user_id, filename, "application/pdf", ocr_text).await
}

async fn seed_document_with_content_type(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    filename: &str,
    content_type: &str,
    ocr_text: Option<&str>,
) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status, ocr_text)
         values ($1, $2, $2, $3, $4, 100, $5, '{}', $6, $7)",
        id,
        user_id,
        filename,
        content_type,
        blob_key,
        if ocr_text.is_some() { "done" } else { "pending" },
        ocr_text,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn view_renders_metadata_and_extracted_text() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "show.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "show.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, user, "contract.pdf", Some("This agreement is made between...")).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}"), &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("contract.pdf"));
    assert!(body.contains("This agreement is made between..."));
}

#[tokio::test]
async fn view_with_a_matching_q_highlights_every_occurrence_in_the_ocr_text() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "highlight.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "highlight.docs@example.com").await;
    let doc_id = seed_document(
        &app.state.pool,
        user,
        "electric_statement.pdf",
        Some("This statement covers your Electric Company account. Electric service charges total $142."),
    )
    .await;

    let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}?q=electric+company"), &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("<mark>Electric</mark>"), "expected every occurrence marked, got: {body}");
    assert!(body.contains("<mark>Company</mark>"), "expected the matched word marked, got: {body}");
    assert!(body.contains("Highlighting matches"), "expected an indicator stating what's highlighted, got: {body}");
}

#[tokio::test]
async fn view_with_no_q_renders_ocr_text_unchanged_with_no_marks() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "nohighlight.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "nohighlight.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, user, "plain.pdf", Some("Electric Company account statement")).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}"), &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("Electric Company account statement"), "expected the plain OCR text, got: {body}");
    assert!(!body.contains("<mark>"), "no q means no highlighting, got: {body}");
    assert!(!body.contains("Highlighting matches"), "no q means no highlighting indicator, got: {body}");
}

#[tokio::test]
async fn view_with_a_non_matching_q_shows_no_marks_or_indicator() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "missnomatch.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "missnomatch.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, user, "unrelated.pdf", Some("Acme Water Utility statement")).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}?q=electric+company"), &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("Acme Water Utility statement"), "expected the plain OCR text, got: {body}");
    assert!(!body.contains("<mark>"), "q that doesn't appear in this doc shouldn't mark anything, got: {body}");
    assert!(!body.contains("Highlighting matches"), "no match means no misleading indicator, got: {body}");
}

#[tokio::test]
async fn view_renders_an_image_preview_with_a_presigned_url() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "imgpreview.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "imgpreview.docs@example.com").await;
    let doc_id = seed_document_with_content_type(&app.state.pool, user, "bill.jpg", "image/jpeg", Some("text")).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}"), &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("<img"), "expected an <img> preview for an image document, got: {body}");
    assert!(!body.contains("<embed"), "an image document shouldn't render a PDF <embed>");
    assert!(
        body.contains("X-Amz-Signature"),
        "expected the image preview to use a presigned blob URL, got: {body}"
    );
}

#[tokio::test]
async fn view_renders_a_pdf_embed_with_a_presigned_url() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "pdfpreview.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "pdfpreview.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, user, "policy.pdf", Some("text")).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}"), &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("<embed"), "expected a PDF <embed> preview, got: {body}");
    assert!(
        body.contains("X-Amz-Signature"),
        "expected the PDF embed to use a presigned blob URL, got: {body}"
    );
}

#[tokio::test]
async fn view_includes_a_delete_link() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "showdelete.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "showdelete.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, user, "contract.pdf", Some("text")).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}"), &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(
        body.contains(&format!("href=\"/documents/{doc_id}/delete\"")),
        "expected a delete link on the detail page, got: {body}"
    );
}

#[tokio::test]
async fn view_shows_a_placeholder_when_text_is_not_yet_extracted() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "pending.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "pending.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, user, "pending.pdf", None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}"), &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("Still processing"));
}

#[tokio::test]
async fn viewing_another_tenants_document_is_not_found() {
    let app = common::test_state().await;

    let owner_login = common::signup_and_login(&app, "owner.docs@example.com", "documentspassword").await;
    let owner = user_id(&app, "owner.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, owner, "owners_document.pdf", Some("secret text")).await;
    drop(owner_login);

    let intruder_login = common::signup_and_login(&app, "intruder.docs@example.com", "documentspassword").await;
    let intruder_cookie = common::session_cookie(&intruder_login).expect("login should set a session cookie");

    let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}"), &intruder_cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn editing_metadata_persists_and_redirects_with_saved_flag() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "edit.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "edit.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, user, "renewal.pdf", Some("text")).await;

    let response = common::post_form_with_cookie(
        &app,
        &format!("/documents/{doc_id}"),
        &cookie,
        "title=Auto+policy+renewal&tags=insurance%2C+auto&date_issued=2026-01-03",
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        common::location(&response),
        Some(format!("/documents/{doc_id}?saved=true"))
    );

    let row = sqlx::query!("select title, tags, date_issued from documents where id = $1", doc_id)
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
    assert_eq!(row.title.as_deref(), Some("Auto policy renewal"));
    assert_eq!(row.tags, vec!["insurance".to_string(), "auto".to_string()]);
    assert_eq!(
        row.date_issued,
        Some(time::Date::from_calendar_date(2026, time::Month::January, 3).unwrap())
    );
}

#[tokio::test]
async fn editing_another_tenants_document_is_not_found() {
    let app = common::test_state().await;

    let owner_login = common::signup_and_login(&app, "edit.owner@example.com", "documentspassword").await;
    let owner = user_id(&app, "edit.owner@example.com").await;
    let doc_id = seed_document(&app.state.pool, owner, "not_yours.pdf", Some("text")).await;
    drop(owner_login);

    let intruder_login = common::signup_and_login(&app, "edit.intruder@example.com", "documentspassword").await;
    let intruder_cookie = common::session_cookie(&intruder_login).expect("login should set a session cookie");

    let response = common::post_form_with_cookie(
        &app,
        &format!("/documents/{doc_id}"),
        &intruder_cookie,
        "title=Hijacked&tags=&date_issued=",
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}
