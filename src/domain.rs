//! Domain-level identity types shared across the web layer and any future
//! OCR/blob-storage modules that need to scope data by tenant/user.
//!
//! Distinct from raw `Uuid` per CLAUDE.md's type-driven-constraints rule:
//! today every signup mints one tenant per user (`TenantId` and `UserId`
//! carry the same value), but keeping them as separate types means a query
//! signature can't accidentally accept one where the other is required.
//!
//! Each type lives only at its extraction boundary, then is unwrapped to
//! the raw `Uuid` immediately (`tenancy.tenant_id.0`, `id.0`) for internal
//! use and query binds — the newtype's job is only to make the boundary
//! call site unambiguous, not to thread through every helper signature.
//! `TenantId`/`UserId` are produced by the tenancy extractor; `DocumentId`
//! is produced by axum's `Path` extractor on document routes, so it
//! derives `Deserialize` for that (the other two never cross a `Path`).

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(transparent)]
pub struct TenantId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(transparent)]
pub struct UserId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, serde::Deserialize)]
#[sqlx(transparent)]
pub struct DocumentId(pub Uuid);
