mod common;

async fn user_id(app: &common::TestApp, email: &str) -> uuid::Uuid {
    sqlx::query_scalar!("select id from users where email = $1", email)
        .fetch_one(&app.state.pool)
        .await
        .unwrap()
}

async fn seed_document(pool: &sqlx::PgPool, user_id: uuid::Uuid, filename: &str) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, '{}', 'done')",
        id,
        user_id,
        filename,
        blob_key,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn confirm_page_shows_the_document_summary() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "confirm.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "confirm.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, user, "lease.pdf").await;

    let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}/delete"), &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("lease.pdf"));
    assert!(body.contains(&format!("action=\"/documents/{doc_id}/delete\"")));
}

#[tokio::test]
async fn confirm_page_for_another_tenants_document_is_not_found() {
    let app = common::test_state().await;

    let owner_login = common::signup_and_login(&app, "confirm.owner@example.com", "documentspassword").await;
    let owner = user_id(&app, "confirm.owner@example.com").await;
    let doc_id = seed_document(&app.state.pool, owner, "owners_lease.pdf").await;
    drop(owner_login);

    let intruder_login = common::signup_and_login(&app, "confirm.intruder@example.com", "documentspassword").await;
    let intruder_cookie = common::session_cookie(&intruder_login).expect("login should set a session cookie");

    let response = common::get_with_cookie(&app, &format!("/documents/{doc_id}/delete"), &intruder_cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn confirm_page_without_a_session_redirects_to_login() {
    let app = common::test_state().await;
    let response = common::get(&app, "/documents/00000000-0000-0000-0000-000000000000/delete").await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response), Some("/login".to_string()));
}

#[tokio::test]
async fn posting_delete_removes_the_document_and_redirects() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "delete.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "delete.docs@example.com").await;
    let doc_id = seed_document(&app.state.pool, user, "old_bill.pdf").await;

    let response = common::post_with_cookie(&app, &format!("/documents/{doc_id}/delete"), &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response), Some("/documents?deleted=true".to_string()));

    let row = sqlx::query!("select id from documents where id = $1", doc_id)
        .fetch_optional(&app.state.pool)
        .await
        .unwrap();
    assert!(row.is_none(), "the document row should be gone after delete");
}

#[tokio::test]
async fn posting_delete_for_another_tenants_document_is_not_found_and_leaves_it_intact() {
    let app = common::test_state().await;

    let owner_login = common::signup_and_login(&app, "delete.owner@example.com", "documentspassword").await;
    let owner = user_id(&app, "delete.owner@example.com").await;
    let doc_id = seed_document(&app.state.pool, owner, "not_yours.pdf").await;
    drop(owner_login);

    let intruder_login = common::signup_and_login(&app, "delete.intruder@example.com", "documentspassword").await;
    let intruder_cookie = common::session_cookie(&intruder_login).expect("login should set a session cookie");

    let response = common::post_with_cookie(&app, &format!("/documents/{doc_id}/delete"), &intruder_cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);

    let row = sqlx::query!("select id from documents where id = $1", doc_id)
        .fetch_optional(&app.state.pool)
        .await
        .unwrap();
    assert!(row.is_some(), "another tenant's document must not be deleted");
}

#[tokio::test]
async fn deleted_flag_shows_a_success_banner_on_the_list_page() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "deletedflag.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::get_with_cookie(&app, "/documents?deleted=true", &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("Deleted"));
}
