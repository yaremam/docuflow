mod common;

use common::user_id;

/// Seeds a document directly (bypassing upload/OCR, like
/// `documents_filters.rs`'s own `seed_document`) with an optional
/// suggested and/or confirmed doc_type already set — these tests exercise
/// the suggestion/accept web-layer logic, not the keyword extraction
/// itself (that's covered by `doc_type_extract`'s own unit tests), so
/// there's no need to depend on real `tesseract` being on PATH.
async fn seed_document_with_doc_type(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    filename: &str,
    doc_type: Option<&str>,
    ocr_suggested_doc_type: Option<&str>,
) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents
            (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status, doc_type, ocr_suggested_doc_type)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, '{}', 'done', $5, $6)",
        id,
        user_id,
        filename,
        blob_key,
        doc_type,
        ocr_suggested_doc_type,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn a_suggested_doc_type_shows_the_accept_action_when_doc_type_is_unset() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "doctypesuggest.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "doctypesuggest.docs@example.com").await;

    let id = seed_document_with_doc_type(&app.state.pool, user, "bill.pdf", None, Some("bill")).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("Use this"), "a fresh doc_type suggestion with no doc_type set should show the accept action, got: {body}");
    assert!(body.contains("Bill"), "the suggested type's label should be shown, got: {body}");
}

#[tokio::test]
async fn no_suggestion_box_when_no_doc_type_was_suggested() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "nodoctypesuggest.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "nodoctypesuggest.docs@example.com").await;

    let id = seed_document_with_doc_type(&app.state.pool, user, "plain.pdf", None, None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains("OCR suggests"), "no suggestion should render when none was found, got: {body}");
}

#[tokio::test]
async fn suggestion_is_hidden_once_doc_type_is_already_set() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "doctypealreadyset.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "doctypealreadyset.docs@example.com").await;

    let id = seed_document_with_doc_type(&app.state.pool, user, "bill.pdf", Some("insurance"), Some("bill")).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        !body.contains("OCR suggests"),
        "the suggestion should stop showing once doc_type has a value, even though ocr_suggested_doc_type is still set in the DB, got: {body}"
    );
}

#[tokio::test]
async fn accept_suggested_doc_type_copies_it_into_doc_type_and_redirects() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "acceptdoctype.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "acceptdoctype.docs@example.com").await;

    let id = seed_document_with_doc_type(&app.state.pool, user, "bill.pdf", None, Some("bill")).await;

    let response = common::post_with_cookie(&app, &format!("/documents/{id}/accept_suggested_doc_type"), &cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response).unwrap(), format!("/documents/{id}?saved=true"));

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains(r#"<option value="bill" selected>Bill</option>"#), "doc_type should now hold the accepted suggestion, got: {body}");
    assert!(!body.contains("OCR suggests"), "the suggestion box should disappear once accepted, got: {body}");
}

#[tokio::test]
async fn accept_suggested_doc_type_never_overwrites_an_already_set_doc_type() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "noacceptdoctype.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "noacceptdoctype.docs@example.com").await;

    let id = seed_document_with_doc_type(&app.state.pool, user, "bill.pdf", Some("insurance"), Some("bill")).await;

    let response = common::post_with_cookie(&app, &format!("/documents/{id}/accept_suggested_doc_type"), &cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        body.contains(r#"<option value="insurance" selected>Insurance</option>"#),
        "accepting a suggestion must never overwrite a doc_type the user already set, got: {body}"
    );
}

#[tokio::test]
async fn accept_suggested_doc_type_is_tenant_scoped() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "tenantA.doctypesuggest@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "tenantA.doctypesuggest@example.com").await;

    let id = seed_document_with_doc_type(&app.state.pool, user, "bill.pdf", None, Some("bill")).await;

    let other_login = common::signup_and_login(&app, "tenantB.doctypesuggest@example.com", "documentspassword").await;
    let other_cookie = common::session_cookie(&other_login).expect("login should set a session cookie");

    let response = common::post_with_cookie(&app, &format!("/documents/{id}/accept_suggested_doc_type"), &other_cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("Use this"), "another tenant's failed accept attempt must not consume the real owner's suggestion, got: {body}");
}

#[tokio::test]
async fn the_use_this_button_is_not_a_form_nested_inside_the_metadata_form() {
    // Same regression class as `documents_date_suggestion.rs`'s nested-form
    // test (a real bug found 2026-07-13): any suggestion box's accept
    // button must live inside the page's one metadata `<form>` via
    // `formaction`/`formmethod`, never as its own nested `<form>`.
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "nonestedformdoctype.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "nonestedformdoctype.docs@example.com").await;

    let id = seed_document_with_doc_type(&app.state.pool, user, "bill.pdf", None, Some("bill")).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("Use this"), "expected a doc_type suggestion to be showing, got: {body}");

    let metadata_form_start = body.find(&format!("action=\"/documents/{id}\"")).expect("expected the metadata form");
    let suggestion_button = body.rfind("Use this").expect("expected the suggestion button");
    assert!(metadata_form_start < suggestion_button, "the metadata form should open before the suggestion button");

    let between = &body[metadata_form_start..suggestion_button];
    assert!(!between.contains("<form"), "the doc_type suggestion button must not be inside a nested <form>, got the region: {between}");
    assert!(
        body.contains(&format!("formaction=\"/documents/{id}/accept_suggested_doc_type\"")),
        "expected the suggestion button to use a formaction override, got: {body}"
    );
}

#[tokio::test]
async fn updating_metadata_form_with_a_doc_type_selection_persists_it() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "doctypeupdate.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "doctypeupdate.docs@example.com").await;

    let id = seed_document_with_doc_type(&app.state.pool, user, "plain.pdf", None, None).await;

    let form_body = "title=A+contract&tags=&date_issued=&doc_type=contract";
    let response = common::post_form_with_cookie(&app, &format!("/documents/{id}"), &cookie, form_body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains(r#"<option value="contract" selected>Contract</option>"#), "doc_type should be persisted from the metadata form, got: {body}");
}
