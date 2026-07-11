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
    #[error("authentication required")]
    Unauthenticated,
}

impl IntoResponse for AppWebError {
    fn into_response(self) -> Response {
        match self {
            AppWebError::Unauthenticated => Redirect::to("/login").into_response(),
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
