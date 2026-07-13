# User Story: Reprocess OCR

## 1. User Value Statement
As a **logged-in DocuFlow user with documents that were uploaded before a
pipeline improvement shipped** (an old PDF stuck at "text extraction isn't
available," a Cyrillic document OCR'd before language support landed, or
simply a document whose text extraction failed),
I want to **run text extraction again with today's OCR pipeline, on
demand**,
So that **I don't have to delete and re-upload a document just to benefit
from an OCR improvement, or to retry a one-off failure.**

## 2. Strict Acceptance Criteria
- **AC-1:** `GET /documents/{id}` shows a "Reprocess OCR" button on the
  "Extracted text" card whenever `ocr_status` is `done`, `failed`, or
  `skipped` — i.e. whenever the document isn't already mid-flight.
- **AC-2:** `POST /documents/{id}/reprocess_ocr` re-runs the current OCR
  pipeline (`crate::ocr::extract`, same dispatch used for a fresh upload)
  against the document's already-stored file, overwriting `ocr_text`,
  `ocr_status`, `ocr_error`, and `ocr_suggested_date_issued` exactly as a
  fresh upload's background OCR pass would. It never touches `date_issued`.
- **AC-3:** A `skipped` document (e.g. a PDF uploaded before feature 010)
  reprocesses the same way a `done` or `failed` one does — reprocessing is
  what turns a permanently-`skipped` row into a real OCR attempt.
- **AC-4:** While `ocr_status` is `pending` or `processing`, the button is
  replaced by a status indicator and a `POST` to `reprocess_ocr` is a
  no-op (no second background job queued) rather than an error — a request
  can't pile up duplicate OCR jobs on one document.
- **AC-5:** No confirmation step is required before reprocessing — unlike
  delete, nothing is permanently lost; the document's stored file is
  untouched and a reprocess can always be run again.
- **AC-6:** Tenant scoping matches every other `/documents/{id}` route — a
  request for another tenant's document 404s the same way `show`/`update`/
  `delete`/`accept_suggested_date` already do.
- **AC-7:** No `.unwrap()`, `.expect()`, or `panic!()` introduced in the
  changed code, per CLAUDE.md's zero-panic rule.
- **AC-8:** No PII (OCR text, file bytes) enters trace spans or logs beyond
  what the existing `run_ocr`/`ocr::extract` instrumentation already
  permits — reprocessing reuses that same code path, not a new one.

## 3. Explicitly out of scope this round
- **Bulk / "reprocess all" action.** This round is a per-document button
  only. A bulk sweep (e.g. re-queue every `skipped` document at once) is
  future work if requested — it needs its own design for not saturating
  `state.ocr_semaphore`.
- **Automatic re-queueing.** Nothing runs this on a schedule or on boot;
  a document only reprocesses when a user clicks the button.
- **Tracking "which pipeline version produced this OCR result."** The
  button always re-runs the *current* pipeline; there's no bookkeeping
  about what changed since the last pass, so a user might reprocess a
  document that gains nothing from it. That's an accepted tradeoff for
  not building version-tracking infrastructure this round.
- **A distinct "retry" label/endpoint for the `failed` case.** The same
  button and copy cover both "redo the OCR" and "retry after failure" —
  see TDR 013 §3.
