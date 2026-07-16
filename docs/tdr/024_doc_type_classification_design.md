# TDR 024: Auto-Classification / Document Type

## 1. Context & Architectural Requirements
Backlog item: suggest a `doc_type` (bill, contract, insurance, receipt,
ID…) from OCR text via keyword rules, confirmable like feature 012's
`ocr_suggested_date_issued`, and it should become another smart-filter
facet alongside Tags/Date issued/Language. Per CLAUDE.md: zero-panic,
tenant-scoped queries, compile-time-verified `sqlx` macros throughout, no
new PII surface in spans.

## 2. Alternatives Evaluated

### Alternative A: Suggest-then-confirm, mirroring feature 012 exactly
- **Pros:** `documents.doc_type` (confirmed) + `ocr_suggested_doc_type`
  (suggestion) is the identical shape as `date_issued`/
  `ocr_suggested_date_issued` — a wrong guess never lands silently, same
  explicit-accept UX (`document_show.html`'s suggestion box +
  `formaction`/`formmethod` button), same guarded-`UPDATE ... where
  doc_type is null` accept handler.
- **Cons:** None identified — this is exactly what the backlog item asked
  for ("confirmable like the 012 date suggestion").

### Alternative B: Auto-write the classification directly into `doc_type`, no confirm step
- **Pros:** One less click for the user when the guess is right.
- **Cons:** Rejected for the same reason TDR 012 rejected auto-filling
  `date_issued` (its Alternative C): a keyword match is a guess, not a
  fact, and a wrong one landing silently is worse than requiring one
  click to accept a right one.

### Alternative C: A fixed Postgres enum / `CHECK` constraint on `doc_type`
- **Pros:** Guarantees only known categories are ever stored.
- **Cons:** Rejected for the same reason TDR 020 opened `language` up from
  a fixed set to any ISO 639-1 code: baking categories into a DB
  constraint means a future category (e.g. "warranty") needs a migration,
  not just a UI change. `doc_type` stays plain `text`; the *dropdown* is
  the fixed list (`doc_type_extract::dropdown_options()`), enforced at the
  Rust boundary (`DocTypeField`'s `TryFrom<String>`), not the schema.

### Alternative D: Facet shape mirrors `date_issued` (single active value) instead of `language` (multi-select + "unset")
- **Pros:** N/A.
- **Cons:** Rejected — a user might reasonably want "show me Bills OR
  Receipts" (an OR-multi-select, like language's `en`+`de`), which
  `date_issued`'s single-active-year-plus-undated shape can't express.
  `doc_type` has no natural "at most one active" constraint the way a
  single calendar year does, so it's structurally the same kind of facet
  as `language`, not `date_issued` — same `Vec<String>` `ListQuery` field,
  same `"unset"` sentinel, same discover-candidates-then-narrow-count
  loop (TDR 018 §3).

## 3. Structural Decision
We choose **Alternative A** for the suggestion mechanism and **Alternative
D** for the facet shape.

**Extraction** (`src/doc_type_extract.rs`): a small `DocType` enum
(`Id`/`Insurance`/`Contract`/`Receipt`/`Bill` — no `Other` variant, see
below) with a fixed keyword list per category, checked in
most-distinctive-first order (`Id` → `Insurance` → `Contract` → `Receipt`
→ `Bill`) so the most generic category (`Bill`'s "invoice"/"amount due")
only wins once nothing more specific matched — the same "narrow, fixed-
shape scanner, not a general classifier" framing `date_extract.rs`'s own
module doc comment uses. Plain `#[cfg(test)]` unit tests against
`extract_doc_type` directly, no DB/HTTP, mirroring `date_extract`'s test
shape exactly.

**Schema**: `documents.doc_type text` (confirmed) + `documents.
ocr_suggested_doc_type text` (suggestion), both nullable, no `CHECK`
constraint (Alternative C rejected above). Written in `run_ocr` (`src/web/
handlers/documents.rs`) in the same `UPDATE` that already writes
`ocr_text`/`ocr_suggested_date_issued`/`language`.

**The confirmed field's `<select>`** offers a fixed 6-option list —
`doc_type_extract::dropdown_options()` (Bill/Contract/Insurance/Receipt/
ID/Other) — including `"other"`, which has no `DocType` variant and is
never suggested; it exists purely as a manual catch-all for documents the
keyword ruleset doesn't recognize. A new `web::forms::DocTypeField`
newtype (mirroring `Language`'s blank-means-clear, validate-against-a-
known-set shape) enforces server-side that a submitted value is blank or
one of those 6, independent of what the `<select>` offers client-side.

**Facet**: `ListQuery.doc_type: Vec<String>`, threaded through
`count_documents` and all 5 `Sort` arms in `list` as two new positional
params (`doc_type_values`, `doc_type_unset`) appended at the end (`$10`/
`$11`) rather than renumbering `$1`-`$9` — same "smaller diff" reasoning
TDR 023 used when it appended `search_text` as `$9`. A new
discover-candidates-then-narrow-count loop (`distinct doc_type`, one
`count_documents` call per candidate plus one for `"unset"`) mirrors the
Language facet loop line for line. `build_query_string`/
`build_documents_url`, `query_has_active_filters`, and the applied-filter
chips all get a `doc_type` case alongside `lang`'s, for the same
save-collection/chip/clear-all consistency reasons TDR 015 §3 already
established.

**Dashboard row / suggestion box UI**: signed off via a combined mockup
Artifact (2026-07-16, alongside features 025/026) before any handler/
template code was written, per CLAUDE.md §5.

## 4. OpenTelemetry Implications
No new spans, no new parameter entering any `#[tracing::instrument]`'d
function's captured args — `show`/`update`/`accept_suggested_doc_type`'s
existing `skip(state, tenancy, ...)` instrumentation already covers every
new value here (a doc_type guess is no more sensitive than the date
suggestion or OCR text already flowing through these same skipped
parameters). Spot-checked in Jaeger, not just assumed, per the
tracing-PII-on-refactor lesson.
