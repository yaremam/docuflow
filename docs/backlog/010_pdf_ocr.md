# User Story: PDF OCR

## 1. User Value Statement
As a **logged-in DocuFlow user**,
I want to **have text automatically extracted from PDF bills, insurance
policies, and contracts I upload, the same way it already works for image
uploads**,
So that **I can find and search PDF documents by their content, not just
by title/tags I typed in manually.**

## 2. Strict Acceptance Criteria
- **AC-1:** `POST /documents` and the phone-camera scan handoff now treat
  `application/pdf` as OCR-eligible: `ocr_status` starts `pending` (not
  `skipped`) and the document is queued for background extraction exactly
  like `image/jpeg`, `image/png`, `image/tiff`, `image/webp`.
- **AC-2:** The background OCR worker rasterizes each page of the PDF to
  an image, runs the existing `extract_text` (Tesseract) pipeline over
  each page image, and concatenates the per-page text (with a clear page
  separator) into a single `ocr_text` value on the document row.
  `ocr_status` transitions `pending` -> `processing` -> `done`, matching
  the existing image-upload state machine.
- **AC-3:** A PDF that fails to rasterize (corrupt file, zero pages,
  password-protected/encrypted) ends at `ocr_status = 'failed'` with a
  populated `ocr_error`, never a panic and never a stuck `processing` row
  outside the existing boot-time stuck-`processing` sweep.
- **AC-4:** `GET /documents/{id}` shows the concatenated extracted text
  for a `done` PDF the same way it does for images today, without any
  template changes beyond what already renders `ocr_text`.
- **AC-5:** Every request/background-task span stays PII-clean: no PDF
  page image bytes, no rasterized page count beyond a safe bound, and no
  extracted text ever appears as a span attribute or log field.
- **AC-6:** No `.unwrap()`, `.expect()`, or `panic!()` in the new
  rasterization/OCR code; a rasterization or Tesseract failure surfaces as
  an `ocr_status = 'failed'` row update via `thiserror`, never a panic.
- **AC-7:** Tenant scoping and the 20MB upload size limit are unaffected —
  this feature only changes what happens to an already-accepted PDF after
  upload.

## 3. Explicitly out of scope this round
- **Retroactive reprocessing.** PDFs uploaded before this feature shipped
  keep `ocr_status = 'skipped'` — they are not automatically re-queued.
  Reprocessing them is the separately-tracked "OCR retry" backlog item.
- **Per-page storage/navigation.** Extracted text is stored as one
  concatenated `ocr_text` value per document, not as separate per-page
  records or a page-by-page viewer.
- **Password-protected / encrypted PDFs.** These are expected to fail
  rasterization and land in `ocr_status = 'failed'`; a decrypt/password
  prompt flow is not part of this feature.
- **Cyrillic / non-Latin OCR accuracy** and **EXIF/OCR date extraction**
  remain their own separately-tracked backlog items.
