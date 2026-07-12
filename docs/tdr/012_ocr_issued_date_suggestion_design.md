# TDR 012: OCR-Based Issued-Date Suggestion

## 1. Context & Architectural Requirements
`documents.date_issued` is a plain nullable `date` column, filled in only
by hand today via the upload form and the `/documents/{id}` edit form
(`src/web/handlers/documents.rs::update`). Since feature 008, every
OCR-eligible upload already produces `ocr_text` via the background
`run_ocr` worker; we want to mine that text for a date the document
itself states (a bill's statement date, a policy's issue date) and offer
it as a one-click suggestion, without ever silently overwriting a value
the user typed or already accepted. Per CLAUDE.md: zero-panic (a
document with unrecognizable text just gets no suggestion, never an
error), PII-safe spans (no OCR text or matched substrings in traces), and
tenant-scoped queries throughout.

## 2. Alternatives Evaluated

### Alternative A: Regex-based scanning over fixed date shapes, stored in a new column, surfaced as an explicit-accept suggestion
- **Pros:** `regex` is a small, well-tested, widely-used crate — precise
  control over which shapes are recognized and in what priority, with no
  fuzzy/surprising matches. Capturing numeric groups directly and
  constructing `time::Date::from_calendar_date(year, month, day)` (the
  same constructor already used in `web::forms::DateIssuedField`, see
  `src/web/forms.rs:375`) means invalid dates (Feb 30, a garbled OCR
  digit) are rejected by the same validated path the manual-entry form
  already relies on — no new panic surface. Storing the result in its own
  `ocr_suggested_date_issued` column (rather than writing straight into
  `date_issued`) keeps "what OCR found" and "what the user confirmed"
  strictly separate, which is what makes the explicit-accept UX (see
  mockup, signed off 2026-07-12) possible at all.
- **Cons:** Regex-based date extraction is inherently a heuristic — a
  handful of fixed shapes, not a general natural-language date parser.
  Ambiguous numeric dates (`03/04/2024`: US MM/DD or day-first?) need a
  documented tie-break rule (see §3) rather than a universally "correct"
  answer.

### Alternative B: A general-purpose natural-language date-parsing crate (e.g. a `dateparser`/`chrono-english`-style crate)
- **Pros:** Handles a much broader range of phrasing out of the box
  ("next Tuesday", relative dates, more locales) with less hand-written
  matching logic.
- **Cons:** OCR'd bill/contract text is not natural language input — it's
  noisy, fixed-format machine/print text, so the extra flexibility a
  natural-language parser buys is mostly wasted, while its larger surface
  (broader grammar, more silent-success edge cases) makes false-positive
  matches on incidental numbers in an invoice (account numbers, amounts)
  *harder* to predict and rule out, not easier. Heavier dependency for a
  narrower actual need than Alternative A.

### Alternative C: Auto-fill `date_issued` directly instead of a separate suggested-date column
- **Pros:** No new column, no new endpoint — one field to reason about.
- **Cons:** Explicitly rejected during scoping (2026-07-12): a wrong OCR
  guess would land silently in the real field with no review step, and
  the todo item's own wording is "suggest," not "set." Keeping the two
  columns separate is also what makes AC-3/AC-4 (never overwrite an
  already-set `date_issued`) trivial to guarantee at the SQL level (`...
  where date_issued is null`) rather than needing extra application-level
  bookkeeping about whether a value was auto- or user-set.

## 3. Structural Decision
We choose **Alternative A**. Add nullable `documents.ocr_suggested_date_issued
date` via migration. Add `src/date_extract.rs` with
`pub fn extract_issued_date(text: &str) -> Option<time::Date>`, tried in
this priority order (first valid match wins, scanning left-to-right
within each shape):
1. ISO `YYYY-MM-DD`
2. English month name, either order — `Month D[,] YYYY` or `D Month YYYY`
3. Numeric `M/D/YYYY` or `M-D-YYYY` — disambiguated as: if one of the two
   numbers is `> 12` it's unambiguously the day; if both are `<= 12`,
   assume US `MM/DD/YYYY` (documented ambiguity — this is a suggestion
   the user can always overrule, not a claimed-authoritative parse).

Every candidate is validated via `time::Date::from_calendar_date` and a
sane-range check (`1900..=current_year + 1`) before being accepted;
anything that fails either check is skipped, never surfaced. `run_ocr`
(`src/web/handlers/documents.rs:448`) calls this once, right after a
successful `crate::ocr::extract` call, and writes the result into
`ocr_suggested_date_issued` in the same `UPDATE` that already sets
`ocr_status = 'done', ocr_text = $3`. A new
`POST /documents/{id}/accept_suggested_date` handler runs
`update documents set date_issued = ocr_suggested_date_issued where id =
$1 and tenant_id = $2 and date_issued is null`, then redirects to
`/documents/{id}?saved=true` (the same flash-message pattern `update`
already uses). `document_show.html` shows the suggestion box only when
`date_issued_input_value` is empty and a suggestion exists.

## 4. OpenTelemetry Implications
`extract_issued_date` takes `text: &str` and must be called with
`#[tracing::instrument(skip(text))]` if it ever gets its own span — in
practice it's called inline inside `run_ocr`'s existing
`#[tracing::instrument(skip(state))]` span, so no new span is
introduced. Only whether a suggestion was found (a `bool`) is safe to
record as a span attribute if ever useful for debugging; the matched
date/substring and the OCR text itself are never attached to spans or
logs.
