//! Shared helper for the small avatar shown in the nav bar on every page,
//! per the "icon-only profile link" mockup — separate from `handlers::profile`
//! since every authenticated page needs it, not just `/profile` itself.

use sqlx::PgPool;
use tracing::Instrument;
use uuid::Uuid;

use crate::blob::BlobStore;
use crate::web::error::AppWebError;

/// Presigned URL for a user's uploaded profile picture. `None` means the nav
/// falls back to the default silhouette icon — whether because no picture
/// was ever uploaded, or because the session's user id no longer matches a
/// row (a missing avatar isn't fatal to any page other than `/profile`
/// itself, so this treats both cases the same rather than erroring).
pub async fn avatar_url(pool: &PgPool, blob: &BlobStore, user_id: Uuid) -> Result<Option<String>, AppWebError> {
    let key = sqlx::query_scalar!("select profile_picture_key from users where id = $1", user_id,)
        .fetch_optional(pool)
        .instrument(tracing::info_span!("db.query"))
        .await?
        .flatten();

    match key {
        Some(key) => Ok(Some(blob.presigned_get_url(&key).await?)),
        None => Ok(None),
    }
}
