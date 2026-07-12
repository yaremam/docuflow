# User Story: Document Upload

## 1. User Value Statement
As a **logged-in DocuFlow user**,
I want to **upload a bill, insurance policy, or contract file and have its
text extracted automatically**,
So that **I don't have to manually retype or re-file information from a
document I already have, and I can find it later by searching its
content's tags.**

## 2. Strict Acceptance Criteria
- **AC-1:** `GET /documents/new` requires an authenticated session and
  renders a form for Title, Tags, Date issued (all optional), and a file
  picker (required).
- **AC-2:** `POST /documents` accepts `image/jpeg`, `image/png`,
  `image/tiff`, and `image/webp` — these are queued for automatic text
  extraction (`ocr_status` starts `pending`, transitions to `processing`
  then `done`/`failed`). `application/pdf` is also accepted and stored,
  but inserted directly with `ocr_status = 'skipped'` — no extraction is
  attempted this round. Any other content type is rejected with `400` and
  no row is created.
- **AC-3:** A successful upload persists `original_filename`,
  `content_type`, `file_size_bytes` (matching the actual uploaded byte
  count), and the optional Title/Tags/Date-issued fields if provided, then
  redirects to `/documents/{id}?uploaded=true`.
- **AC-4:** An upload exceeding the size limit (20MB) is rejected and
  creates no row.
- **AC-5:** An invalid optional field (e.g. a title over the shared
  length cap) is rejected with `400` and creates no row — same validation
  used by the existing metadata-edit form.
- **AC-6:** For an OCR-eligible upload, the extracted text eventually
  appears on `GET /documents/{id}` (`ocr_status` reaches `done`,
  `ocr_text` populated) without the user needing to resubmit anything —
  the page updates on its own on a reasonable timescale while `pending`/
  `processing`.
- **AC-7:** Uploads are strictly tenant-scoped: a document uploaded by one
  tenant is never visible in another tenant's `/documents` list or
  reachable via `/documents/{id}`.
- **AC-8:** Every request to `/documents/new` and `POST /documents` emits
  a trace span; no document title, tag, file content, or extracted OCR
  text ever appears as a span attribute or log field.
- **AC-9:** No `.unwrap()`, `.expect()`, or `panic!()` in the new
  handler/OCR code; a database, blob-storage, or OCR failure surfaces as
  a `Result`/`thiserror` error (or, for the background OCR pass, an
  `ocr_status = 'failed'` row update), never a panic.

## 3. Explicitly out of scope this round
- **PDF text extraction.** PDFs are accepted and viewable but not OCR'd —
  rasterizing PDF pages for Tesseract is a larger follow-up feature.
- **Phone-camera scanning** (cross-device QR handoff) remains its own,
  separately-scoped future feature, per the user's earlier direction to
  split upload and phone-scan into separate increments.
- **OCR retry.** A document stuck at `ocr_status = 'failed'` (or reset to
  `'pending'` by the boot-time sweep after an interrupted background job)
  has no automatic or manual retry path yet.
