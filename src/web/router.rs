use axum::extract::DefaultBodyLimit;
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
        .route(
            "/documents",
            get(handlers::documents::list)
                .post(handlers::documents::create)
                .layer(DefaultBodyLimit::max(
                    handlers::documents::MAX_DOCUMENT_BYTES,
                )),
        )
        .route("/documents/new", get(handlers::documents::new_form))
        .route(
            "/documents/collections",
            post(handlers::documents::save_collection),
        )
        .route(
            "/documents/collections/:id/delete",
            post(handlers::documents::delete_collection),
        )
        .route(
            "/documents/collections/:id/rename",
            post(handlers::documents::rename_collection),
        )
        .route(
            "/documents/bulk/delete",
            post(handlers::documents::bulk_delete_confirm),
        )
        .route(
            "/documents/bulk/delete/confirm",
            post(handlers::documents::bulk_delete),
        )
        .route("/documents/bulk/tag", post(handlers::documents::bulk_tag))
        .route("/spending", get(handlers::spending::show))
        .route(
            "/documents/bulk/reprocess_ocr",
            post(handlers::documents::bulk_reprocess_ocr),
        )
        .route(
            "/documents/:id",
            get(handlers::documents::show).post(handlers::documents::update),
        )
        .route(
            "/documents/:id/delete",
            get(handlers::documents::confirm_delete).post(handlers::documents::delete),
        )
        .route(
            "/documents/:id/accept_suggested_date",
            post(handlers::documents::accept_suggested_date),
        )
        .route(
            "/documents/:id/accept_suggested_doc_type",
            post(handlers::documents::accept_suggested_doc_type),
        )
        .route(
            "/documents/:id/accept_suggested_expiry_date",
            post(handlers::documents::accept_suggested_expiry_date),
        )
        .route(
            "/documents/:id/accept_suggested_amount",
            post(handlers::documents::accept_suggested_amount),
        )
        .route(
            "/documents/:id/reprocess_ocr",
            post(handlers::documents::reprocess_ocr),
        )
        .route("/scan", get(handlers::scan::new_scan))
        .route(
            "/profile",
            get(handlers::profile::show).post(handlers::profile::update),
        )
        .route(
            "/profile/picture",
            post(handlers::profile::upload_picture)
                // `BlobStore::stream_upload` also enforces this mid-stream —
                // this layer just rejects an oversized request earlier,
                // before any of it is read.
                .layer(DefaultBodyLimit::max(handlers::profile::MAX_PICTURE_BYTES)),
        )
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
        .route(
            "/forgot-password",
            get(handlers::password_reset::forgot_password_form)
                .post(handlers::password_reset::forgot_password_submit),
        )
        .route(
            "/reset-password",
            get(handlers::password_reset::reset_password_form)
                .post(handlers::password_reset::reset_password_submit),
        )
        .route(
            // Deliberately outside `protected` — the phone loading this
            // never has a session cookie, by design (see
            // `docs/tdr/009_phone_camera_scan_design.md` §1/§3). Tenancy is
            // instead resolved inside `web::handlers::scan` from the
            // path token itself.
            "/scan/:token",
            get(handlers::scan::show_scan_phone)
                .post(handlers::scan::submit_scan)
                .layer(DefaultBodyLimit::max(
                    handlers::documents::MAX_DOCUMENT_BYTES,
                )),
        )
        .route(
            // Also public/tokened, like its capture sibling above —
            // finalizes the session's pages into one PDF document
            // (feature 022).
            "/scan/:token/finish",
            post(handlers::scan::finish_scan),
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
