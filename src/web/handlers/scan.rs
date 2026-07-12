//! Phone-camera scan handoff (feature 009): a desktop page shows a QR code,
//! a phone (never logged in) loads the QR-encoded URL and uploads a photo,
//! and the desktop picks up the resulting document. See
//! `docs/tdr/009_phone_camera_scan_design.md` for the full design rationale.

use axum::extract::{Multipart, Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use uuid::Uuid;

use crate::web::error::AppWebError;
use crate::web::forms::ScanToken;
use crate::web::handlers::documents::{bad_request, insert_document_and_queue_ocr, stream_document_to_blob};
use crate::web::nav;
use crate::web::state::AppState;
use crate::web::tenancy::TenantContext;
use crate::web::templates::{ScanNewTemplate, ScanPhoneState, ScanPhoneTemplate};

const SCAN_SESSION_TTL_MINUTES: i64 = 10;

/// Only the two content types a phone camera's native capture UI actually
/// produces — a subset of `documents::OCR_ELIGIBLE_CONTENT_TYPES` (feature
/// 008's desktop upload also accepts TIFF/WEBP/PDF, none of which apply
/// here).
const PHONE_ACCEPTED_CONTENT_TYPES: &[&str] = &["image/jpeg", "image/png"];

/// Renders `url` as inline SVG markup (no XML prolog — this is embedded
/// directly inside an HTML document, not served as a standalone `.svg`
/// file), colored with the page's own CSS custom properties so it follows
/// the active light/dark theme like everything else on the page.
fn qr_svg_markup(url: &str) -> Result<String, qrcode::types::QrError> {
    let code = qrcode::QrCode::new(url.as_bytes())?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .dark_color(qrcode::render::svg::Color("var(--ink)"))
        .light_color(qrcode::render::svg::Color("var(--paper-raised)"))
        .build();
    Ok(svg.split_once("?>").map(|(_, rest)| rest.to_string()).unwrap_or(svg))
}

/// Mints a fresh scan session and redirects to it — used both for a bare
/// `GET /scan` and for a `GET /scan?token=...` whose token turned out to be
/// unknown or expired, so an unusable/stale QR code is silently replaced
/// with a fresh one rather than shown as an error to the desktop user (who,
/// unlike the phone side, isn't the one who might be replaying a stale
/// link).
async fn mint_and_redirect(state: &AppState, tenancy: TenantContext) -> Result<Response, AppWebError> {
    let token = ScanToken::generate();
    let expires_at = time::OffsetDateTime::now_utc() + time::Duration::minutes(SCAN_SESSION_TTL_MINUTES);

    sqlx::query!(
        "insert into scan_sessions (id, tenant_id, user_id, token_hash, expires_at)
         values ($1, $2, $3, $4, $5)",
        Uuid::new_v4(),
        tenancy.tenant_id.0,
        tenancy.user_id.0,
        token.hash(),
        expires_at,
    )
    .execute(&state.pool)
    .await?;

    Ok(Redirect::to(&format!("/scan?token={}", token.as_str())).into_response())
}

#[derive(Debug, Deserialize)]
pub struct ScanQuery {
    token: Option<ScanToken>,
}

#[tracing::instrument(skip(state, tenancy, query))]
pub async fn new_scan(
    tenancy: TenantContext,
    State(state): State<AppState>,
    Query(query): Query<ScanQuery>,
) -> Result<Response, AppWebError> {
    let Some(token) = query.token else {
        return mint_and_redirect(&state, tenancy).await;
    };

    let token_hash = token.hash();
    let row = sqlx::query!(
        "select status, document_id from scan_sessions
         where token_hash = $1 and tenant_id = $2 and user_id = $3
           and (status = 'captured' or expires_at > now())",
        token_hash,
        tenancy.tenant_id.0,
        tenancy.user_id.0,
    )
    .fetch_optional(&state.pool)
    .await?;

    match row {
        Some(row) if row.status == "captured" => {
            let document_id = row.document_id.ok_or(AppWebError::InconsistentScanSession)?;
            Ok(Redirect::to(&format!("/documents/{document_id}?uploaded=true")).into_response())
        }
        Some(_pending) => {
            let nav_avatar_url = nav::avatar_url(&state.pool, &state.blob, tenancy.user_id.0).await?;
            let scan_url = format!("{}/scan/{}", state.app_base_url, token.as_str());
            let qr_svg = qr_svg_markup(&scan_url)?;
            Ok(ScanNewTemplate {
                active_tab: "documents",
                authenticated: true,
                nav_avatar_url,
                scan_url,
                qr_svg,
            }
            .into_response())
        }
        None => mint_and_redirect(&state, tenancy).await,
    }
}

#[tracing::instrument(skip(state))]
pub async fn show_scan_phone(
    State(state): State<AppState>,
    Path(token): Path<ScanToken>,
) -> Result<ScanPhoneTemplate, AppWebError> {
    let token_hash = token.hash();
    let row = sqlx::query!(
        "select status from scan_sessions
         where token_hash = $1 and (status = 'captured' or expires_at > now())",
        token_hash,
    )
    .fetch_optional(&state.pool)
    .await?;

    let phone_state = match row {
        Some(row) if row.status == "captured" => ScanPhoneState::Captured,
        Some(row) if row.status == "pending" => ScanPhoneState::Capture,
        _ => ScanPhoneState::Invalid,
    };

    Ok(ScanPhoneTemplate {
        authenticated: false,
        active_tab: "",
        nav_avatar_url: None,
        state: phone_state,
        token: token.as_str().to_string(),
    })
}

#[tracing::instrument(skip(state, multipart))]
pub async fn submit_scan(
    State(state): State<AppState>,
    Path(token): Path<ScanToken>,
    mut multipart: Multipart,
) -> Result<Response, AppWebError> {
    let token_hash = token.hash();
    let row = sqlx::query!(
        "select id, tenant_id, user_id from scan_sessions
         where token_hash = $1 and status = 'pending' and expires_at > now()",
        token_hash,
    )
    .fetch_optional(&state.pool)
    .await?;

    let Some(row) = row else {
        return Ok(bad_request("this scan code is invalid or has expired"));
    };

    // Each field is handled entirely within its own loop iteration (never
    // stored past it) — matching `documents::create`'s existing pattern —
    // since a `Field<'_>` borrows `multipart` mutably, and stashing one
    // outside the loop to use after it ends doesn't satisfy the borrow
    // checker across further (even unreached) loop iterations.
    let mut captured_document_id = None;
    while let Some(field) = multipart.next_field().await? {
        if field.name() != Some("photo") {
            continue;
        }

        let content_type = field.content_type().unwrap_or("application/octet-stream").to_string();
        let original_filename = field.file_name().unwrap_or("scan.jpg").to_string();
        if !PHONE_ACCEPTED_CONTENT_TYPES.contains(&content_type.as_str()) {
            return Ok(bad_request("unsupported file type"));
        }

        let document_id = Uuid::new_v4();
        let (blob_key, file_size_bytes) =
            stream_document_to_blob(&state, row.user_id, document_id, &content_type, field).await?;
        insert_document_and_queue_ocr(
            &state,
            row.tenant_id,
            row.user_id,
            document_id,
            blob_key,
            original_filename,
            content_type,
            file_size_bytes,
            None,
            Vec::new(),
            None,
        )
        .await?;
        captured_document_id = Some(document_id);
        break;
    }

    let Some(document_id) = captured_document_id else {
        return Ok(bad_request("no photo provided"));
    };

    // Re-checks `status = 'pending'` so a second, concurrent submit of the
    // same token can't also flip this row to `'captured'` — the ordinary
    // sequential-reuse case (submit once, then submit again) is fully
    // covered by this; a genuine race between two simultaneous submits of
    // the same one-time QR code is an accepted, undefended edge case at
    // this project's scale (same tradeoff already made for OCR crash
    // recovery in TDR 008).
    sqlx::query!(
        "update scan_sessions set status = 'captured', document_id = $2
         where id = $1 and status = 'pending'",
        row.id,
        document_id,
    )
    .execute(&state.pool)
    .await?;

    Ok(ScanPhoneTemplate {
        authenticated: false,
        active_tab: "",
        nav_avatar_url: None,
        state: ScanPhoneState::Captured,
        token: token.as_str().to_string(),
    }
    .into_response())
}
