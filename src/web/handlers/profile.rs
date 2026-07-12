use axum::extract::{Extension, Form, Multipart, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use tower_sessions::Session;

use crate::web::error::AppWebError;
use crate::web::forms::ProfileForm;
use crate::web::state::AppState;
use crate::web::tenancy::TenantContext;
use crate::web::templates::ProfileTemplate;

/// Also used by the router to size its `DefaultBodyLimit` layer — kept as
/// one constant so the two enforcement points (that layer, and the
/// mid-stream check in `BlobStore::stream_upload` below) can't drift apart.
pub const MAX_PICTURE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct ShowQuery {
    #[serde(default)]
    saved: bool,
}

#[tracing::instrument(skip(state, tenancy, session))]
pub async fn show(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Query(query): Query<ShowQuery>,
) -> Result<Response, AppWebError> {
    let row = sqlx::query!(
        "select first_name, last_name, street_address, city, postcode, country, phone, profile_picture_key
         from users where id = $1",
        tenancy.user_id.0,
    )
    .fetch_one(&state.pool)
    .await;

    let row = match row {
        Ok(row) => row,
        // The session is otherwise valid, but the user id it names has no
        // matching row (e.g. the account was removed out from under a
        // still-live session) — clear the stale session and send them to
        // log in again instead of 500ing on every subsequent visit.
        Err(sqlx::Error::RowNotFound) => {
            tracing::warn!(
                user_id = %tenancy.user_id.0,
                "profile session referenced a missing user; clearing session"
            );
            session.flush().await?;
            return Ok(Redirect::to("/login").into_response());
        }
        Err(e) => return Err(e.into()),
    };

    let picture_url = match row.profile_picture_key {
        Some(key) => Some(state.blob.presigned_get_url(&key).await?),
        None => None,
    };

    Ok(ProfileTemplate {
        active_tab: "profile",
        authenticated: true,
        nav_avatar_url: picture_url.clone(),
        saved: query.saved,
        first_name: row.first_name.unwrap_or_default(),
        last_name: row.last_name.unwrap_or_default(),
        street_address: row.street_address.unwrap_or_default(),
        city: row.city.unwrap_or_default(),
        postcode: row.postcode.unwrap_or_default(),
        country: row.country.unwrap_or_default(),
        phone: row.phone.unwrap_or_default(),
        picture_url,
    }
    .into_response())
}

#[tracing::instrument(skip(state, tenancy, form))]
pub async fn update(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Form(form): Form<ProfileForm>,
) -> Result<Response, AppWebError> {
    sqlx::query!(
        "update users set first_name = $2, last_name = $3, street_address = $4, city = $5, \
         postcode = $6, country = $7, phone = $8 where id = $1",
        tenancy.user_id.0,
        form.first_name.into_option(),
        form.last_name.into_option(),
        form.street_address.into_option(),
        form.city.into_option(),
        form.postcode.into_option(),
        form.country.into_option(),
        form.phone.into_option(),
    )
    .execute(&state.pool)
    .await?;

    Ok(Redirect::to("/profile?saved=true").into_response())
}

#[tracing::instrument(skip(state, tenancy, multipart))]
pub async fn upload_picture(
    tenancy: TenantContext,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, AppWebError> {
    let Some(field) = multipart.next_field().await? else {
        return Ok((StatusCode::BAD_REQUEST, "no file provided").into_response());
    };

    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();
    if !content_type.starts_with("image/") {
        return Ok((StatusCode::BAD_REQUEST, "only image uploads are accepted").into_response());
    }

    let key = format!("profile-pictures/{}", tenancy.user_id.0);
    state
        .blob
        .stream_upload(&key, &content_type, field, MAX_PICTURE_BYTES)
        .await?;

    sqlx::query!(
        "update users set profile_picture_key = $2 where id = $1",
        tenancy.user_id.0,
        key,
    )
    .execute(&state.pool)
    .await?;

    Ok(Redirect::to("/profile?saved=true").into_response())
}
