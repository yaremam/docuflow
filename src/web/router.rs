use axum::http::header::CACHE_CONTROL;
use axum::http::HeaderValue;
use axum::routing::get;
use axum::Router;
use tower::ServiceBuilder;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::web::handlers;

pub fn app() -> Router {
    let pages = Router::new()
        .route("/", get(handlers::landing::show))
        .route("/health", get(handlers::health::show))
        .route(
            "/signup",
            get(handlers::auth::signup_form).post(handlers::auth::signup_submit),
        )
        .route(
            "/login",
            get(handlers::auth::login_form).post(handlers::auth::login_submit),
        )
        .layer(TraceLayer::new_for_http());

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
