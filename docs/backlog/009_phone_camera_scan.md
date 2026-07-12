# User Story: Phone-Camera Scan Handoff

## 1. User Value Statement
As a **logged-in DocuFlow user sitting at a desktop**,
I want to **scan a QR code with my phone and photograph a document with my
phone's camera**,
So that **I can get a paper bill, insurance policy, or contract into
DocuFlow without needing a scanner, a file already on my phone, or emailing
myself a photo first.**

## 2. Strict Acceptance Criteria
- **AC-1:** `GET /scan` requires an authenticated desktop session, creates a
  new scan session scoped to the caller's `tenant_id`/`user_id`, and renders
  a page with a QR code encoding a phone-facing URL
  (`{APP_BASE_URL}/scan/{token}`).
- **AC-2:** `GET /scan/{token}` is public (no session cookie required) and,
  for a token that is unexpired and not yet used, renders a mobile page with
  a native camera-capture file input and a submit button. An unknown,
  expired, or already-used token renders a distinct error state and creates
  no document.
- **AC-3:** `POST /scan/{token}` accepts `image/jpeg` or `image/png` (the
  formats phone cameras produce) for a still-valid token, and persists the
  photo as a new document — same validation, size cap, blob storage, and
  automatic-OCR pipeline as the existing `POST /documents` upload (feature
  008) — attributed to the `tenant_id`/`user_id` recorded on the scan
  session, not any session cookie on the phone (the phone is never logged
  in). Any other content type is rejected with `400` and creates no
  document.
- **AC-4:** A successful `POST /scan/{token}` marks the scan session
  captured (recording the new document's id) and shows the phone a "Scan
  received — you can close this tab" confirmation; the token cannot be
  reused for a second photo afterward.
- **AC-5:** The desktop `GET /scan` page updates on its own on a reasonable
  timescale once the phone has captured a photo, and takes the user to
  `/documents/{id}?uploaded=true` for the newly created document — no manual
  refresh or resubmission needed.
- **AC-6:** A scan session expires 10 minutes after creation; an expired
  token behaves identically to an already-used one (AC-2's error state, no
  document created), even if the phone had the page loaded before expiry.
- **AC-7:** The scan token is never logged, traced, or persisted in plain
  text — only a hash of it is stored, matching the existing
  `password_reset_tokens` pattern.
- **AC-8:** Documents captured via `/scan/{token}` are strictly
  tenant-scoped exactly like feature 008 uploads: never visible to, or
  reachable by, a tenant other than the one whose desktop session created
  the QR code.
- **AC-9:** Every request to `/scan`, `GET /scan/{token}`, and
  `POST /scan/{token}` emits a trace span; no scan token, file content, or
  extracted OCR text ever appears as a span attribute or log field.
- **AC-10:** No `.unwrap()`, `.expect()`, or `panic!()` in the new
  handler/QR-generation code; a database, blob-storage, or malformed-token
  failure surfaces as a `Result`/`thiserror` error, never a panic.

## 3. Explicitly out of scope this round
- **Multi-photo / batch capture.** One QR code produces exactly one
  document. Scanning a multi-page document is a future increment (repeat
  the flow per page, or a dedicated multi-page mode later).
- **Live in-browser camera preview (WebRTC).** The phone uses its native
  camera app via the browser's standard capture affordance, not an
  in-page live video/canvas capture flow.
- **PDF from phone.** Only the two image types a phone camera actually
  produces are accepted; PDF-from-phone (e.g. a scanning app's PDF export)
  can go through the existing desktop `/documents/new` upload instead.
- **Editing title/tags/date-issued from the phone.** The phone flow only
  captures the photo; metadata stays editable afterward from the desktop
  document page, same as any other upload.
