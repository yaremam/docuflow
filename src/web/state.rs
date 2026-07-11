use sqlx::PgPool;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::PostgresStore;

use crate::blob::{self, BlobStore};
use crate::error::AppError;
use crate::mailer::Mailer;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub blob: BlobStore,
    pub mailer: Mailer,
    /// Embedded in emailed password-reset links, which are followed by the
    /// user's own browser — loaded once at boot rather than re-read from
    /// the environment on every `/forgot-password` request.
    pub app_base_url: String,
}

/// Connects to Postgres, builds an S3 client, and builds the session layer,
/// without running any migrations or bucket setup. Pair with `migrate` to
/// fully prepare a fresh database/bucket — split out so callers that need
/// that setup at most once (e.g. the test harness, which connects a fresh
/// pool per test) don't have to pay for a repeat check on every connection.
pub async fn connect(
    database_url: &str,
) -> Result<(AppState, PostgresStore, SessionManagerLayer<PostgresStore>), AppError> {
    let pool = PgPool::connect(database_url).await?;
    let session_store = PostgresStore::new(pool.clone());
    // Local dev serves plain HTTP (no TLS) — `secure` cookies would silently
    // never be sent by the browser, breaking every session. Revisit once
    // this is deployed behind TLS.
    let session_layer = SessionManagerLayer::new(session_store.clone()).with_secure(false);

    let bucket = std::env::var("BLOB_BUCKET_NAME").unwrap_or_else(|_| "docuflow-uploads".to_string());
    let (client, presign_client) = blob::clients_from_env().await;
    let blob = BlobStore::new(client, presign_client, bucket);

    let mailer = Mailer::from_env()?;

    let app_base_url =
        std::env::var("APP_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    Ok((
        AppState {
            pool,
            blob,
            mailer,
            app_base_url,
        },
        session_store,
        session_layer,
    ))
}

/// Arbitrary fixed key for the Postgres advisory lock guarding
/// `BlobStore::ensure_bucket` below — any `i64` works, it just needs to be
/// the same constant every caller uses.
const ENSURE_BUCKET_LOCK_KEY: i64 = 0x646f6375_666c6f77; // "docuflow" in hex, truncated to fit i64

/// Runs the app's own migrations, tower-sessions' self-managed session-table
/// migration, and ensures the blob-storage bucket exists. Idempotent, but
/// each call still does the round trips to verify that — callers invoking
/// this repeatedly in a short span (e.g. once per test) should guard it
/// themselves.
pub async fn migrate(pool: &PgPool, session_store: &PostgresStore, blob: &BlobStore) -> Result<(), AppError> {
    sqlx::migrate!().run(pool).await?;
    session_store.migrate().await?;

    // Several test binaries (separate processes) can call this at once on a
    // fresh environment, all racing to create the same S3 bucket — observed
    // to make LocalStack return a malformed response to one of the racing
    // HeadBucket calls. A Postgres advisory lock serializes them across
    // processes, the same coordination `sqlx::migrate!` already relies on
    // internally for its own cross-process safety.
    let mut conn = pool.acquire().await?;
    sqlx::query!("select pg_advisory_lock($1)", ENSURE_BUCKET_LOCK_KEY)
        .fetch_one(&mut *conn)
        .await?;
    let ensure_bucket_result = blob.ensure_bucket().await;
    sqlx::query!("select pg_advisory_unlock($1)", ENSURE_BUCKET_LOCK_KEY)
        .fetch_one(&mut *conn)
        .await?;
    ensure_bucket_result?;

    Ok(())
}

/// Connects, migrates, and builds the session layer in one call — the
/// common case for a fresh process (`main.rs`).
pub async fn bootstrap(
    database_url: &str,
) -> Result<(AppState, SessionManagerLayer<PostgresStore>), AppError> {
    let (state, session_store, session_layer) = connect(database_url).await?;
    migrate(&state.pool, &session_store, &state.blob).await?;
    Ok((state, session_layer))
}
