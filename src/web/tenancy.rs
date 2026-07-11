//! Axum extractor that proves a request carries a valid session and scopes
//! it to a tenant/user, per CLAUDE.md's multi-tenancy rule: "every incoming
//! HTTP request must extract a `TenantId` and `UserId`... inject `tenant.id`
//! and `user.id` into the active OpenTelemetry context using OTel Baggage."
//!
//! In the current 1:1 tenancy model, `tenant_id` and `user_id` share the
//! same underlying UUID (the value stored under the session's `"user_id"`
//! key at signup/login) — kept as distinct types so query signatures stay
//! accurate once a tenant can hold more than one user.
//!
//! Rather than attaching an `opentelemetry::Context` guard (which is `!Send`
//! and unsound to hold across the `.await` points of an async handler on a
//! multi-threaded runtime — the extracted value would need to survive to
//! the end of the handler, which can resume on a different worker thread),
//! `tenant.id`/`user.id` are recorded directly as attributes on the request's
//! active span via `OpenTelemetrySpanExt::set_attribute`. This is Send-safe
//! (no thread-local guard involved) and is what actually surfaces as visible
//! span tags in Jaeger, which raw Baggage propagation alone would not do
//! without an additional baggage-to-span-attribute processor.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use tower_sessions::Session;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::domain::{TenantId, UserId};
use crate::web::error::AppWebError;

/// The session key that stores the authenticated user's id, set at
/// signup/login (`src/web/handlers/auth.rs`) and read here. Shared as a
/// constant so the three call sites can't drift out of sync via a typo.
pub const SESSION_USER_ID_KEY: &str = "user_id";

#[derive(Debug, Clone, Copy)]
pub struct TenantContext {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for TenantContext
where
    S: Send + Sync,
{
    type Rejection = AppWebError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // `tower_sessions::Session` doesn't implement `axum::FromRequestParts`
        // in a version compatible with our pinned `axum = "0.7"` (its
        // "axum-core" feature currently targets axum 0.8's axum-core 0.5).
        // `SessionManagerLayer` inserts the `Session` into request extensions
        // unconditionally, regardless of that feature, so we read it from
        // there directly instead.
        let session = parts
            .extensions
            .get::<Session>()
            .cloned()
            .ok_or(AppWebError::Unauthenticated)?;

        let user_id: Uuid = session
            .get(SESSION_USER_ID_KEY)
            .await?
            .ok_or(AppWebError::Unauthenticated)?;

        let user_id = UserId(user_id);
        let tenant_id = TenantId(user_id.0);

        let span = tracing::Span::current();
        span.set_attribute("tenant.id", tenant_id.0.to_string());
        span.set_attribute("user.id", user_id.0.to_string());

        Ok(Self {
            tenant_id,
            user_id,
        })
    }
}
