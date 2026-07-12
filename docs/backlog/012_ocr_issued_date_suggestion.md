# User Story: OCR-Based Issued-Date Suggestion

## 1. User Value Statement
As a **logged-in DocuFlow user uploading a bill, insurance policy, or
contract**,
I want to **be shown a date DocuFlow noticed printed on the document
itself, and accept it with one click instead of retyping it**,
So that **I don't have to hunt through the document to fill in "Date
issued" by hand every time.**

## 2. Strict Acceptance Criteria
- **AC-1:** Once a document's OCR pass completes successfully
  (`ocr_status = 'done'`), the extracted text is scanned for a single
  best-guess date and, if found, stored in a new
  `ocr_suggested_date_issued` column — independent of `date_issued`,
  which is never written automatically.
- **AC-2:** On `GET /documents/{id}`, if `ocr_suggested_date_issued` is
  set and `date_issued` is still empty, the detail page shows the
  suggested date next to the "Date issued" field with a "Use this date"
  action. If `date_issued` already has a value, or no suggestion was
  found, nothing extra is shown.
- **AC-3:** Clicking "Use this date" (`POST
  /documents/{id}/accept_suggested_date`) copies the stored suggestion
  into `date_issued` and redirects back to the (now-saved) detail page —
  it never overwrites an already-set `date_issued`.
- **AC-4:** The user can still ignore the suggestion and type any date
  (or leave it blank) into the existing "Date issued" field and click
  "Save changes" as today — the suggestion is a convenience, never a
  requirement, and typing over it works exactly like it does now.
- **AC-5:** Date recognition covers, at minimum: ISO (`2024-03-15`),
  numeric slash/dash (`03/15/2024`, `03-15-2024`), and English month-name
  forms (`March 15, 2024`, `15 March 2024`). A date outside a sane range
  (before 1900, more than a year in the future) is never suggested.
- **AC-6:** No `.unwrap()`, `.expect()`, or `panic!()` introduced in the
  changed code, per CLAUDE.md's zero-panic rule — a document whose OCR
  text yields no recognizable date simply gets no suggestion, never an
  error.
- **AC-7:** No PII (OCR text, the matched date substring, surrounding
  context) enters trace spans or logs — only that a suggestion was or
  wasn't found is safe to record, never the extracted text itself.
- **AC-8:** Tenant scoping is preserved on the new endpoint exactly like
  every other `/documents/{id}` route — a request for another tenant's
  document 404s the same way `show`/`update`/`delete` already do.

## 3. Explicitly out of scope this round
- **Non-English month names** (e.g. Cyrillic month names like "март" even
  though feature 011 added Cyrillic OCR). Only English month names and
  numeric formats are recognized this round; broader locale support is
  future work if requested.
- **EXIF-based date suggestion.** Separately tracked backlog item — a
  different data source (image metadata, not OCR text) with its own
  design.
- **Retroactive reprocessing** of documents OCR'd before this feature
  ships — they simply have no `ocr_suggested_date_issued` until
  reprocessed, which is the separately-tracked "redo the OCR" item.
- **Multiple suggested dates / picking among candidates.** If OCR text
  contains several plausible dates (e.g. both a statement date and a due
  date), only the first match in priority/scan order is stored and
  suggested — no UI for choosing among several.
- **Editing/dismissing a suggestion without accepting it.** There's no
  explicit "dismiss" action; the suggestion simply stops showing once
  `date_issued` is set by any means (accept button or manual save).
