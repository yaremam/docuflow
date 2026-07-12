# User Story: Documents Dashboard

## 1. User Value Statement
As a **logged-in DocuFlow user**,
I want to **see all my documents in one place, search them by tag, sort them
by date or tag, and view/edit a single document's metadata and extracted
text**,
So that **the app functions as an actual filing system for my bills,
insurance policies, and contracts, not just a place I've signed up to.**

## 2. Strict Acceptance Criteria
- **AC-1:** `GET /documents` requires an authenticated session; an
  unauthenticated request is redirected to `/login`, never rendering the
  page.
- **AC-2:** `GET /documents` lists only documents belonging to the caller's
  own tenant — a second tenant's documents never appear, regardless of
  search or sort parameters.
- **AC-3:** When the tenant has no documents, the page renders an empty
  state (not an empty list with no explanation).
- **AC-4:** `GET /documents?q=<term>` filters to documents whose tags
  overlap with the comma-separated search term(s); a document with no
  matching tag is excluded.
- **AC-5:** `GET /documents?sort=<mode>` supports at minimum: date
  uploaded (newest/oldest), date issued (newest/oldest), and tags
  (alphabetical) — an unrecognized or absent `sort` value falls back to
  date uploaded, newest first.
- **AC-6:** `GET /documents/:id` renders a single document's metadata
  (title, filename, tags, date issued, upload date) and its extracted
  text if OCR has completed; if OCR hasn't completed yet (or failed, or
  isn't supported for the file type), a clear status placeholder is shown
  instead of blank space.
- **AC-7:** `GET /documents/:id` for a document belonging to another
  tenant returns `404`, indistinguishable from a nonexistent document id
  — it must not leak whether the id exists at all.
- **AC-8:** `POST /documents/:id` updates title, tags, and date issued in
  one request, persists them, and redirects back to the detail view with
  a visible "saved" confirmation.
- **AC-9:** `POST /documents/:id` against another tenant's document
  returns `404` and makes no change, same anti-enumeration guarantee as
  AC-7.
- **AC-10:** Every request to `/documents` and `/documents/:id` emits a
  trace span; no document title, tag, extracted OCR text, or other
  document content ever appears as a span attribute or log field.
- **AC-11:** No `.unwrap()`, `.expect()`, or `panic!()` in the new handler
  code; a database failure surfaces as a `Result`/`thiserror` error mapped
  to a proper HTTP response, never a panic.

## 3. Explicitly out of scope this round
Uploading a new document (file upload + OCR pipeline) and scanning a
document via a phone camera (cross-device handoff) are each their own,
separately-scoped future feature — this story covers only the
list/search/sort/view/edit surface over documents that already exist in
the database. The schema was still built with OCR-related columns
(`ocr_status`/`ocr_text`/`ocr_error`) up front so those future features
don't require a schema migration of their own.
