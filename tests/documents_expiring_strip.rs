mod common;

use common::user_id;

async fn seed_expiring_document(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    filename: &str,
    doc_type: &str,
    date_expires: Option<time::Date>,
) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents
            (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, ocr_status, doc_type, date_expires)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, '{}', 'done', $5, $6)",
        id,
        user_id,
        filename,
        blob_key,
        doc_type,
        date_expires,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

fn days_from_today(offset: i64) -> time::Date {
    time::OffsetDateTime::now_utc().date() + time::Duration::days(offset)
}

/// Just the strip's own markup — the regular results list below it
/// legitimately lists every document in the tenant regardless of expiry
/// status, so a plain `body.contains(...)` can't tell "in the strip"
/// apart from "just somewhere on the dashboard."
fn strip_html(body: &str) -> &str {
    let Some(start) = body.find("class=\"expiring-strip\"") else { return "" };
    let end = body[start..]
        .find("filters-layout")
        .or_else(|| body[start..].find("empty-state"))
        .map(|i| start + i)
        .unwrap_or(body.len());
    &body[start..end]
}

#[tokio::test]
async fn strip_shows_expired_and_soon_to_expire_documents() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "stripbasic.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "stripbasic.docs@example.com").await;

    seed_expiring_document(&app.state.pool, user, "expired.pdf", "insurance", Some(days_from_today(-3))).await;
    seed_expiring_document(&app.state.pool, user, "soon.pdf", "insurance", Some(days_from_today(5))).await;
    seed_expiring_document(&app.state.pool, user, "later.pdf", "insurance", Some(days_from_today(60))).await;

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    let strip = strip_html(&body);
    assert!(body.contains("expiring-strip"), "expected the strip to render, got: {body}");
    assert!(strip.contains("expired.pdf"), "expected the expired doc in the strip, got: {strip}");
    assert!(strip.contains("soon.pdf"), "expected the soon-to-expire doc in the strip, got: {strip}");
    assert!(!strip.contains("later.pdf"), "a doc expiring well beyond 14 days shouldn't be in the strip, got: {strip}");
}

#[tokio::test]
async fn strip_is_absent_when_nothing_is_expiring() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "stripabsent.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "stripabsent.docs@example.com").await;

    seed_expiring_document(&app.state.pool, user, "far_future.pdf", "insurance", Some(days_from_today(60))).await;
    seed_expiring_document(&app.state.pool, user, "no_expiry.pdf", "insurance", None).await;

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains("expiring-strip"), "the strip shouldn't render when nothing qualifies, got: {body}");
}

#[tokio::test]
async fn strip_excludes_ineligible_doc_types_even_with_date_expires_set() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "stripineligible.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "stripineligible.docs@example.com").await;

    // A receipt with date_expires set anyway (the UI never allows this,
    // but the DB doesn't enforce it — see documents.rs's update() doc
    // comment on why that's an accepted trust-boundary judgment call).
    seed_expiring_document(&app.state.pool, user, "receipt_with_expiry.pdf", "receipt", Some(days_from_today(-3))).await;

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    assert!(!body.contains("expiring-strip"), "an ineligible doc_type shouldn't trigger the strip even with date_expires set, got: {body}");
}

#[tokio::test]
async fn strip_shows_most_overdue_first() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "striporder.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "striporder.docs@example.com").await;

    seed_expiring_document(&app.state.pool, user, "less_overdue.pdf", "insurance", Some(days_from_today(-2))).await;
    seed_expiring_document(&app.state.pool, user, "more_overdue.pdf", "insurance", Some(days_from_today(-10))).await;

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;
    let body = common::body_string(response).await;
    let strip = strip_html(&body);
    let more_overdue_pos = strip.find("more_overdue.pdf").expect("expected more_overdue.pdf in the strip");
    let less_overdue_pos = strip.find("less_overdue.pdf").expect("expected less_overdue.pdf in the strip");
    assert!(more_overdue_pos < less_overdue_pos, "the most-overdue document should sort first in the strip, got: {strip}");
}

#[tokio::test]
async fn strip_is_unaffected_by_active_facets_or_search() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "stripfacet.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "stripfacet.docs@example.com").await;

    seed_expiring_document(&app.state.pool, user, "expiring_untagged.pdf", "insurance", Some(days_from_today(-3))).await;

    // An active tag facet that this document doesn't match would exclude
    // it from the results list below, but the strip answers "what needs
    // my attention right now" independent of whatever's currently
    // filtered (TDR 031 §4).
    let response = common::get_with_cookie(&app, "/documents?tags=unrelated-tag", &cookie).await;
    let body = common::body_string(response).await;
    assert!(
        body.contains("expiring-strip") && body.contains("expiring_untagged.pdf"),
        "the strip should still show an expiring doc even when an active facet would exclude it from the results below, got: {body}"
    );
}
