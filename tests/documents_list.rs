mod common;

async fn user_id(app: &common::TestApp, email: &str) -> uuid::Uuid {
    sqlx::query_scalar!("select id from users where email = $1", email)
        .fetch_one(&app.state.pool)
        .await
        .unwrap()
}

#[allow(clippy::too_many_arguments)]
async fn seed_document(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    filename: &str,
    tags: &[&str],
    date_issued: Option<time::Date>,
    created_at: time::OffsetDateTime,
) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let tags: Vec<String> = tags.iter().map(|tag| tag.to_string()).collect();
    let blob_key = format!("documents/{user_id}/{id}");
    sqlx::query!(
        "insert into documents
            (id, tenant_id, user_id, original_filename, content_type, file_size_bytes, blob_key, tags, date_issued, ocr_status, created_at)
         values ($1, $2, $2, $3, 'application/pdf', 100, $4, $5, $6, 'done', $7)",
        id,
        user_id,
        filename,
        blob_key,
        &tags,
        date_issued,
        created_at,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

fn date(year: i32, month: u8, day: u8) -> time::Date {
    time::Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), day).unwrap()
}

fn datetime(year: i32, month: u8, day: u8) -> time::OffsetDateTime {
    date(year, month, day).midnight().assume_utc()
}

#[tokio::test]
async fn empty_state_renders_when_tenant_has_no_documents() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "empty.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");

    let response = common::get_with_cookie(&app, "/documents", &cookie).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = common::body_string(response).await;
    assert!(body.contains("No documents yet"));
}

#[tokio::test]
async fn a_user_only_sees_their_own_tenants_documents() {
    let app = common::test_state().await;

    let login_a = common::signup_and_login(&app, "tenant.a@example.com", "documentspassword").await;
    let cookie_a = common::session_cookie(&login_a).expect("login should set a session cookie");
    let user_a = user_id(&app, "tenant.a@example.com").await;

    let login_b = common::signup_and_login(&app, "tenant.b@example.com", "documentspassword").await;
    let cookie_b = common::session_cookie(&login_b).expect("login should set a session cookie");
    let user_b = user_id(&app, "tenant.b@example.com").await;

    seed_document(&app.state.pool, user_a, "tenant_a_bill.pdf", &["utilities"], None, datetime(2026, 1, 1)).await;
    seed_document(&app.state.pool, user_b, "tenant_b_bill.pdf", &["utilities"], None, datetime(2026, 1, 1)).await;

    let response_a = common::get_with_cookie(&app, "/documents", &cookie_a).await;
    let body_a = common::body_string(response_a).await;
    assert!(body_a.contains("tenant_a_bill.pdf"));
    assert!(!body_a.contains("tenant_b_bill.pdf"));

    let response_b = common::get_with_cookie(&app, "/documents", &cookie_b).await;
    let body_b = common::body_string(response_b).await;
    assert!(body_b.contains("tenant_b_bill.pdf"));
    assert!(!body_b.contains("tenant_a_bill.pdf"));
}

#[tokio::test]
async fn tag_search_filters_by_overlap() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "tagsearch.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "tagsearch.docs@example.com").await;

    seed_document(&app.state.pool, user, "insurance_doc.pdf", &["insurance", "auto"], None, datetime(2026, 1, 1)).await;
    seed_document(&app.state.pool, user, "gas_bill.pdf", &["utilities"], None, datetime(2026, 1, 2)).await;

    let response = common::get_with_cookie(&app, "/documents?q=insurance", &cookie).await;
    let body = common::body_string(response).await;
    assert!(body.contains("insurance_doc.pdf"));
    assert!(!body.contains("gas_bill.pdf"));
}

#[tokio::test]
async fn sort_by_date_issued_orders_documents_correctly() {
    let app = common::test_state().await;
    let login = common::signup_and_login(&app, "sort.docs@example.com", "documentspassword").await;
    let cookie = common::session_cookie(&login).expect("login should set a session cookie");
    let user = user_id(&app, "sort.docs@example.com").await;

    seed_document(&app.state.pool, user, "earliest.pdf", &["bill"], Some(date(2026, 1, 1)), datetime(2026, 3, 1)).await;
    seed_document(&app.state.pool, user, "latest.pdf", &["bill"], Some(date(2026, 6, 1)), datetime(2026, 2, 1)).await;

    let response = common::get_with_cookie(&app, "/documents?sort=date_issued_asc", &cookie).await;
    let body = common::body_string(response).await;
    let earliest_pos = body.find("earliest.pdf").expect("earliest.pdf should be present");
    let latest_pos = body.find("latest.pdf").expect("latest.pdf should be present");
    assert!(earliest_pos < latest_pos, "date_issued_asc should list the earlier issue date first");
}
