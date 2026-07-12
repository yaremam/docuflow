use axum::extract::{Extension, Form, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use tower_sessions::Session;
use tracing::Instrument;
use uuid::Uuid;

use crate::web::error::AppWebError;
use crate::web::forms::{EmailAddress, Password, ResetToken};
use crate::web::nav;
use crate::web::state::AppState;
use crate::web::tenancy::{MaybeTenantContext, SESSION_USER_ID_KEY};
use crate::web::templates::{ForgotPasswordTemplate, ResetPasswordTemplate};

const RESET_TOKEN_TTL_HOURS: i64 = 1;

#[derive(Debug, Deserialize)]
pub struct ForgotPasswordQuery {
    #[serde(default)]
    sent: bool,
}

#[tracing::instrument(skip(tenancy, state))]
pub async fn forgot_password_form(
    MaybeTenantContext(tenancy): MaybeTenantContext,
    State(state): State<AppState>,
    Query(query): Query<ForgotPasswordQuery>,
) -> Result<ForgotPasswordTemplate, AppWebError> {
    let nav_avatar_url = match &tenancy {
        Some(t) => nav::avatar_url(&state.pool, &state.blob, t.user_id.0).await?,
        None => None,
    };

    Ok(ForgotPasswordTemplate {
        active_tab: "",
        authenticated: tenancy.is_some(),
        nav_avatar_url,
        sent: query.sent,
    })
}

#[derive(Debug, serde::Deserialize)]
pub struct ForgotPasswordForm {
    pub email: EmailAddress,
}

#[tracing::instrument(skip(state, form))]
pub async fn forgot_password_submit(
    State(state): State<AppState>,
    Form(form): Form<ForgotPasswordForm>,
) -> Result<Response, AppWebError> {
    let user = sqlx::query!("select id from users where email = $1", form.email.as_str())
        .fetch_optional(&state.pool)
        .await?;

    // Anti-enumeration (mirrors signup/login): the response is identical
    // whether or not the email matches an account, so this endpoint can't be
    // used to test which emails have accounts.
    if let Some(user) = user {
        let token = ResetToken::generate();
        let token_hash = token.hash();
        let expires_at = time::OffsetDateTime::now_utc() + time::Duration::hours(RESET_TOKEN_TTL_HOURS);

        sqlx::query!(
            "insert into password_reset_tokens (id, user_id, token_hash, expires_at) \
             values ($1, $2, $3, $4)",
            Uuid::new_v4(),
            user.id,
            token_hash,
            expires_at,
        )
        .execute(&state.pool)
        .await?;

        let reset_url = format!(
            "{}/reset-password?token={}",
            state.app_base_url,
            token.as_str()
        );

        // Sending is fire-and-forget from the caller's perspective (a
        // transient mail-relay failure shouldn't turn into a response that's
        // distinguishable from the "email not found" path — it's only
        // logged, never surfaced), so there's no reason to make the request
        // wait on the SMTP round trip. Spawned onto the current span so the
        // eventual success/failure log still correlates with this request.
        let mailer = state.mailer.clone();
        let to = form.email.as_str().to_string();
        tokio::spawn(
            async move {
                if let Err(error) = mailer.send_reset_email(&to, &reset_url).await {
                    tracing::error!(%error, "failed to send password reset email");
                }
            }
            .instrument(tracing::Span::current()),
        );
    }

    Ok(Redirect::to("/forgot-password?sent=true").into_response())
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordQuery {
    pub token: ResetToken,
}

#[tracing::instrument(skip(state, tenancy, query))]
pub async fn reset_password_form(
    MaybeTenantContext(tenancy): MaybeTenantContext,
    State(state): State<AppState>,
    Query(query): Query<ResetPasswordQuery>,
) -> Result<ResetPasswordTemplate, AppWebError> {
    let valid = match find_valid_token(&state, query.token.hash()).await {
        Ok(()) => true,
        Err(AppWebError::InvalidResetToken) => false,
        Err(other) => return Err(other),
    };

    let nav_avatar_url = match &tenancy {
        Some(t) => nav::avatar_url(&state.pool, &state.blob, t.user_id.0).await?,
        None => None,
    };

    Ok(ResetPasswordTemplate {
        active_tab: "",
        authenticated: tenancy.is_some(),
        nav_avatar_url,
        valid,
        token: query.token.as_str().to_string(),
    })
}

/// Existence check only — `reset_password_form` just needs to know whether
/// the token is valid to decide which state to render. `reset_password_submit`
/// below re-validates with its own `for update` query instead of calling
/// this, since it additionally needs to row-lock the token for the rest of
/// its transaction.
async fn find_valid_token(state: &AppState, token_hash: String) -> Result<(), AppWebError> {
    let row = sqlx::query!(
        "select user_id from password_reset_tokens \
         where token_hash = $1 and used_at is null and expires_at > now()",
        token_hash,
    )
    .fetch_optional(&state.pool)
    .await?;

    row.map(|_| ()).ok_or(AppWebError::InvalidResetToken)
}

#[derive(Debug, serde::Deserialize)]
pub struct ResetPasswordForm {
    pub token: ResetToken,
    pub password: Password,
}

#[tracing::instrument(skip(state, session, form))]
pub async fn reset_password_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Form(form): Form<ResetPasswordForm>,
) -> Result<Response, AppWebError> {
    let token_hash = form.token.hash();

    // Hash the new password before opening the transaction — Argon2 is
    // deliberately slow, and there's no reason to hold the token row's lock
    // (below) for the duration of that work.
    let password_hash = form.password.into_hash().await?;

    let mut tx = state.pool.begin().await?;

    // Row-lock the token for the rest of the transaction so a second,
    // concurrent submit of the same still-unused token can't also pass
    // validation before this one marks it used.
    let row = sqlx::query!(
        "select user_id from password_reset_tokens \
         where token_hash = $1 and used_at is null and expires_at > now() \
         for update",
        token_hash,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        return Ok((
            StatusCode::BAD_REQUEST,
            ResetPasswordTemplate {
                active_tab: "",
                authenticated: false,
                nav_avatar_url: None,
                valid: false,
                token: form.token.as_str().to_string(),
            },
        )
            .into_response());
    };

    sqlx::query!(
        "update users set password_hash = $2 where id = $1",
        row.user_id,
        password_hash,
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "update password_reset_tokens set used_at = now() where token_hash = $1",
        token_hash,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // The token already proved control of the account's email — equivalent
    // trust to a fresh login — so establish a session immediately rather
    // than sending the user back through /login. Soft-fail to /login on a
    // session-store error exactly like `signup_submit` already does.
    if session.insert(SESSION_USER_ID_KEY, row.user_id).await.is_err() {
        tracing::error!(
            user_id = %row.user_id,
            "password reset succeeded but establishing a session failed; sending the user to log in"
        );
        return Ok(Redirect::to("/login").into_response());
    }

    Ok(Redirect::to("/documents").into_response())
}
