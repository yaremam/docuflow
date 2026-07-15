//! Phone-camera scan handoff (feature 009), multi-page since feature 022:
//! a desktop page shows a QR code, a phone (never logged in) loads the
//! QR-encoded URL and uploads one photo per page, then finishes the session
//! — all captured pages become one PDF document. See
//! `docs/tdr/009_phone_camera_scan_design.md` and
//! `docs/tdr/022_multipage_scan_design.md` for the design rationale.

use axum::extract::{Multipart, Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use uuid::Uuid;

use crate::pdf_assemble::{self, PageImage};
use crate::web::error::AppWebError;
use crate::web::forms::ScanToken;
use crate::web::handlers::documents::{bad_request, insert_document_and_queue_ocr, MAX_DOCUMENT_BYTES};
use crate::web::nav;
use crate::web::state::AppState;
use crate::web::tenancy::TenantContext;
use crate::web::templates::{ScanNewTemplate, ScanPhoneState, ScanPhoneTemplate, ScanProgressTemplate};

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
        r#"select s.status, s.document_id,
                  (select count(*) from scan_pages p where p.scan_session_id = s.id) as "page_count!"
           from scan_sessions s
           where s.token_hash = $1 and s.tenant_id = $2 and s.user_id = $3
             and (s.status = 'captured' or s.expires_at > now())"#,
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
        Some(row) if row.status == "capturing" => {
            let nav_avatar_url = nav::avatar_url(&state.pool, &state.blob, tenancy.user_id.0).await?;
            Ok(ScanProgressTemplate {
                active_tab: "documents",
                authenticated: true,
                nav_avatar_url,
                page_count: row.page_count,
            }
            .into_response())
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

fn phone_template(state: ScanPhoneState, token: &ScanToken) -> ScanPhoneTemplate {
    ScanPhoneTemplate {
        authenticated: false,
        active_tab: "",
        nav_avatar_url: None,
        state,
        token: token.as_str().to_string(),
    }
}

#[tracing::instrument(skip(state))]
pub async fn show_scan_phone(
    State(state): State<AppState>,
    Path(token): Path<ScanToken>,
) -> Result<ScanPhoneTemplate, AppWebError> {
    let token_hash = token.hash();
    let row = sqlx::query!(
        r#"select s.status,
                  (select count(*) from scan_pages p where p.scan_session_id = s.id) as "page_count!"
           from scan_sessions s
           where s.token_hash = $1 and (s.status = 'captured' or s.expires_at > now())"#,
        token_hash,
    )
    .fetch_optional(&state.pool)
    .await?;

    let phone_state = match row {
        Some(row) if row.status == "captured" => ScanPhoneState::Captured(row.page_count),
        Some(row) if row.status == "capturing" => ScanPhoneState::Capturing(row.page_count),
        Some(row) if row.status == "pending" => ScanPhoneState::Capture,
        _ => ScanPhoneState::Invalid,
    };

    Ok(phone_template(phone_state, &token))
}

/// Appends one captured page to the session (feature 022 — this no longer
/// creates a document; `finish_scan` below does, once, for all pages).
#[tracing::instrument(skip(state, multipart))]
pub async fn submit_scan(
    State(state): State<AppState>,
    Path(token): Path<ScanToken>,
    mut multipart: Multipart,
) -> Result<Response, AppWebError> {
    let token_hash = token.hash();
    let row = sqlx::query!(
        "select id, tenant_id, user_id from scan_sessions
         where token_hash = $1 and status in ('pending', 'capturing') and expires_at > now()",
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
    let mut captured_page_number = None;
    while let Some(field) = multipart.next_field().await? {
        if field.name() != Some("photo") {
            continue;
        }

        let content_type = field.content_type().unwrap_or("application/octet-stream").to_string();
        if !PHONE_ACCEPTED_CONTENT_TYPES.contains(&content_type.as_str()) {
            return Ok(bad_request("unsupported file type"));
        }

        // The page's own id keys the blob (not the page number, which is
        // only assigned inside the insert below) — order lives in the
        // `page_number` column alone.
        let page_id = Uuid::new_v4();
        let blob_key = format!("scan-pages/{}/{}/{}", row.user_id, row.id, page_id);
        let file_size_bytes = state
            .blob
            .stream_upload(&blob_key, &content_type, field, MAX_DOCUMENT_BYTES)
            .await? as i64;

        // Two phones racing this max()+1 can collide on the unique
        // (session, page_number) pair and one of them errors — the same
        // accepted simultaneous-submit edge TDR 009 documented; sequential
        // captures (the actual flow) are fully ordered.
        let page_number = sqlx::query_scalar!(
            r#"insert into scan_pages (id, scan_session_id, page_number, blob_key, content_type, file_size_bytes)
               values ($1, $2, (select coalesce(max(page_number), 0) + 1 from scan_pages where scan_session_id = $2), $3, $4, $5)
               returning page_number"#,
            page_id,
            row.id,
            blob_key,
            content_type,
            file_size_bytes,
        )
        .fetch_one(&state.pool)
        .await?;
        captured_page_number = Some(page_number);
        break;
    }

    let Some(page_number) = captured_page_number else {
        return Ok(bad_request("no photo provided"));
    };

    // Sliding expiry (AC-5): every page buys the session a fresh full TTL,
    // so a slow multi-page scan isn't cut off by the fuse lit at QR-mint
    // time. Guarded on status so a concurrently-finalized session isn't
    // reopened.
    sqlx::query!(
        "update scan_sessions
         set status = 'capturing', expires_at = now() + make_interval(mins => $2)
         where id = $1 and status in ('pending', 'capturing')",
        row.id,
        SCAN_SESSION_TTL_MINUTES as i32,
    )
    .execute(&state.pool)
    .await?;

    Ok(phone_template(ScanPhoneState::Capturing(page_number as i64), &token).into_response())
}

/// Finalizes the session (feature 022): assembles all captured pages, in
/// order, into one PDF document via the same ingest path desktop uploads
/// use, then marks the session captured.
#[tracing::instrument(skip(state))]
pub async fn finish_scan(
    State(state): State<AppState>,
    Path(token): Path<ScanToken>,
) -> Result<Response, AppWebError> {
    let token_hash = token.hash();
    let row = sqlx::query!(
        r#"select s.id, s.tenant_id, s.user_id, s.status,
                  (select count(*) from scan_pages p where p.scan_session_id = s.id) as "page_count!"
           from scan_sessions s
           where s.token_hash = $1 and (s.status = 'captured' or s.expires_at > now())"#,
        token_hash,
    )
    .fetch_optional(&state.pool)
    .await?;

    let Some(row) = row else {
        return Ok(bad_request("this scan code is invalid or has expired"));
    };
    // The common double-tap: already finalized — re-render the confirmation
    // rather than erroring or assembling a second document (TDR 022 §3).
    if row.status == "captured" {
        return Ok(phone_template(ScanPhoneState::Captured(row.page_count), &token).into_response());
    }
    if row.status != "capturing" || row.page_count == 0 {
        return Ok(bad_request("no pages captured yet — take at least one photo first"));
    }

    let pages = sqlx::query!(
        "select blob_key, content_type from scan_pages
         where scan_session_id = $1 order by page_number",
        row.id,
    )
    .fetch_all(&state.pool)
    .await?;

    let mut page_images = Vec::with_capacity(pages.len());
    for page in &pages {
        page_images.push(PageImage {
            bytes: state.blob.get_object(&page.blob_key).await?,
            content_type: page.content_type.clone(),
        });
    }

    let pdf_bytes = match pdf_assemble::images_to_pdf(&page_images) {
        Ok(bytes) => bytes,
        Err(error) => {
            // A page that won't parse as an image (mis-declared content
            // type, corrupt upload) is the submitter's problem, not a
            // server fault — same 400 discipline as the capture path.
            tracing::warn!(scan_session_id = %row.id, %error, "scan pages could not be assembled into a PDF");
            return Ok(bad_request("one of the captured pages couldn't be read — start a new scan"));
        }
    };

    let document_id = Uuid::new_v4();
    let blob_key = format!("documents/{}/{}", row.user_id, document_id);
    let file_size_bytes = state.blob.upload_bytes(&blob_key, "application/pdf", pdf_bytes).await? as i64;
    insert_document_and_queue_ocr(
        &state,
        row.tenant_id,
        row.user_id,
        document_id,
        blob_key,
        format!("phone-scan-{}-pages.pdf", row.page_count),
        "application/pdf".to_string(),
        file_size_bytes,
        None,
        Vec::new(),
        None,
    )
    .await?;

    // Guarded finalize, mirroring 009: only one finish can flip
    // 'capturing' → 'captured'. A genuinely simultaneous pair of finishes
    // remains the same accepted, undefended race 009 documented for
    // double-submit (TDR 022 §3).
    sqlx::query!(
        "update scan_sessions set status = 'captured', document_id = $2
         where id = $1 and status = 'capturing'",
        row.id,
        document_id,
    )
    .execute(&state.pool)
    .await?;

    // Best-effort cleanup: the pages now live inside the document's PDF.
    // The `scan_pages` rows stay — they're how the captured screens know
    // the page count.
    for page in &pages {
        if let Err(error) = state.blob.delete_object(&page.blob_key).await {
            tracing::warn!(scan_session_id = %row.id, %error, "failed to delete a finalized scan-page blob");
        }
    }

    Ok(phone_template(ScanPhoneState::Captured(row.page_count), &token).into_response())
}
