//! Shared request-building helpers for the integration tests. Each test file
//! compiles as its own crate, so any given binary only uses a subset of these
//! — the resulting dead-code warnings are an accepted characteristic of the
//! `tests/common/mod.rs` pattern, suppressed here rather than per call site.
#![allow(dead_code)]

use docuflow::web::state::{self, AppState};
use http_body_util::BodyExt;
use tokio::sync::OnceCell;
use tower::ServiceExt;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::PostgresStore;

pub struct TestApp {
    pub state: AppState,
    pub session_layer: SessionManagerLayer<PostgresStore>,
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://admin:secretpassword@localhost:5432/doc_manager_db".to_string()
    })
}

// Migrations are idempotent and process-wide; only actually run them once
// per test binary. Each test still connects its own fresh pool (see
// `test_state`) — a shared/cached pool caused contention flakiness under
// parallel test execution.
static MIGRATIONS_DONE: OnceCell<()> = OnceCell::const_new();

/// Connects to the same Postgres instance as local dev (`docker-compose up -d`
/// must be running), runs our migrations (once per test binary), and
/// truncates `users`/`tenants` for a clean slate.
pub async fn test_state() -> TestApp {
    let (app_state, session_store, session_layer) = state::connect(&database_url())
        .await
        .expect("failed to connect to the test database — is `docker compose up -d` running locally?");

    MIGRATIONS_DONE
        .get_or_init(|| async {
            state::migrate(&app_state.pool, &session_store, &app_state.blob)
                .await
                .expect("failed to migrate the test database or ensure the test bucket exists — is `docker compose up -d` running locally (including `localstack`)?");
        })
        .await;

    sqlx::query!("truncate users, tenants cascade")
        .execute(&app_state.pool)
        .await
        .expect("failed to truncate test tables");

    TestApp {
        state: app_state,
        session_layer,
    }
}

/// Builds a `TestApp` backed by a lazily-connected pool — suitable only for
/// routes that never touch the database (or blob storage), like static asset
/// serving. Avoids paying for a live Postgres/LocalStack connection (and the
/// migration/truncate cost above) for tests that don't need one. Building
/// the S3 client itself makes no network call (it only resolves env-var
/// credentials), so it's safe to construct here even without LocalStack
/// running.
pub async fn lazy_test_app() -> TestApp {
    let pool = sqlx::PgPool::connect_lazy(&database_url())
        .expect("DATABASE_URL should be a valid Postgres connection string");
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

/// Builds a minimal single-file `multipart/form-data` body — just enough
/// structure for the one file-upload route this suite tests, not a general
/// multipart builder.
fn multipart_body(field_name: &str, filename: &str, content_type: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "docuflow-test-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{field_name}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

pub async fn post_multipart_with_cookie(
    test_app: &TestApp,
    uri: &str,
    cookie: &str,
    field_name: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> axum::http::Response<axum::body::Body> {
    let (multipart_content_type, body) = multipart_body(field_name, filename, content_type, bytes);
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(uri)
        .header(axum::http::header::COOKIE, cookie)
        .header(axum::http::header::CONTENT_TYPE, multipart_content_type)
        .body(axum::body::Body::from(body))
        .unwrap();

    app(test_app).oneshot(request).await.unwrap()
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
