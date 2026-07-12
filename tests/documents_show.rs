mod common;

async fn user_id(app: &common::TestApp, email: &str) -> uuid::Uuid {
    sqlx::query_scalar!("select id from users where email = $1", email)
        .fetch_one(&app.state.pool)
        .await
        .unwrap()
}

async fn seed_document(pool: &sqlx::PgPool, user_id: uuid::Uuid, filename: &str, ocr_text: Option<&str>) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status, ocr_text)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, '{}', $5, $6)",
        id,
        user_id,
        filename,
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
