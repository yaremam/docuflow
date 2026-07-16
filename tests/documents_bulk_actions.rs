mod common;

use common::user_id;

#[allow(clippy::too_many_arguments)]
async fn seed_document(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    filename: &str,
    tags: &[&str],
    ocr_status: &str,
) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let tags: Vec<String> = tags.iter().map(|tag| tag.to_string()).collect();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents
            (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, $5, $6)",
        id,
        user_id,
        filename,
        blob_key,
        &tags,
        ocr_status,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn document_exists(pool: &sqlx::PgPool, id: uuid::Uuid) -> bool {
    sqlx::query_scalar!("select exists(select 1 from documents where id = $1)", id).fetch_one(pool).await.unwrap().unwrap_or(false)
}

async fn tags_of(pool: &sqlx::PgPool, id: uuid::Uuid) -> Vec<String> {
    sqlx::query_scalar!("select tags from documents where id = $1", id).fetch_one(pool).await.unwrap()
}

async fn ocr_status_of(pool: &sqlx::PgPool, id: uuid::Uuid) -> String {
    sqlx::query_scalar!("select ocr_status from documents where id = $1", id).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn bulk_delete_confirm_page_lists_every_selected_document() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "bulkconfirm.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "bulkconfirm.docs@example.com").await;

    let a = seed_document(&app.state.pool, user, "alpha.pdf", &[], "done").await;
    let b = seed_document(&app.state.pool, user, "beta.pdf", &[], "done").await;

    let body = format!("doc_ids={a}&doc_ids={b}&return_to=");
    let response = common::post_form_with_cookie(&app, "/documents/bulk/delete", &cookie, &body).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("alpha.pdf"), "expected the confirm page to list alpha.pdf, got: {body}");
    assert!(body.contains("beta.pdf"), "expected the confirm page to list beta.pdf, got: {body}");
    assert!(body.contains(&format!("value=\"{a}\"")), "expected a hidden doc_ids field for {a}, got: {body}");
}

#[tokio::test]
async fn bulk_delete_confirm_execute_deletes_all_selected_and_redirects_to_return_to() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "bulkdelete.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "bulkdelete.docs@example.com").await;

    let a = seed_document(&app.state.pool, user, "alpha.pdf", &[], "done").await;
    let b = seed_document(&app.state.pool, user, "beta.pdf", &[], "done").await;
    let untouched = seed_document(&app.state.pool, user, "gamma.pdf", &[], "done").await;

    let body = format!("doc_ids={a}&doc_ids={b}&return_to=tags%3Dinsurance");
    let response = common::post_form_with_cookie(&app, "/documents/bulk/delete/confirm", &cookie, &body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let location = common::location(&response).unwrap();
    assert!(location.starts_with("/documents?tags=insurance"), "expected the return_to filter state preserved, got: {location}");
    assert!(location.contains("deleted=true"), "expected a deleted=true flash flag, got: {location}");

    assert!(!document_exists(&app.state.pool, a).await);
    assert!(!document_exists(&app.state.pool, b).await);
    assert!(document_exists(&app.state.pool, untouched).await, "a document not in the selection must survive");
}

#[tokio::test]
async fn bulk_delete_is_tenant_scoped() {
    let app = common::test_state().await;
    let login_a = common::signup_and_login(&app, "bulkdeleteA.docs@example.com", "documentspassword").await;
    let cookie_a = common::session_cookie(&login_a).expect("login should set a session cookie");
    let user_a = user_id(&app, "bulkdeleteA.docs@example.com").await;

    common::signup_and_login(&app, "bulkdeleteB.docs@example.com", "documentspassword").await;
    let user_b = user_id(&app, "bulkdeleteB.docs@example.com").await;

    let mine = seed_document(&app.state.pool, user_a, "mine.pdf", &[], "done").await;
    let theirs = seed_document(&app.state.pool, user_b, "theirs.pdf", &[], "done").await;

    let body = format!("doc_ids={mine}&doc_ids={theirs}&return_to=");
    let response = common::post_form_with_cookie(&app, "/documents/bulk/delete/confirm", &cookie_a, &body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    assert!(!document_exists(&app.state.pool, mine).await);
    assert!(document_exists(&app.state.pool, theirs).await, "another tenant's document must never be deleted by someone else's bulk action");
}

#[tokio::test]
async fn bulk_tag_adds_the_tag_to_every_selected_document_without_duplicating_existing_tags() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "bulktag.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "bulktag.docs@example.com").await;

    let a = seed_document(&app.state.pool, user, "alpha.pdf", &["utilities"], "done").await;
    let b = seed_document(&app.state.pool, user, "beta.pdf", &["insurance"], "done").await;

    let body = format!("doc_ids={a}&doc_ids={b}&tag=insurance&return_to=");
    let response = common::post_form_with_cookie(&app, "/documents/bulk/tag", &cookie, &body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let a_tags = tags_of(&app.state.pool, a).await;
    assert!(a_tags.contains(&"utilities".to_string()) && a_tags.contains(&"insurance".to_string()), "expected both tags on alpha, got: {a_tags:?}");
    let b_tags = tags_of(&app.state.pool, b).await;
    assert_eq!(b_tags, vec!["insurance".to_string()], "tagging with an already-present tag must not duplicate it, got: {b_tags:?}");
}

#[tokio::test]
async fn bulk_tag_preserves_the_order_of_a_documents_existing_tags() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "bulktagorder.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "bulktagorder.docs@example.com").await;

    let doc = seed_document(&app.state.pool, user, "alpha.pdf", &["zebra", "apple", "mango"], "done").await;

    let body = format!("doc_ids={doc}&tag=urgent&return_to=");
    let response = common::post_form_with_cookie(&app, "/documents/bulk/tag", &cookie, &body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let tags = tags_of(&app.state.pool, doc).await;
    assert_eq!(
        tags,
        vec!["zebra".to_string(), "apple".to_string(), "mango".to_string(), "urgent".to_string()],
        "existing tags must keep their original order, with the new tag appended after them, got: {tags:?}"
    );
}

#[tokio::test]
async fn bulk_reprocess_ocr_only_touches_eligible_documents() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "bulkreprocess.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "bulkreprocess.docs@example.com").await;

    let eligible = seed_document(&app.state.pool, user, "done.pdf", &[], "done").await;
    let in_flight = seed_document(&app.state.pool, user, "processing.pdf", &[], "processing").await;

    let body = format!("doc_ids={eligible}&doc_ids={in_flight}&return_to=");
    let response = common::post_form_with_cookie(&app, "/documents/bulk/reprocess_ocr", &cookie, &body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    assert_eq!(ocr_status_of(&app.state.pool, eligible).await, "pending", "an eligible (done) document should be requeued");
    assert_eq!(ocr_status_of(&app.state.pool, in_flight).await, "processing", "an already in-flight document must not be re-queued");
}

#[tokio::test]
async fn bulk_actions_are_reachable_only_via_the_single_outer_form_not_a_nested_one() {
    // Same regression class as the date/doc_type suggestion nested-form
    // bugs: the bulk toolbar's buttons must live inside the page's one
    // GET filters `<form>` via formaction/formmethod, never a second
    // nested `<form>`.
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "bulknoform.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "bulknoform.docs@example.com").await;
    seed_document(&app.state.pool, user, "alpha.pdf", &[], "done").await;

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;

    let outer_form_tag_start = body.find("<form method=\"get\" action=\"/documents\"").expect("expected the outer filters form");
    let outer_form_tag_end = outer_form_tag_start + body[outer_form_tag_start..].find('>').expect("expected the outer form's opening tag to close") + 1;
    let bulk_delete_button = body.find("formaction=\"/documents/bulk/delete\"").expect("expected a bulk delete button");
    assert!(outer_form_tag_end < bulk_delete_button, "the outer form should open before the bulk delete button");

    let between = &body[outer_form_tag_end..bulk_delete_button];
    assert!(!between.contains("<form"), "the bulk toolbar must not be inside a nested <form>, got the region: {between}");
}
