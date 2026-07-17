mod common;

use common::user_id;

/// Seeds a document directly (bypassing upload/OCR, like
/// `documents_doc_type_suggestion.rs`'s own helper) with a confirmed
/// doc_type, and optional confirmed/suggested expiry dates — these tests
/// exercise the gating/suggestion/accept web-layer logic, not the
/// keyword extraction itself (covered by `expiry_extract`'s own unit
/// tests), so there's no need to depend on real `tesseract`.
#[allow(clippy::too_many_arguments)]
async fn seed_document_with_expiry(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    filename: &str,
    doc_type: Option<&str>,
    ocr_suggested_doc_type: Option<&str>,
    date_expires: Option<time::Date>,
    ocr_suggested_date_expires: Option<time::Date>,
) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents
            (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status,
             doc_type, ocr_suggested_doc_type, date_expires, ocr_suggested_date_expires)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, '{}', 'done', $5, $6, $7, $8)",
        id,
        user_id,
        filename,
        blob_key,
        doc_type,
        ocr_suggested_doc_type,
        date_expires,
        ocr_suggested_date_expires,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

fn date(year: i32, month: u8, day: u8) -> time::Date {
    time::Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), day).unwrap()
}

#[tokio::test]
async fn expiry_field_is_shown_for_an_eligible_confirmed_doc_type() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "expiryshown.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "expiryshown.docs@example.com").await;

    let id = seed_document_with_expiry(&app.state.pool, user, "policy.pdf", Some("insurance"), None, None, None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains(r#"name="date_expires""#), "expiry field should show for an eligible confirmed doc_type, got: {body}");
}

#[tokio::test]
async fn expiry_field_is_hidden_for_an_ineligible_confirmed_doc_type() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "expiryhidden.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "expiryhidden.docs@example.com").await;

    let id = seed_document_with_expiry(&app.state.pool, user, "receipt.pdf", Some("receipt"), None, None, None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains(r#"name="date_expires""#), "expiry field shouldn't show for receipt, got: {body}");
}

#[tokio::test]
async fn expiry_field_is_hidden_when_doc_type_is_unset() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "expiryunset.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "expiryunset.docs@example.com").await;

    let id = seed_document_with_expiry(&app.state.pool, user, "plain.pdf", None, None, None, None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains(r#"name="date_expires""#), "expiry field shouldn't show with no confirmed doc_type, got: {body}");
}

#[tokio::test]
async fn expiry_field_is_hidden_when_only_an_eligible_doc_type_is_suggested_not_confirmed() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "expirysuggestedonly.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "expirysuggestedonly.docs@example.com").await;

    // doc_type is unconfirmed — only ocr_suggested_doc_type is set — so
    // the expiry field must stay hidden until the type itself is accepted
    // (AC-1, confirmed-only gating).
    let id = seed_document_with_expiry(&app.state.pool, user, "maybeinsurance.pdf", None, Some("insurance"), None, None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        !body.contains(r#"name="date_expires""#),
        "expiry field shouldn't show for an unconfirmed doc_type suggestion, got: {body}"
    );
}

#[tokio::test]
async fn a_suggested_expiry_date_shows_the_accept_action_when_unset() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "expirysuggest.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "expirysuggest.docs@example.com").await;

    let id =
        seed_document_with_expiry(&app.state.pool, user, "policy.pdf", Some("insurance"), None, None, Some(date(2026, 7, 31))).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        body.contains(&format!("/documents/{id}/accept_suggested_expiry_date")),
        "a fresh expiry suggestion with no date_expires set should show the accept action, got: {body}"
    );
    assert!(body.contains("2026-07-31"), "the suggested date should be shown, got: {body}");
}

#[tokio::test]
async fn no_suggestion_box_when_no_expiry_was_suggested() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "noexpirysuggest.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "noexpirysuggest.docs@example.com").await;

    let id = seed_document_with_expiry(&app.state.pool, user, "policy.pdf", Some("insurance"), None, None, None).await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        !body.contains("accept_suggested_expiry_date"),
        "no suggestion should render when none was found, got: {body}"
    );
}

#[tokio::test]
async fn expiry_suggestion_is_hidden_once_date_expires_is_already_set() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "expiryalreadyset.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "expiryalreadyset.docs@example.com").await;

    let id = seed_document_with_expiry(
        &app.state.pool,
        user,
        "policy.pdf",
        Some("insurance"),
        None,
        Some(date(2026, 1, 1)),
        Some(date(2026, 7, 31)),
    )
    .await;

    let response = common::get_with_cookie(&app, &format!("/documents/{id}"), &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        !body.contains("accept_suggested_expiry_date"),
        "the suggestion should stop showing once date_expires has a value, even though ocr_suggested_date_expires is still set, got: {body}"
    );
    assert!(body.contains("2026-01-01"), "the confirmed date should still be shown in the field, got: {body}");
}

#[tokio::test]
async fn accept_suggested_expiry_date_copies_it_into_date_expires_and_redirects() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "acceptexpiry.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "acceptexpiry.docs@example.com").await;

    let id =
        seed_document_with_expiry(&app.state.pool, user, "policy.pdf", Some("insurance"), None, None, Some(date(2026, 7, 31))).await;

    let response = common::post_with_cookie(&app, &format!("/documents/{id}/accept_suggested_expiry_date"), &cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response).unwrap(), format!("/documents/{id}?saved=true"));

    let row = sqlx::query!("select date_expires from documents where id = $1", id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.date_expires, Some(date(2026, 7, 31)));
}

#[tokio::test]
async fn accept_suggested_expiry_date_never_overwrites_an_already_set_date_expires() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "noacceptexpiry.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "noacceptexpiry.docs@example.com").await;

    let id = seed_document_with_expiry(
        &app.state.pool,
        user,
        "policy.pdf",
        Some("insurance"),
        None,
        Some(date(2020, 1, 1)),
        Some(date(2026, 7, 31)),
    )
    .await;

    let response = common::post_with_cookie(&app, &format!("/documents/{id}/accept_suggested_expiry_date"), &cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let row = sqlx::query!("select date_expires from documents where id = $1", id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.date_expires, Some(date(2020, 1, 1)), "an already-set date_expires should never be overwritten by accept");
}

#[tokio::test]
async fn accept_suggested_expiry_date_is_tenant_scoped() {
    let app = common::test_state().await;
    common::signup_and_login(&app, "tenantA.expirysuggest@example.com", "documentspassword").await;
    let user = user_id(&app, "tenantA.expirysuggest@example.com").await;
    let id =
        seed_document_with_expiry(&app.state.pool, user, "policy.pdf", Some("insurance"), None, None, Some(date(2026, 7, 31))).await;

    let other_login = common::signup_and_login(&app, "tenantB.expirysuggest@example.com", "documentspassword").await;
    let other_cookie = common::session_cookie(&other_login).expect("login should set a session cookie");

    let response = common::post_with_cookie(&app, &format!("/documents/{id}/accept_suggested_expiry_date"), &other_cookie).await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn manually_setting_date_expires_via_the_metadata_form_persists_it() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "manualexpiry.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "manualexpiry.docs@example.com").await;

    let id = seed_document_with_expiry(&app.state.pool, user, "policy.pdf", Some("insurance"), None, None, None).await;

    let form_body = "title=My+Policy&tags=&date_issued=&doc_type=insurance&language=&date_expires=2027-01-01";
    let response = common::post_form_with_cookie(&app, &format!("/documents/{id}"), &cookie, form_body).await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);

    let row = sqlx::query!("select date_expires from documents where id = $1", id).fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(row.date_expires, Some(date(2027, 1, 1)));
}
