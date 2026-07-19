mod common;

use common::user_id;

/// Seeds a document directly with a confirmed doc_type, confirmed/suggested
/// amount, and a real-world date — /spending's own aggregation logic is
/// what these tests exercise, not OCR, so there's no need to depend on
/// real `tesseract` (same rationale as `documents_amount_suggestion.rs`).
#[allow(clippy::too_many_arguments)]
async fn seed_document_for_spending(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    filename: &str,
    doc_type: Option<&str>,
    date_issued: Option<time::Date>,
    amount: Option<i64>,
    ocr_suggested_amount: Option<i64>,
) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents
            (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status,
             doc_type, date_issued, amount, ocr_suggested_amount)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, '{}', 'done', $5, $6, $7, $8)",
        id,
        user_id,
        filename,
        blob_key,
        doc_type,
        date_issued,
        amount,
        ocr_suggested_amount,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

fn this_month() -> time::Date {
    let today = time::OffsetDateTime::now_utc().date();
    time::Date::from_calendar_date(today.year(), today.month(), 1).unwrap()
}

#[tokio::test]
async fn get_spending_without_session_redirects_to_login() {
    let app = common::test_state().await;
    let response = common::get(&app, "/spending").await;

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    assert_eq!(common::location(&response), Some("/login".to_string()));
}

#[tokio::test]
async fn spending_page_shows_empty_state_when_no_confirmed_amounts_exist() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "spendingempty@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "spendingempty@example.com").await;

    // A suggestion alone (never accepted) must not count.
    seed_document_for_spending(&app.state.pool, user, "bill.pdf", Some("bill"), Some(this_month()), None, Some(4500)).await;

    let response = common::get_with_cookie(&app, "/spending", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("No confirmed amounts yet"), "expected the empty state, got: {body}");
}

#[tokio::test]
async fn spending_page_sums_confirmed_bill_and_receipt_amounts_in_the_current_month() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "spendingsum@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "spendingsum@example.com").await;

    seed_document_for_spending(&app.state.pool, user, "bill.pdf", Some("bill"), Some(this_month()), Some(4500), None).await;
    seed_document_for_spending(&app.state.pool, user, "receipt.pdf", Some("receipt"), Some(this_month()), Some(1000), None)
        .await;

    let response = common::get_with_cookie(&app, "/spending", &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains("No confirmed amounts yet"), "should not show the empty state, got: {body}");
    // 45.00 + 10.00 = 55.00, displayed as a whole-unit "55" (feature 032's
    // "unitless, whole-number" display convention for this page).
    assert!(body.contains(">55<"), "expected the combined total 55 to appear, got: {body}");
}

#[tokio::test]
async fn spending_page_excludes_an_unaccepted_ocr_suggested_amount() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "spendingsuggested@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "spendingsuggested@example.com").await;

    seed_document_for_spending(&app.state.pool, user, "bill.pdf", Some("bill"), Some(this_month()), None, Some(9900)).await;

    let response = common::get_with_cookie(&app, "/spending", &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        body.contains("No confirmed amounts yet"),
        "an unaccepted ocr_suggested_amount alone must not count, got: {body}"
    );
}

#[tokio::test]
async fn spending_page_excludes_a_confirmed_amount_on_an_ineligible_doc_type() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "spendingineligible@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "spendingineligible@example.com").await;

    // Directly-set amount on an insurance doc (not normally reachable via
    // the UI, which only renders the Amount field for bill/receipt) —
    // the query itself must still exclude it, not just the form.
    seed_document_for_spending(&app.state.pool, user, "policy.pdf", Some("insurance"), Some(this_month()), Some(5000), None)
        .await;

    let response = common::get_with_cookie(&app, "/spending", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("No confirmed amounts yet"), "an insurance premium must not count as spend, got: {body}");
}

#[tokio::test]
async fn spending_page_excludes_a_confirmed_amount_with_no_date_issued() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "spendingnodate@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "spendingnodate@example.com").await;

    seed_document_for_spending(&app.state.pool, user, "bill.pdf", Some("bill"), None, Some(4500), None).await;

    let response = common::get_with_cookie(&app, "/spending", &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        body.contains("No confirmed amounts yet"),
        "a confirmed amount with no date_issued can't be placed on the chart, got: {body}"
    );
}

#[tokio::test]
async fn spending_page_is_tenant_scoped() {
    let app = common::test_state().await;
    common::signup_and_login(&app, "tenantA.spending@example.com", "documentspassword").await;
    let user_a = user_id(&app, "tenantA.spending@example.com").await;
    seed_document_for_spending(&app.state.pool, user_a, "bill.pdf", Some("bill"), Some(this_month()), Some(9999), None)
        .await;

    let login_b = common::signup_and_login(&app, "tenantB.spending@example.com", "documentspassword").await;
    let cookie_b = common::session_cookie(&login_b).expect("login should set a session cookie");

    let response = common::get_with_cookie(&app, "/spending", &cookie_b).await;
    let body = common::body_string(response).await;
    assert!(
        body.contains("No confirmed amounts yet"),
        "tenant B should not see tenant A's spend, got: {body}"
    );
}
