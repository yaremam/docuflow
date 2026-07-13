mod common;

async fn user_id(app: &common::TestApp, email: &str) -> uuid::Uuid {
    sqlx::query_scalar!("select id from users where email = $1", email)
        .fetch_one(&app.state.pool)
        .await
        .unwrap()
}

async fn seed_document(pool: &sqlx::PgPool, user_id: uuid::Uuid, filename: &str, tags: &[&str]) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let tags: Vec<String> = tags.iter().map(|tag| tag.to_string()).collect();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, $5, 'done')",
        id,
        user_id,
        filename,
        blob_key,
        &tags,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

fn extract_href(body: &str, class: &str) -> Option<String> {
    let marker = format!("class=\"{class}\" href=\"");
    let start = body.find(&marker)? + marker.len();
    let end = body[start..].find('"')? + start;
    Some(body[start..end].replace("&#38;", "&").replace("&amp;", "&"))
}

#[tokio::test]
async fn saving_a_search_creates_a_collection_and_lists_it() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "savecol.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "savecol.docs@example.com").await;
    seed_document(&app.state.pool, user, "a.pdf", &["insurance"]).await;

    let response = common::post_form_with_cookie(
        &app,
        "/documents/collections",
        &cookie,
        "name=Medical+%26+insurance&query=tags%3Dinsurance",
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    // Askama's HTML auto-escaping renders `&` as the numeric entity `&#38;`, not `&amp;`.
    assert!(body.contains("Medical &#38; insurance"), "expected the saved collection name, got: {body}");
}

#[tokio::test]
async fn applying_a_collection_reapplies_its_saved_filters() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "applycol.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "applycol.docs@example.com").await;
    seed_document(&app.state.pool, user, "matches.pdf", &["insurance"]).await;
    seed_document(&app.state.pool, user, "not_matching.pdf", &["utilities"]).await;

    common::post_form_with_cookie(&app, "/documents/collections", &cookie, "name=Insurance&query=tags%3Dinsurance").await;

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    let href = extract_href(&body, "collection-link").expect("expected a collection link in the panel");
    assert!(href.starts_with("/documents?"), "expected a /documents link, got: {href}");

    let response = common::get_with_cookie(&app, &href, &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("matches.pdf"));
    assert!(!body.contains("not_matching.pdf"));
}

#[tokio::test]
async fn collection_shows_a_live_document_count() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "countcol.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "countcol.docs@example.com").await;
    seed_document(&app.state.pool, user, "a.pdf", &["insurance"]).await;
    seed_document(&app.state.pool, user, "b.pdf", &["insurance"]).await;
    seed_document(&app.state.pool, user, "c.pdf", &["utilities"]).await;

    common::post_form_with_cookie(&app, "/documents/collections", &cookie, "name=Insurance&query=tags%3Dinsurance").await;

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("<span class=\"collection-count\">2</span>"), "expected a live count of 2, got: {body}");

    // Adding a third matching document should bump the count on next render — proving
    // it's computed live, not frozen at save time.
    seed_document(&app.state.pool, user, "d.pdf", &["insurance"]).await;
    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("<span class=\"collection-count\">3</span>"), "expected the live count to update to 3, got: {body}");
}

#[tokio::test]
async fn saving_with_an_empty_query_is_rejected() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "emptyquery.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::post_form_with_cookie(&app, "/documents/collections", &cookie, "name=Nothing&query=").await;
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);

    let count = sqlx::query_scalar!("select count(*) from smart_collections").fetch_one(&app.state.pool).await.unwrap().unwrap_or(0);
    assert_eq!(count, 0, "no collection should have been created for an empty (no-op) query");
}

#[tokio::test]
async fn saving_with_an_empty_name_is_rejected() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "emptyname.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::post_form_with_cookie(&app, "/documents/collections", &cookie, "name=&query=tags%3Dinsurance").await;
    assert_eq!(response.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn deleting_a_collection_removes_it_without_a_confirmation_step() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "deletecol.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    common::post_form_with_cookie(&app, "/documents/collections", &cookie, "name=Temp&query=tags%3Dinsurance").await;
    let id: uuid::Uuid = sqlx::query_scalar!("select id from smart_collections where name = 'Temp'").fetch_one(&app.state.pool).await.unwrap();

    let response = common::post_with_cookie(&app, &format!("/documents/collections/{id}/delete"), &cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let remaining = sqlx::query_scalar!("select count(*) from smart_collections where id = $1", id).fetch_one(&app.state.pool).await.unwrap().unwrap_or(0);
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn collections_are_tenant_scoped() {
    let app = common::test_state().await;
    let login_a = common::signup_and_login(&app, "coltenant.a@example.com", "documentspassword").await;
    let cookie_a = common::session_cookie(&login_a).expect("login should set a session cookie");
    let login_b = common::signup_and_login(&app, "coltenant.b@example.com", "documentspassword").await;
    let cookie_b = common::session_cookie(&login_b).expect("login should set a session cookie");

    common::post_form_with_cookie(&app, "/documents/collections", &cookie_a, "name=TenantA+Only&query=tags%3Dinsurance").await;
    let id: uuid::Uuid = sqlx::query_scalar!("select id from smart_collections where name = 'TenantA Only'").fetch_one(&app.state.pool).await.unwrap();

    let response_b = common::get_with_cookie(&app, "/documents", &cookie_b).await;
    let body_b = common::body_string(response_b).await;
    assert!(!body_b.contains("TenantA Only"), "tenant B should never see tenant A's collection, got: {body_b}");

    let delete_response = common::post_with_cookie(&app, &format!("/documents/collections/{id}/delete"), &cookie_b).await;
    assert_eq!(delete_response.status(), axum::http::StatusCode::NOT_FOUND, "tenant B should not be able to delete tenant A's collection");

    let still_there = sqlx::query_scalar!("select count(*) from smart_collections where id = $1", id).fetch_one(&app.state.pool).await.unwrap().unwrap_or(0);
    assert_eq!(still_there, 1, "tenant A's collection should be untouched");
}

#[tokio::test]
async fn save_this_search_control_only_shown_when_a_filter_is_active() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "savecontrol.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "savecontrol.docs@example.com").await;
    seed_document(&app.state.pool, user, "a.pdf", &["insurance"]).await;

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains("save-search"), "the bare unfiltered view shouldn't offer to save a search, got: {body}");

    let response = common::get_with_cookie(&app, "/documents?tags=insurance", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("save-search"), "an active filter should offer to save the search, got: {body}");
}

#[tokio::test]
async fn collections_panel_is_absent_when_none_are_saved() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "nocollections.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "nocollections.docs@example.com").await;
    seed_document(&app.state.pool, user, "a.pdf", &["insurance"]).await;

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains("My collections"), "the collections panel shouldn't render with zero saved collections, got: {body}");
}

#[tokio::test]
async fn renaming_a_collection_updates_only_its_name() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "renamecol.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "renamecol.docs@example.com").await;
    // Without at least one document, the page renders the true "no documents
    // yet" empty state, which never shows the filters/collections panel.
    seed_document(&app.state.pool, user, "a.pdf", &["insurance"]).await;

    common::post_form_with_cookie(&app, "/documents/collections", &cookie, "name=Old+name&query=tags%3Dinsurance").await;
    let id: uuid::Uuid = sqlx::query_scalar!("select id from smart_collections where name = 'Old name'").fetch_one(&app.state.pool).await.unwrap();
    let original_created_at: time::OffsetDateTime =
        sqlx::query_scalar!("select created_at from smart_collections where id = $1", id).fetch_one(&app.state.pool).await.unwrap();

    let response = common::post_form_with_cookie(&app, &format!("/documents/collections/{id}/rename"), &cookie, "name=New+name").await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let row = sqlx::query!("select name, query, created_at from smart_collections where id = $1", id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.name, "New name");
    assert_eq!(row.query, "tags=insurance", "renaming must not touch the saved filter query");
    assert_eq!(row.created_at, original_created_at, "renaming must not change the row's created_at / list position");

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("New name"), "expected the renamed collection, got: {body}");
    assert!(!body.contains("Old name"), "expected the old name to be gone, got: {body}");
}

#[tokio::test]
async fn renaming_with_an_empty_name_is_rejected() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "renameempty.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    common::post_form_with_cookie(&app, "/documents/collections", &cookie, "name=Keep+me&query=tags%3Dinsurance").await;
    let id: uuid::Uuid = sqlx::query_scalar!("select id from smart_collections where name = 'Keep me'").fetch_one(&app.state.pool).await.unwrap();

    let response = common::post_form_with_cookie(&app, &format!("/documents/collections/{id}/rename"), &cookie, "name=").await;
    assert_eq!(response.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);

    let name = sqlx::query_scalar!("select name from smart_collections where id = $1", id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(name, "Keep me", "a rejected rename must leave the original name untouched");
}

#[tokio::test]
async fn renaming_another_tenants_collection_is_not_found() {
    let app = common::test_state().await;
    let login_a = common::signup_and_login(&app, "renametenant.a@example.com", "documentspassword").await;
    let cookie_a = common::session_cookie(&login_a).expect("login should set a session cookie");
    let login_b = common::signup_and_login(&app, "renametenant.b@example.com", "documentspassword").await;
    let cookie_b = common::session_cookie(&login_b).expect("login should set a session cookie");

    common::post_form_with_cookie(&app, "/documents/collections", &cookie_a, "name=TenantA&query=tags%3Dinsurance").await;
    let id: uuid::Uuid = sqlx::query_scalar!("select id from smart_collections where name = 'TenantA'").fetch_one(&app.state.pool).await.unwrap();

    let response = common::post_form_with_cookie(&app, &format!("/documents/collections/{id}/rename"), &cookie_b, "name=Hijacked").await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);

    let name = sqlx::query_scalar!("select name from smart_collections where id = $1", id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(name, "TenantA");
}
