# User Story: Dashboard Thumbnails + Side-by-Side Document Preview

## 1. User Value Statement
As a **logged-in DocuFlow user scanning my dashboard for a document**,
I want to **see a real preview thumbnail for every document (including
PDFs), and view a document's page next to its extracted text**,
So that **I can recognize a document at a glance instead of relying on a
generic file icon, and cross-check the OCR text against the actual page
without scrolling.**

## 2. Strict Acceptance Criteria
- **AC-1:** After OCR completes for an eligible document (image or PDF),
  a resized preview thumbnail is generated and stored — `documents.
  thumbnail_status` reaches `"done"`.
- **AC-2:** The dashboard (`/documents`) shows this generated thumbnail
  for both images and PDFs, replacing the pre-025 generic "PDF" icon for
  PDFs and the full-resolution-scaled-by-CSS image for direct image
  uploads.
- **AC-3:** A document whose thumbnail hasn't been generated yet (still
  processing) or failed to generate falls back to exactly the pre-025
  rendering — no broken image, no missing row.
- **AC-4:** The document detail page (`/documents/{id}`) shows the
  original-file preview and the extracted-OCR-text card side by side at
  wide viewport widths, rather than stacked vertically.
- **AC-5:** No `.unwrap()`, `.expect()`, or `panic!()` introduced — a
  corrupt/unrecognized image fails thumbnail generation gracefully
  (`thumbnail_status = "failed"`), never panics the background task.
- **AC-6:** No new PII in spans/logs — raw image bytes never enter a
  trace.

## 3. Explicitly out of scope this round
- **Search-hit highlighting/snippets** in the OCR text box — the reason
  this was originally paired with feature 023 (full-text search) in
  ARCHITECTURE.md §8; a distinct follow-up, not built here (TDR 025 §4).
- **Backfilling thumbnails for documents uploaded before this feature
  shipped.** A pre-existing document's thumbnail only appears once it's
  reprocessed via feature 013's existing "Reprocess OCR" button.
- **Multi-page previews for a multi-page scanned PDF** (feature 022) —
  only page 1 is thumbnailed; the detail page's PDF `<embed>` already lets
  a user page through the rest natively.
