use axum::http::header::CACHE_CONTROL;
use axum::http::HeaderValue;
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceBuilder;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::PostgresStore;

use crate::web::handlers;
use crate::web::state::AppState;
use crate::web::tenancy::TenantContext;

pub fn app(state: AppState, session_layer: SessionManagerLayer<PostgresStore>) -> Router {
    // Every route added here requires a valid session: `route_layer` runs
    // the `TenantContext` extractor as router-level middleware and rejects
    // before the handler is ever called, so this is structural enforcement,
    // not just a per-handler convention — a future protected route added to
    // this group can't accidentally ship unauthenticated even if its own
    // handler forgets to also declare `TenantContext` as a parameter (which
    // `logout` still does, separately, to read the tenant/user id for
    // logging — that's a second, cheap extraction, not the auth check).
    let protected = Router::new()
        .route("/logout", post(handlers::auth::logout))
        .route_layer(middleware::from_extractor::<TenantContext>());

    let pages = Router::new()
        .route("/", get(handlers::landing::show))
        .route("/welcome", get(handlers::landing::welcome))
        .route("/health", get(handlers::health::show))
        .route(
            "/signup",
            get(handlers::auth::signup_form).post(handlers::auth::signup_submit),
        )
        .route(
            "/login",
            get(handlers::auth::login_form).post(handlers::auth::login_submit),
        )
        .merge(protected)
        .layer(TraceLayer::new_for_http())
        .layer(session_layer)
        .with_state(state);

    // Static assets never change at a given URL, so they're kept out of the
    // page-level TraceLayer above (no point exporting a span per CSS/font
    // fetch) and are cached long-term by the browser instead of
    // re-validated on every navigation.
    let assets = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ))
        .service(ServeDir::new("static"));

    pages.nest_service("/static", assets)
}
