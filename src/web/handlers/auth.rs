use axum::extract::{Extension, Form, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use tower_sessions::Session;
use uuid::Uuid;

use crate::web::error::AppWebError;
use crate::web::forms::Credentials;
use crate::web::state::AppState;
use crate::web::tenancy::{MaybeTenantContext, TenantContext, SESSION_USER_ID_KEY};
use crate::web::templates::{LoginTemplate, SignupTemplate};

const INVALID_CREDENTIALS: &str = "Invalid email or password.";
const SIGNUP_FAILED: &str = "We couldn't create that account — check your details and try again.";

#[tracing::instrument(skip(tenancy))]
pub async fn signup_form(MaybeTenantContext(tenancy): MaybeTenantContext) -> SignupTemplate {
    SignupTemplate {
        active_tab: "signup",
        authenticated: tenancy.is_some(),
        error: None,
    }
}

#[tracing::instrument(skip(state, session, form))]
pub async fn signup_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Form(form): Form<Credentials>,
) -> Result<Response, AppWebError> {
    // Hash unconditionally, before touching the database, so the request
    // takes roughly the same time whether or not the email turns out to be
    // taken — a duplicate-email response shouldn't be distinguishable from
    // any other signup-rejection reason by timing (AC-2).
    let password_hash = form.password.into_hash().await?;

    let user_id = Uuid::new_v4();

    // 1:1 tenancy: the new user's tenant id is minted alongside their own id.
    // A data-modifying CTE keeps the tenant+user insert atomic as a single
    // round trip, rather than an explicit multi-statement transaction.
    let inserted = sqlx::query!(
        "with ins_tenant as (insert into tenants (id) values ($1)) \
         insert into users (id, tenant_id, email, password_hash) values ($1, $1, $2, $3)",
        user_id,
        form.email.as_str(),
        password_hash,
    )
    .execute(&state.pool)
    .await;

    match inserted {
        Ok(_) => {
            // The account is already durably committed — if establishing a
            // session now fails (a transient session-store issue), don't
            // surface a raw error that leaves the user stuck not knowing
            // their account exists: send them to log in instead, since the
            // account is real and login will succeed normally.
            if session.insert(SESSION_USER_ID_KEY, user_id).await.is_err() {
                tracing::error!(
                    %user_id,
                    "signup succeeded but establishing a session failed; sending the user to log in"
                );
                return Ok(Redirect::to("/login").into_response());
            }
            Ok(Redirect::to("/welcome").into_response())
        }
        Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => Ok((
            StatusCode::CONFLICT,
            SignupTemplate {
                active_tab: "signup",
                authenticated: false,
                error: Some(SIGNUP_FAILED),
            },
        )
            .into_response()),
        Err(e) => Err(e.into()),
    }
}

#[tracing::instrument(skip(tenancy))]
pub async fn login_form(MaybeTenantContext(tenancy): MaybeTenantContext) -> LoginTemplate {
    LoginTemplate {
        active_tab: "login",
        authenticated: tenancy.is_some(),
        error: None,
    }
}

#[tracing::instrument(skip(state, session, form))]
pub async fn login_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Form(form): Form<Credentials>,
) -> Result<Response, AppWebError> {
    let invalid = || {
        (
            StatusCode::UNAUTHORIZED,
            LoginTemplate {
                active_tab: "login",
                authenticated: false,
                error: Some(INVALID_CREDENTIALS),
            },
        )
            .into_response()
    };

    let row = sqlx::query!(
        "select id, password_hash from users where email = $1",
        form.email.as_str(),
    )
    .fetch_optional(&state.pool)
    .await?;

    // Wrong password and unknown email must be indistinguishable to the
    // caller (AC-3) — in body/status AND in timing. For a known user we
    // verify against their real hash; for an unknown user there's no real
    // hash to check against, so we still run one full Argon2 operation
    // (hashing the submitted password against a throwaway salt, result
    // discarded) so both paths cost roughly the same before either falls
    // through to the same `invalid()` response.
    let password = form.password;
    let verified = match &row {
        Some(row) => password.matches_hash(row.password_hash.clone()).await?,
        None => {
            password.into_hash().await?;
            false
        }
    };

    let Some(row) = row else {
        return Ok(invalid());
    };

    if !verified {
        return Ok(invalid());
    }

    session.insert(SESSION_USER_ID_KEY, row.id).await?;
    Ok(Redirect::to("/welcome?returning=true").into_response())
}

#[tracing::instrument(skip(session, tenancy))]
pub async fn logout(
    Extension(session): Extension<Session>,
    tenancy: TenantContext,
) -> Result<Response, AppWebError> {
    tracing::info!(tenant_id = %tenancy.tenant_id.0, user_id = %tenancy.user_id.0, "logging out");
    session.flush().await?;
    Ok(Redirect::to("/").into_response())
}
