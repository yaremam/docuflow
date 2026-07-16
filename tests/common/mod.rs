//! Shared request-building helpers for the integration tests. Each test file
//! compiles as its own crate, so any given binary only uses a subset of these
//! — the resulting dead-code warnings are an accepted characteristic of the
//! `tests/common/mod.rs` pattern, suppressed here rather than per call site.
#![allow(dead_code)]

use std::str::FromStr;

use docuflow::web::state::{self, AppState};
use http_body_util::BodyExt;
use sqlx::postgres::PgConnectOptions;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use tower::ServiceExt;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::PostgresStore;

pub struct TestApp {
    pub state: AppState,
    pub session_layer: SessionManagerLayer<PostgresStore>,
}

/// Deliberately **not** derived from `DATABASE_URL` (the app's own dev/prod
/// connection string, read from `.env` and used by the Docker-deployed
/// container) — tests get their own database, on the same Postgres server,
/// so a `cargo test` run can never truncate the dev stack's real data again.
/// `TEST_DATABASE_URL` is an explicit escape hatch (e.g. for CI pointing at
/// a different host), not something a normal dev-loop `.env` needs to set.
fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://admin:secretpassword@localhost:5432/doc_manager_db_test".to_string()
    })
}

/// Creates the test database if it doesn't exist yet, by connecting to the
/// server's built-in `postgres` maintenance database first — `PgPool::connect`
/// against a not-yet-existing database fails outright, so this has to run
/// before `state::connect` touches `test_database_url()` for real. Only
/// needs to happen once per test binary (see `DB_ENSURED` below); safe to
/// call from many test binaries racing at once, since each just re-checks
/// `pg_database` and no-ops if a sibling process already created it.
async fn ensure_test_database_exists(test_db_url: &str) {
    let options = PgConnectOptions::from_str(test_db_url)
        .expect("TEST_DATABASE_URL should be a valid Postgres connection string");
    let db_name = options
        .get_database()
        .expect("TEST_DATABASE_URL should include a database name")
        .to_string();
    let maintenance_options = options.database("postgres");

    let pool = PgPool::connect_with(maintenance_options)
        .await
        .expect("failed to connect to the Postgres server to create the test database — is `docker compose up -d postgres` running locally?");

    let exists: bool = sqlx::query_scalar("select exists(select 1 from pg_database where datname = $1)")
        .bind(&db_name)
        .fetch_one(&pool)
        .await
        .expect("failed to check whether the test database exists");

    if !exists {
        // Identifiers can't be bound as query parameters; safe here since
        // `db_name` comes from our own hardcoded `test_database_url`
        // default (or an operator-controlled `TEST_DATABASE_URL`), never
        // from request input.
        sqlx::query(&format!("create database \"{db_name}\""))
            .execute(&pool)
            .await
            .expect("failed to create the test database");
    }

    pool.close().await;
}

static DB_ENSURED: OnceCell<()> = OnceCell::const_new();

// Migrations are idempotent and process-wide; only actually run them once
// per test binary. Each test still connects its own fresh pool (see
// `test_state`) — a shared/cached pool caused contention flakiness under
// parallel test execution.
static MIGRATIONS_DONE: OnceCell<()> = OnceCell::const_new();

/// Arbitrary fixed key for the Postgres advisory lock guarding the
/// per-test `truncate` below — same coordination idiom already used by
/// `state::migrate`'s `ENSURE_BUCKET_LOCK_KEY`, just a different key so
/// the two locks don't collide. Needed because `cargo test`'s default
/// (multi-threaded, cross-binary) parallelism runs many tests' `truncate
/// users, tenants cascade` concurrently — two such truncates can lock the
/// cascaded tables in different orders and deadlock (`40P01`) under
/// Postgres's `AccessExclusiveLock` semantics for `TRUNCATE`. Serializing
/// just the truncate step avoids that without giving up intra-suite
/// parallelism for everything else.
const TRUNCATE_LOCK_KEY: i64 = 0x646f6375_74657374; // "docutest" in hex, truncated to fit i64

/// Connects to a dedicated `doc_manager_db_test` database on the same
/// Postgres instance as local dev (`docker-compose up -d` must be
/// running) — never the dev/prod `doc_manager_db` the Docker-deployed app
/// actually uses. Creates the test database on first use, runs our
/// migrations (once per test binary), and truncates `users`/`tenants` for
/// a clean slate.
pub async fn test_state() -> TestApp {
    let url = test_database_url();
    DB_ENSURED.get_or_init(|| ensure_test_database_exists(&url)).await;

    let (mut app_state, session_store, session_layer) = state::connect(&url)
        .await
        .expect("failed to connect to the test database — is `docker compose up -d` running locally?");

    // `state::connect` reads `BLOB_BUCKET_NAME` from the environment, which
    // (like `DATABASE_URL`) is the *real* dev bucket's name — tests must
    // never touch it, same isolation guarantee as the dedicated
    // `doc_manager_db_test` database above, and for the same reason
    // (found 2026-07-13: this project already fixed the equivalent DB-side
    // gap, but the blob-storage side was still reading straight from the
    // environment, so every `cargo test` run had been uploading to and
    // deleting from the real dev bucket). Overridden here rather than in
    // `state::connect` itself, since production code has no reason to know
    // about a test-only bucket name.
    let (client, presign_client) = docuflow::blob::clients_from_env().await;
    app_state.blob = docuflow::blob::BlobStore::new(client, presign_client, "docuflow-uploads-test".to_string());

    MIGRATIONS_DONE
        .get_or_init(|| async {
            state::migrate(&app_state.pool, &session_store, &app_state.blob)
                .await
                .expect("failed to migrate the test database or ensure the test bucket exists — is `docker compose up -d` running locally (including `minio`)?");
        })
        .await;

    let mut conn = app_state
        .pool
        .acquire()
        .await
        .expect("failed to acquire a connection to serialize the test truncate");
    sqlx::query!("select pg_advisory_lock($1)", TRUNCATE_LOCK_KEY)
        .fetch_one(&mut *conn)
        .await
        .expect("failed to acquire the truncate advisory lock");
    let truncate_result = sqlx::query!("truncate users, tenants cascade").execute(&mut *conn).await;
    sqlx::query!("select pg_advisory_unlock($1)", TRUNCATE_LOCK_KEY)
        .fetch_one(&mut *conn)
        .await
        .expect("failed to release the truncate advisory lock");
    truncate_result.expect("failed to truncate test tables");

    TestApp {
        state: app_state,
        session_layer,
    }
}

/// Builds a `TestApp` backed by a lazily-connected pool — suitable only for
/// routes that never touch the database (or blob storage), like static asset
/// serving. Avoids paying for a live Postgres/MinIO connection (and the
/// migration/truncate cost above) for tests that don't need one. Building
/// the S3 client itself makes no network call (it only resolves env-var
/// credentials), so it's safe to construct here even without MinIO
/// running.
pub async fn lazy_test_app() -> TestApp {
    let pool = sqlx::PgPool::connect_lazy(&test_database_url())
        .expect("TEST_DATABASE_URL should be a valid Postgres connection string");
    let session_store = PostgresStore::new(pool.clone());
    let session_layer = SessionManagerLayer::new(session_store).with_secure(false);
    let (client, presign_client) = docuflow::blob::clients_from_env().await;
    let blob = docuflow::blob::BlobStore::new(client, presign_client, "docuflow-uploads-test".to_string());
    let mailer = docuflow::mailer::Mailer::from_env().expect("mailer config should build from env");
    let app_base_url =
        std::env::var("APP_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    TestApp {
        state: AppState {
            pool,
            blob,
            mailer,
            app_base_url,
            ocr_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(2)),
        },
        session_layer,
    }
}

fn app(test_app: &TestApp) -> axum::Router {
    docuflow::web::router::app(test_app.state.clone(), test_app.session_layer.clone())
}

async fn request(
    test_app: &TestApp,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    form_body: Option<&str>,
) -> axum::http::Response<axum::body::Body> {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(axum::http::header::COOKIE, cookie);
    }
    let request = match form_body {
        Some(form_body) => builder
            .header(
                axum::http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(axum::body::Body::from(form_body.to_string())),
        None => builder.body(axum::body::Body::empty()),
    }
    .unwrap();

    app(test_app).oneshot(request).await.unwrap()
}

pub async fn get(test_app: &TestApp, uri: &str) -> axum::http::Response<axum::body::Body> {
    request(test_app, "GET", uri, None, None).await
}

pub async fn get_with_cookie(
    test_app: &TestApp,
    uri: &str,
    cookie: &str,
) -> axum::http::Response<axum::body::Body> {
    request(test_app, "GET", uri, Some(cookie), None).await
}

pub async fn post_form(
    test_app: &TestApp,
    uri: &str,
    form_body: &str,
) -> axum::http::Response<axum::body::Body> {
    request(test_app, "POST", uri, None, Some(form_body)).await
}

pub async fn post_form_with_cookie(
    test_app: &TestApp,
    uri: &str,
    cookie: &str,
    form_body: &str,
) -> axum::http::Response<axum::body::Body> {
    request(test_app, "POST", uri, Some(cookie), Some(form_body)).await
}

pub async fn post_with_cookie(
    test_app: &TestApp,
    uri: &str,
    cookie: &str,
) -> axum::http::Response<axum::body::Body> {
    request(test_app, "POST", uri, Some(cookie), None).await
}

/// A single multipart part, for building bodies with more than one field
/// (the document-upload form has several) in a caller-chosen order.
pub enum MultipartPart<'a> {
    Text { name: &'a str, value: &'a str },
    File { name: &'a str, filename: &'a str, content_type: &'a str, bytes: &'a [u8] },
}

fn multipart_body_with_parts(parts: &[MultipartPart]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "docuflow-test-boundary-multi";
    let mut body = Vec::new();
    for part in parts {
        match part {
            MultipartPart::Text { name, value } => {
                body.extend_from_slice(
                    format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                        .as_bytes(),
                );
            }
            MultipartPart::File { name, filename, content_type, bytes } => {
                body.extend_from_slice(
                    format!(
                        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(bytes);
                body.extend_from_slice(b"\r\n");
            }
        }
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

/// Posts a multipart body built from `parts`, in the given order — lets a
/// test exercise both the normal "metadata then file" order and a
/// deliberately out-of-order body.
pub async fn post_multipart_parts_with_cookie(
    test_app: &TestApp,
    uri: &str,
    cookie: &str,
    parts: &[MultipartPart<'_>],
) -> axum::http::Response<axum::body::Body> {
    let (multipart_content_type, body) = multipart_body_with_parts(parts);
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(uri)
        .header(axum::http::header::COOKIE, cookie)
        .header(axum::http::header::CONTENT_TYPE, multipart_content_type)
        .body(axum::body::Body::from(body))
        .unwrap();

    app(test_app).oneshot(request).await.unwrap()
}

/// Posts a single-file `multipart/form-data` body — a thin convenience
/// wrapper over `post_multipart_parts_with_cookie` for the common
/// one-file, no-metadata case.
pub async fn post_multipart_with_cookie(
    test_app: &TestApp,
    uri: &str,
    cookie: &str,
    field_name: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> axum::http::Response<axum::body::Body> {
    post_multipart_parts_with_cookie(
        test_app,
        uri,
        cookie,
        &[MultipartPart::File { name: field_name, filename, content_type, bytes }],
    )
    .await
}

/// Posts a `/signup` submission for the given email/password. The only
/// special character handled is `@` — fine for the plain test-fixture emails
/// used throughout this suite, not a general URL encoder.
pub async fn signup(
    test_app: &TestApp,
    email: &str,
    password: &str,
) -> axum::http::Response<axum::body::Body> {
    let form_body = format!("email={}&password={password}", email.replace('@', "%40"));
    post_form(test_app, "/signup", &form_body).await
}

/// Posts a `/login` submission for the given email/password.
pub async fn login(
    test_app: &TestApp,
    email: &str,
    password: &str,
) -> axum::http::Response<axum::body::Body> {
    let form_body = format!("email={}&password={password}", email.replace('@', "%40"));
    post_form(test_app, "/login", &form_body).await
}

/// Signs up a fresh account and immediately logs in with the same
/// credentials, returning the login response — the common "get an
/// authenticated session" setup shared by tests that need to already be
/// logged in.
/// Looks up a signed-up user's id by email — shared by every test file
/// that seeds document rows directly (bypassing the upload/OCR pipeline)
/// and needs a real `user_id`/`tenant_id` to insert them under.
pub async fn user_id(app: &TestApp, email: &str) -> uuid::Uuid {
    sqlx::query_scalar!("select id from users where email = $1", email).fetch_one(&app.state.pool).await.unwrap()
}

pub async fn signup_and_login(
    test_app: &TestApp,
    email: &str,
    password: &str,
) -> axum::http::Response<axum::body::Body> {
    signup(test_app, email, password).await;
    login(test_app, email, password).await
}

pub async fn body_string(response: axum::http::Response<axum::body::Body>) -> String {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(body.to_vec()).unwrap()
}

pub fn content_type(response: &axum::http::Response<axum::body::Body>) -> String {
    response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

pub fn location(response: &axum::http::Response<axum::body::Body>) -> Option<String> {
    response
        .headers()
        .get(axum::http::header::LOCATION)?
        .to_str()
        .ok()
        .map(str::to_string)
}

/// Extracts just the `name=value` pair from a `Set-Cookie` response header,
/// suitable for replaying as a `Cookie` request header in a follow-up call.
pub fn session_cookie(response: &axum::http::Response<axum::body::Body>) -> Option<String> {
    response
        .headers()
        .get(axum::http::header::SET_COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .next()
        .map(str::to_string)
}

/// Whether `name` resolves on `PATH` — used to soft-skip real-OCR tests
/// (`tesseract`, `pdftoppm`) rather than failing where the binary isn't
/// installed.
pub fn command_on_path(name: &str) -> bool {
    std::process::Command::new("which").arg(name).output().map(|o| o.status.success()).unwrap_or(false)
}

/// Whether `tesseract`'s installed trained-data set includes `lang` (e.g.
/// `"rus"`) — used to soft-skip Cyrillic OCR tests on a box that has
/// `tesseract` on `PATH` but not the `tesseract-ocr-rus` language pack,
/// rather than failing where the pack isn't installed.
pub fn tesseract_has_lang(lang: &str) -> bool {
    std::process::Command::new("tesseract")
        .arg("--list-langs")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|line| line.trim() == lang))
        .unwrap_or(false)
}

pub struct OcrOutcome {
    pub status: String,
    pub text: Option<String>,
    pub error: Option<String>,
    pub suggested_date_issued: Option<time::Date>,
}

/// Polls `documents.ocr_status` for `id` until it reaches a terminal state
/// (`done`/`failed`) or `timeout` elapses, whichever comes first — the OCR
/// pass runs as a detached `tokio::spawn` task, so polling (not a fixed
/// sleep) both keeps the common case fast and gives a slow CI box more
/// headroom than a single guess would. On timeout, `status` is left empty,
/// which every caller's `assert_eq!(status, "done"/"failed", ...)` already
/// turns into a clear failure.
pub async fn wait_for_ocr_outcome(app: &TestApp, id: uuid::Uuid, timeout: std::time::Duration) -> OcrOutcome {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let row = sqlx::query!(
            "select ocr_status, ocr_text, ocr_error, ocr_suggested_date_issued from documents where id = $1",
            id
        )
        .fetch_one(&app.state.pool)
        .await
        .unwrap();
        if row.ocr_status == "done" || row.ocr_status == "failed" {
            return OcrOutcome {
                status: row.ocr_status,
                text: row.ocr_text,
                error: row.ocr_error,
                suggested_date_issued: row.ocr_suggested_date_issued,
            };
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    OcrOutcome { status: String::new(), text: None, error: None, suggested_date_issued: None }
}

/// Extracts the document id out of a `/documents/{id}?...` redirect
/// Location header. Shared by every test file that uploads a document and
/// then needs its id (the upload response only returns a redirect, never
/// the id directly).
pub fn document_id_from_location(location: &str) -> uuid::Uuid {
    let after_prefix = location.strip_prefix("/documents/").expect("redirect should target /documents/{id}");
    let id_str = after_prefix.split('?').next().unwrap();
    id_str.parse().expect("redirect should contain a valid document id")
}

pub struct UploadedDocument {
    pub id: uuid::Uuid,
    pub cookie: String,
    pub outcome: OcrOutcome,
}

/// Shared by every real-OCR test (image, PDF, Cyrillic, dated): signs up a
/// fresh user, uploads `fixture_path` under `filename`/`content_type`, and
/// polls until OCR reaches a terminal state. Returns the new document's id
/// and the uploader's session cookie alongside the outcome, for tests that
/// need to keep interacting with it (viewing/editing the page) after OCR
/// completes.
pub async fn upload_and_wait_for_ocr(
    app: &TestApp,
    email: &str,
    fixture_path: &str,
    filename: &str,
    content_type: &str,
) -> UploadedDocument {
    let login = signup_and_login(app, email, "documentspassword").await;
    let cookie = session_cookie(&login).expect("login should set a session cookie");

    let bytes = std::fs::read(fixture_path).unwrap();
    let response = post_multipart_parts_with_cookie(
        app,
        "/documents",
        &cookie,
        &[MultipartPart::File { name: "file", filename, content_type, bytes: &bytes }],
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    let id = document_id_from_location(&location(&response).unwrap());

    let outcome = wait_for_ocr_outcome(app, id, std::time::Duration::from_secs(15)).await;
    UploadedDocument { id, cookie, outcome }
}
