use sqlx::PgPool;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::PostgresStore;

use crate::error::AppError;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
}

/// Connects to Postgres and builds the session layer, without running any
/// migrations. Pair with `migrate` to fully prepare a fresh database — split
/// out so callers that need to run migrations at most once (e.g. the test
/// harness, which connects a fresh pool per test) don't have to pay for a
/// repeat migration check on every connection.
pub async fn connect(
    database_url: &str,
) -> Result<(AppState, PostgresStore, SessionManagerLayer<PostgresStore>), AppError> {
    let pool = PgPool::connect(database_url).await?;
    let session_store = PostgresStore::new(pool.clone());
    // Local dev serves plain HTTP (no TLS) — `secure` cookies would silently
    // never be sent by the browser, breaking every session. Revisit once
    // this is deployed behind TLS.
    let session_layer = SessionManagerLayer::new(session_store.clone()).with_secure(false);

    Ok((AppState { pool }, session_store, session_layer))
}

/// Runs the app's own migrations plus tower-sessions' self-managed
/// session-table migration. Idempotent, but each call still does the round
/// trips to verify that — callers invoking this repeatedly in a short span
/// (e.g. once per test) should guard it themselves.
pub async fn migrate(pool: &PgPool, session_store: &PostgresStore) -> Result<(), AppError> {
    sqlx::migrate!().run(pool).await?;
    session_store.migrate().await?;
    Ok(())
}

/// Connects, migrates, and builds the session layer in one call — the
/// common case for a fresh process (`main.rs`).
pub async fn bootstrap(
    database_url: &str,
) -> Result<(AppState, SessionManagerLayer<PostgresStore>), AppError> {
    let (state, session_store, session_layer) = connect(database_url).await?;
    migrate(&state.pool, &session_store).await?;
    Ok((state, session_layer))
}
