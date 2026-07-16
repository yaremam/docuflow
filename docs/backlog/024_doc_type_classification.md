# User Story: Auto-Classification / Document Type

## 1. User Value Statement
As a **logged-in DocuFlow user reviewing an uploaded document**,
I want to **see a suggested document type (Bill, Contract, Insurance,
Receipt, ID) based on its scanned text, and confirm or override it**,
So that **I can filter my dashboard by type without having to tag every
document by hand.**

## 2. Strict Acceptance Criteria
- **AC-1:** After OCR completes, a document whose text matches one of the
  keyword rulesets gets an `ocr_suggested_doc_type` — never written into
  the confirmed `doc_type` automatically.
- **AC-2:** `document_show` shows a suggestion box ("OCR suggests: Bill" +
  a "Use this" button) only when a suggestion exists *and* `doc_type` is
  still unset — same one-condition rule as the existing date suggestion
  (TDR 012).
- **AC-3:** Accepting a suggestion (`POST /documents/{id}/
  accept_suggested_doc_type`) copies it into `doc_type` and redirects to
  `/documents/{id}?saved=true`; calling it again (or on a document that
  already has `doc_type` set some other way) is a no-op, not an error.
- **AC-4:** The document's metadata form has a "Document type" `<select>`
  (Bill/Contract/Insurance/Receipt/ID/Other/Not set) that can be changed
  and saved independently of any suggestion.
- **AC-5:** The smart-filters sidebar gets a new "Document type" facet
  group — checkboxes, OR-combined within the facet, AND-combined with
  Tags/Date issued/Language, with a "Not set" option — same shape as the
  existing Language facet (TDR 024 §2 Alternative D).
- **AC-6:** Each Document type facet option's count narrows by whichever
  other facets are currently active, same as every other facet since
  feature 018.
- **AC-7:** `accept_suggested_doc_type` is tenant-scoped — another
  tenant's attempt 404s and never consumes the real owner's suggestion.
- **AC-8:** No `.unwrap()`, `.expect()`, or `panic!()` introduced.
- **AC-9:** No new PII in spans/logs.
- **AC-10:** The "Use this" suggestion button lives inside the page's one
  metadata `<form>` via `formaction`/`formmethod`, never as a nested
  `<form>` of its own (regression class covered by feature 012's own
  nested-form bug, 2026-07-13).

## 3. Explicitly out of scope this round
- **A machine-learning classifier.** This ships a small, fixed keyword
  ruleset only (`src/doc_type_extract.rs`) — no training data, no model.
- **User-defined custom categories.** The `<select>`'s 6 options are
  fixed in code; `doc_type` itself is open `text` in the schema so a
  future category needs a UI change only, not a migration (TDR 024 §2
  Alternative C).
- **Bulk re-classification of existing documents.** A document uploaded
  before this feature shipped has no `ocr_suggested_doc_type` until it's
  reprocessed (feature 013's existing per-document "Reprocess OCR"
  button) — no automatic backfill sweep.
