# TDR 009: Phone-Camera Scan Handoff

## 1. Context & Architectural Requirements
Feature 008 built the upload/OCR pipeline assuming the file already sits on
the device the user is browsing from. This feature adds a second entry
point into that same pipeline — a photo taken on a phone that was never
logged in to DocuFlow — without duplicating the validation, size-cap,
blob-storage, or OCR-spawn logic feature 008 already established, and
without weakening tenant isolation (CLAUDE.md §2) just because the
uploading device has no session cookie.

The phone side is deliberately the one place in this app where a request
reaches a document-creating handler without going through the normal
`TenantContext` extractor (which requires a `tower-sessions` cookie) —
that's flagged explicitly in §3 below since it's a bend, not a violation,
of CLAUDE.md's "every incoming HTTP request must extract a `TenantId` and
`UserId` via an Axum extractor" rule.

## 2. Alternatives Evaluated

### Alternative A: Piggyback the phone on the desktop's session cookie
- **Pros:** Would reuse `TenantContext` unmodified — no new auth path.
- **Cons:** Not actually possible — the phone is a different device/browser
  with no cookie jar shared with the desktop. Ruled out immediately.

### Alternative B: Dedicated single-use, hashed, expiring scan token (chosen)
- **Pros:** Directly mirrors the already-established, already-reviewed
  `password_reset_tokens` pattern (hash at rest, single-use, `expires_at`
  column, no plaintext token ever persisted) — no new security idiom for
  this codebase. A `scan_sessions` row carries `tenant_id`/`user_id`
  captured at `GET /scan` creation time (while the desktop *does* have a
  normal authenticated session), so the later phone-side requests resolve
  tenancy from that row instead of a cookie.
- **Cons:** A second token table with near-identical shape to
  `password_reset_tokens` rather than a generalized "single-use token"
  abstraction — accepted, since generalizing two call sites into a shared
  table now would guess at a schema neither concretely needs yet.

**Chosen: Alternative B.**

---

### Alternative C: WebRTC `getUserMedia` + in-page `<canvas>` capture
- **Pros:** Live preview and retake before upload without leaving the page;
  finer control over resolution/compression client-side.
- **Cons:** Would be the first non-trivial client-side JavaScript this
  project ships — everything today is server-rendered HTML with, at most,
  a `<meta http-equiv="refresh">` (see `document_show.html`'s OCR-status
  polling). Camera permission prompts and codec/format quirks vary enough
  across mobile browsers to be a meaningfully larger surface than this
  feature's scope calls for.

### Alternative D: Native `<input type="file" accept="image/*" capture="environment">` (chosen)
- **Pros:** Zero JavaScript — the browser hands off to the phone's own
  camera app (better focus/flash/exposure handling than a JS canvas ever
  gets for free) and returns a file the existing multipart-handling code
  already knows how to consume. Consistent with this project's
  server-rendered-only architecture.
- **Cons:** The user briefly leaves the page for the OS camera UI; no
  in-page retake before the file is attached (the OS camera app's own
  confirm/retake screen covers this in practice).

**Chosen: Alternative D.**

---

### Alternative E: JS `fetch`/`EventSource` polling for desktop "waiting for scan" detection
- **Pros:** Partial-page update, no full reload.
- **Cons:** Same objection as Alternative C — new JS idiom for a project
  that has none today.

### Alternative F: `<meta http-equiv="refresh">` polling on `GET /scan` (chosen)
- **Pros:** Literally the same mechanism `document_show.html` already uses
  to detect `ocr_status` flipping from `pending`/`processing` to `done` —
  one polling idiom for the whole app instead of two. Each refresh is a
  normal `GET /scan` that re-checks the (still-valid, in-session) scan
  token's status server-side and redirects to `/documents/{id}?uploaded=true`
  once `scan_sessions.status = 'captured'`.
- **Cons:** Full-page reload every few seconds while waiting — the same
  tradeoff already accepted for OCR-status polling, so not a new cost this
  feature introduces to the codebase's UX.

**Chosen: Alternative F.**

---

### Alternative G: Client-side JS QR-code rendering
- **Pros:** No new Rust dependency.
- **Cons:** New JS dependency instead, same objection as C/E.

### Alternative H: Server-side QR generation (`qrcode` crate) rendered to inline SVG (chosen)
- **Pros:** Pure-Rust crate, no system dependency (unlike Tesseract in
  feature 008 — this one needs nothing added to the `Dockerfile`), SVG
  markup drops straight into the Askama template with no separate static
  asset or JS to load it.
- **Cons:** One new Cargo dependency — accepted; it's small, has no
  transitive system requirement, and there's no already-vendored
  alternative in this codebase to reuse instead.

**Chosen: Alternative H.**

---

### Alternative I: Duplicate the save-to-blob-and-insert-row logic in a new phone-scan handler
- **Pros:** No changes to existing `handlers/documents.rs` code.
- **Cons:** Feature 008's validation (size cap, content-type allow-list,
  `ocr_status` transitions, tenant-scoped `blob_key`) would exist in two
  places that must be kept in sync by hand — exactly the kind of drift
  CLAUDE.md's TDD/architecture discipline exists to avoid.

### Alternative J: Extract a shared `ingest` function, called by both entry points (chosen)
- **Pros:** `handlers::documents::create`'s body (multipart-field
  validation aside, which differs — `/scan` never receives title/tags/
  date-issued fields) already does "validate content-type, size-check,
  stream to blob, insert row, spawn detached OCR." Pulling that core into a
  shared function (`documents::ingest::store_and_queue_ocr(tenant_id,
  user_id, filename, content_type, bytes, state) -> Result<DocumentId,
  IngestError>`) means both the desktop multipart handler and the new
  phone-scan handler call one path — a size-cap or content-type-allow-list
  change only has to happen once.
- **Cons:** A small refactor to already-shipped, already-tested feature 008
  code — mitigated by feature 008's existing integration tests continuing
  to cover the extracted path unchanged; no test behavior should change,
  only where the logic lives.

**Chosen: Alternative J.**

## 3. Structural Decision
New migration adds `scan_sessions` (`id uuid pk`, `tenant_id`, `user_id`,
`token_hash text unique`, `status text` — `pending`/`captured`/`expired` —
`document_id uuid null`, `expires_at timestamptz`, `created_at
timestamptz`), following `password_reset_tokens`'s existing shape.

New `src/web/handlers/scan.rs`:
- `new_scan` (`GET /scan`, behind the normal `protected` router group —
  goes through `TenantContext` like every other authenticated route) mints
  a token, stores its hash, renders the QR-code page, and on each
  meta-refresh re-checks its own session's status server-side.
- `show_scan_phone` (`GET /scan/:token`) and `submit_scan`
  (`POST /scan/:token`) sit in a new **public** router group (not
  `protected` — there is no cookie to check) and resolve tenancy by hashing
  the path token and looking up the matching, unexpired, still-`pending`
  `scan_sessions` row. This is the one deliberate exception to "every
  request goes through `TenantContext`" flagged in §1: `tenant.id`/
  `user.id` are set as span attributes manually from the resolved row,
  matching `TenantContext::from_user_id`'s existing convention, rather than
  via the extractor itself, since the extractor's precondition (a session
  cookie) doesn't hold here by design.

`submit_scan` calls the new `documents::ingest::store_and_queue_ocr`
(extracted from `handlers::documents::create`, see Alternative J), then
updates `scan_sessions` to `status = 'captured'`, `document_id = <new id>`.

`AppState` gains `app_base_url` reuse (already exists, used by
`password_reset.rs`) for building the QR-encoded `{APP_BASE_URL}/scan/
{token}` URL — no new config.

## 4. OpenTelemetry Implications
`new_scan`, `show_scan_phone`, and `submit_scan` all carry
`#[tracing::instrument(skip(state, multipart))]` (skipping raw file bytes on
`submit_scan`, matching feature 008's convention on `documents::create`).
The scan token itself is never passed to `tracing::` macros or stored as a
span attribute — only the `scan_sessions.id` (a random UUID, not the
credential) is used for correlation, mirroring how `password_reset_tokens`
already keeps its raw token out of spans. `tenant.id`/`user.id` are set
manually as span attributes in the two public phone-side handlers once the
token resolves to a `scan_sessions` row, so downstream spans (including the
detached OCR task `submit_scan` spawns via `store_and_queue_ocr`) still
carry the same tenant/user correlation every other document-creating
request gets.
