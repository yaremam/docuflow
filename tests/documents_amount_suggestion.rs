mod common;

use common::user_id;

/// Seeds a document directly (bypassing upload/OCR), like
/// `documents_expiry_suggestion.rs`'s own helper) with a confirmed
/// doc_type, and optional confirmed/suggested amounts — these tests
/// exercise the gating/suggestion/accept web-layer logic, not the
/// keyword extraction itself (covered by `amount_extract`'s own unit
/// tests), so there's no need to depend on real `tesseract`.
#[allow(clippy::too_many_arguments)]
async fn seed_document_with_amount(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    filename: &str,
    doc_type: Option<&str>,
    ocr_suggested_doc_type: Option<&str>,
    amount: Option<i64>,
    ocr_suggested_amount: Option<i64>,
) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents
            (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status,
             doc_type, ocr_suggested_doc_type, amount, ocr_suggested_amount)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, '{}', 'done', $5, $6, $7, $8)",
        id,
        user_id,
        filename,
        blob_key,
        doc_type,
        ocr_suggested_doc_type,
        amount,
        ocr_suggested_amount,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn amount_field_is_shown_for_an_eligible_confirmed_doc_type() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "amountshown.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "amountshown.docs@example.com").await;

    let id = seed_document_with_amount(&app.state.pool, user, "bill.pdf", Some("bill"), None, None, None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains(r#"name="amount""#), "amount field should show for an eligible confirmed doc_type, got: {body}");
}

#[tokio::test]
async fn amount_field_is_hidden_for_an_ineligible_confirmed_doc_type() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "amounthidden.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "amounthidden.docs@example.com").await;

    let id = seed_document_with_amount(&app.state.pool, user, "policy.pdf", Some("insurance"), None, None, None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains(r#"name="amount""#), "amount field shouldn't show for insurance, got: {body}");
}

#[tokio::test]
async fn amount_field_is_hidden_when_doc_type_is_unset() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "amountunset.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "amountunset.docs@example.com").await;

    let id = seed_document_with_amount(&app.state.pool, user, "plain.pdf", None, None, None, None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains(r#"name="amount""#), "amount field shouldn't show with no confirmed doc_type, got: {body}");
}

#[tokio::test]
async fn amount_field_is_hidden_when_only_an_eligible_doc_type_is_suggested_not_confirmed() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "amountsuggestedonly.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "amountsuggestedonly.docs@example.com").await;

    // doc_type is unconfirmed — only ocr_suggested_doc_type is set — so
    // the amount field must stay hidden until the type itself is accepted
    // (mirrors expiry's AC-1, confirmed-only gating).
    let id = seed_document_with_amount(&app.state.pool, user, "maybebill.pdf", None, Some("bill"), None, None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        !body.contains(r#"name="amount""#),
        "amount field shouldn't show for an unconfirmed doc_type suggestion, got: {body}"
    );
}

#[tokio::test]
async fn a_suggested_amount_shows_the_accept_action_when_unset() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "amountsuggest.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "amountsuggest.docs@example.com").await;

    let id = seed_document_with_amount(&app.state.pool, user, "bill.pdf", Some("bill"), None, None, Some(4500)).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        body.contains(&format!("/documents/{id}/accept_suggested_amount")),
        "a fresh amount suggestion with no amount set should show the accept action, got: {body}"
    );
    assert!(body.contains("45.00"), "the suggested amount should be shown, got: {body}");
}

#[tokio::test]
async fn no_suggestion_box_when_no_amount_was_suggested() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "noamountsuggest.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "noamountsuggest.docs@example.com").await;

    let id = seed_document_with_amount(&app.state.pool, user, "bill.pdf", Some("bill"), None, None, None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains("accept_suggested_amount"), "no suggestion should render when none was found, got: {body}");
}

#[tokio::test]
async fn amount_suggestion_is_hidden_once_amount_is_already_set() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "amountalreadyset.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "amountalreadyset.docs@example.com").await;

    let id = seed_document_with_amount(&app.state.pool, user, "bill.pdf", Some("bill"), None, Some(3000), Some(4500)).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        !body.contains("accept_suggested_amount"),
        "the suggestion should stop showing once amount has a value, even though ocr_suggested_amount is still set, got: {body}"
    );
    assert!(body.contains("30.00"), "the confirmed amount should still be shown in the field, got: {body}");
}

#[tokio::test]
async fn accept_suggested_amount_copies_it_into_amount_and_redirects() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "acceptamount.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "acceptamount.docs@example.com").await;

    let id = seed_document_with_amount(&app.state.pool, user, "bill.pdf", Some("bill"), None, None, Some(4500)).await;

    let response = common::post_with_cookie(&app, &format!("/documents/{id}/accept_suggested_amount"), &cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response).unwrap(), format!("/documents/{id}?saved=true"));

    let row = sqlx::query!("select amount from documents where id = $1", id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.amount, Some(4500));
}

#[tokio::test]
async fn accept_suggested_amount_never_overwrites_an_already_set_amount() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "noacceptamount.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "noacceptamount.docs@example.com").await;

    let id = seed_document_with_amount(&app.state.pool, user, "bill.pdf", Some("bill"), None, Some(1000), Some(4500)).await;

    let response = common::post_with_cookie(&app, &format!("/documents/{id}/accept_suggested_amount"), &cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let row = sqlx::query!("select amount from documents where id = $1", id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.amount, Some(1000), "an already-set amount should never be overwritten by accept");
}

#[tokio::test]
async fn accept_suggested_amount_is_tenant_scoped() {
    let app = common::test_state().await;
    common::signup_and_login(&app, "tenantA.amountsuggest@example.com", "documentspassword").await;
    let user = user_id(&app, "tenantA.amountsuggest@example.com").await;
    let id = seed_document_with_amount(&app.state.pool, user, "bill.pdf", Some("bill"), None, None, Some(4500)).await;

    let other_login = common::signup_and_login(&app, "tenantB.amountsuggest@example.com", "documentspassword").await;
    let other_cookie = common::session_cookie(&other_login).expect("login should set a session cookie");

    let response = common::post_with_cookie(&app, &format!("/documents/{id}/accept_suggested_amount"), &other_cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn manually_setting_amount_via_the_metadata_form_persists_it() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "manualamount.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "manualamount.docs@example.com").await;

    let id = seed_document_with_amount(&app.state.pool, user, "bill.pdf", Some("bill"), None, None, None).await;

    let form_body = "title=My+Bill&tags=&date_issued=&doc_type=bill&language=&amount=128.50";
    let response = common::post_form_with_cookie(&app, &format!("/documents/{id}"), &cookie, form_body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let row = sqlx::query!("select amount from documents where id = $1", id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.amount, Some(12850));
}
