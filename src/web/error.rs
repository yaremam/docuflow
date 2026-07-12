//! Web-layer error type. Distinct from the bootstrap-only `crate::error::AppError`
//! (telemetry/bind/serve failures returned from `main()`), this covers
//! fallible actions inside request handlers and maps them to HTTP responses
//! without ever leaking the underlying error detail to the client — the raw
//! `sqlx`/`argon2`/session error is logged server-side instead.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};

#[derive(Debug, thiserror::Error)]
pub enum AppWebError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("password hashing error: {0}")]
    Hashing(#[from] argon2::password_hash::Error),
    #[error("session store error: {0}")]
    Session(#[from] tower_sessions::session::Error),
    #[error("background task error: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
    #[error("blob storage error: {0}")]
    Blob(#[from] crate::blob::BlobError),
    #[error("multipart error: {0}")]
    Multipart(#[from] axum::extract::multipart::MultipartError),
    #[error("mail transport error: {0}")]
    Mail(String),
    #[error("this reset link is invalid, expired, or already used")]
    InvalidResetToken,
    #[error("qr code generation error: {0}")]
    QrGeneration(#[from] qrcode::types::QrError),
    /// Should never actually happen — `scan_sessions.status` only ever
    /// becomes `'captured'` in the same `UPDATE` that also sets
    /// `document_id` (see `web::handlers::scan::submit_scan`). Surfacing
    /// this as a `Result` rather than `.expect()`-ing keeps that assumption
    /// from ever becoming a panic if it's wrong, per CLAUDE.md's zero-panic
    /// rule.
    #[error("scan session is captured but missing its document id")]
    InconsistentScanSession,
    #[error("authentication required")]
    Unauthenticated,
    #[error("not found")]
    NotFound,
}

impl IntoResponse for AppWebError {
    fn into_response(self) -> Response {
        match self {
            AppWebError::Unauthenticated => Redirect::to("/login").into_response(),
            // `reset_password_form` catches this variant explicitly to
            // render a friendlier "invalid or expired" template state — this
            // arm is that path's defense-in-depth fallback, not the primary
            // one. `reset_password_submit` never produces this variant at
            // all: it needs to row-lock the token for its update, so it runs
            // its own `for update` query and renders the equivalent state
            // directly rather than going through this error type.
            AppWebError::InvalidResetToken => (
                StatusCode::BAD_REQUEST,
                "This reset link is invalid or has expired.",
            )
                .into_response(),
            // Deliberately a plain 404, not a redirect — unlike
            // `Unauthenticated` (a session-integrity concern), this covers a
            // resource that either doesn't exist or belongs to another
            // tenant, and must not leak which case it is.
            AppWebError::NotFound => (StatusCode::NOT_FOUND, "Not found.").into_response(),
            other => {
                tracing::error!(error = %other, "unhandled web error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong. Please try again.",
                )
                    .into_response()
            }
        }
    }
}
