//! Domain-level identity types shared across the web layer and any future
//! OCR/blob-storage modules that need to scope data by tenant/user.
//!
//! Distinct from raw `Uuid` per CLAUDE.md's type-driven-constraints rule:
//! today every signup mints one tenant per user (`TenantId` and `UserId`
//! carry the same value), but keeping them as separate types means a query
//! signature can't accidentally accept one where the other is required.

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(transparent)]
pub struct TenantId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(transparent)]
pub struct UserId(pub Uuid);
