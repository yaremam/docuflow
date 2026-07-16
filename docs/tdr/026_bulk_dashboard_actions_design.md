# TDR 026: Bulk Actions on the Dashboard

## 1. Context & Architectural Requirements
Backlog item: multi-select for bulk tag/delete, plus the deferred bulk
"reprocess all eligible OCR" action named in ARCHITECTURE.md §8 (feature
013's per-document-only limitation). Per CLAUDE.md: zero-panic,
tenant-scoped queries, compile-time-verified `sqlx` macros throughout.

## 2. Alternatives Evaluated

### Alternative A: Checkboxes + a bulk-action toolbar inside the existing single `<form method="get" action="/documents">`, using the project's established `formaction`/`formmethod` button-override idiom
- **Pros:** This project has a documented, real nested-`<form>` bug (found
  2026-07-13, affecting the date-suggestion "Use this date" button — see
  `tests/documents_date_suggestion.rs`'s regression test): a second
  `<form>` nested inside another is invalid HTML, and browsers silently
  close the *outer* form early against the inner form's closing tag,
  breaking every field rendered after it. Every existing alternate action
  on this page (collection rename/delete, "Save this search") already
  avoids this by using `<button formaction="..." formmethod="post"
  formnovalidate>` inside the one outer form instead of a second `<form>`.
  Bulk actions are POSTs living inside the same GET-method outer form —
  exactly the same shape, so the same idiom applies directly.
- **Cons:** None identified — this is the established, safe pattern.

### Alternative B: A second `<form>` wrapping just the doc-list and bulk toolbar
- **Pros:** Conceptually simpler to read in isolation.
- **Cons:** Rejected outright — this is exactly the bug class described
  above. A regression test (`bulk_actions_are_reachable_only_via_the_
  single_outer_form_not_a_nested_one`) asserts no `<form` appears between
  the outer form's opening tag and the bulk toolbar's buttons.

### Alternative C: A `return_to` hidden field carrying the dashboard's current filter/sort state through every bulk POST
- **Pros:** A bulk action (especially delete) redirects the user back to
  wherever they were filtered/sorted to, not a reset unfiltered dashboard
  — reuses `list`'s already-computed `save_search_query` (the exact same
  query string "Save this search" already bookmarks) as the hidden
  field's value, so no new backend computation is needed.
- **Cons:** None identified.

### Alternative D: Bulk delete confirmation, mirroring single-document `delete`'s precedent
- **Pros:** A document isn't trivially re-creatable (unlike a saved
  collection, whose `delete_collection` explicitly has no confirm step)
  — bulk delete gets the same confirm-page precedent as deleting one
  document, just listing N documents instead of one.
- **Cons:** None identified.

### Alternative E: Bulk "reprocess all eligible OCR" reuses `reprocess_ocr`'s exact eligibility guard and spawns the same `run_ocr` task per row, relying on the existing `state.ocr_semaphore` to bound concurrency
- **Pros:** Closes the exact gap ARCHITECTURE.md §8 named ("no bulk
  'reprocess all eligible documents' action... has to click the button
  once per document") without inventing new batching/throttling — the
  semaphore `run_ocr` already acquires bounds how many OCR passes run
  concurrently regardless of how many `tokio::spawn` calls happen at
  once. `docs/backlog/013_reprocess_ocr.md`'s deferred-scope note flagged
  "needs its own design for not saturating `state.ocr_semaphore`" as the
  open question — it turns out to already be solved by the semaphore's
  existence, not new machinery.
- **Cons:** None identified.

## 3. Structural Decision
We choose **Alternative A** (form structure), **Alternative C**
(`return_to`), **Alternative D** (delete confirmation), and
**Alternative E** (reprocess reuse).

**Routes** (`src/web/router.rs`), registered as literal 3-segment paths
(`/documents/bulk/...`) alongside the existing `/documents/:id/...`
parameterized routes — axum's router (matchit) prioritizes static
segments over a dynamic `:id` at the same position, so `/documents/bulk/
delete` never gets swallowed by `/documents/:id`'s handler:
- `POST /documents/bulk/delete` → `bulk_delete_confirm` — renders
  `document_bulk_delete_confirm.html` listing every selected document.
- `POST /documents/bulk/delete/confirm` → `bulk_delete` — the actual
  `DELETE ... WHERE id = ANY($1) AND tenant_id = $2 RETURNING blob_key`,
  best-effort per-row blob + thumbnail (`{blob_key}-thumb`, feature 025)
  delete, mirroring single `delete`'s pattern.
- `POST /documents/bulk/tag` → `bulk_tag` — no confirmation (additive,
  reversible): `UPDATE documents SET tags = array(select distinct
  unnest(tags || $1)) WHERE id = ANY($2) AND tenant_id = $3`, reusing the
  existing `Tags` form newtype (`src/web/forms.rs`) for the tag(s)
  parsed from the toolbar's single text input.
- `POST /documents/bulk/reprocess_ocr` → `bulk_reprocess_ocr` — the
  guarded `UPDATE ... WHERE id = ANY($1) AND tenant_id = $2 AND
  ocr_status NOT IN ('pending', 'processing') RETURNING id, blob_key,
  content_type`, then `tokio::spawn(run_ocr(...))` per returned row.

**Repeated form values**: checkboxes (`doc_ids`) and a bulk tag input
submit as a POST body with a repeated key — `axum::extract::Form`
(`serde_urlencoded`) can't collect that into a `Vec<Uuid>` (only ever
sees the last value), so these handlers use `axum_extra::extract::Form`
(`serde_html_form`) instead, aliased `MultiForm` to avoid colliding with
`axum::extract::Form`'s existing import — the exact same reasoning
`list` already established for `MultiQuery` on the GET side, just for
POST bodies (required enabling axum-extra's `form` Cargo feature
alongside its existing `query` feature).

**Tenant isolation**: every bulk mutation's `WHERE ... AND tenant_id =
$N` is the only isolation needed — an id belonging to another tenant
simply doesn't match and is silently excluded from the affected set,
consistent with every other mutating query in this file. No special
cross-tenant error path.

**Minimal vanilla JS** (`documents_list.html`, inline `<script>` at the
end of the content block, same defensive `if (!element) return;` /
IIFE shape `base.html`'s existing theme-toggle script already uses — not
a new framework decision): shows/hides the bulk toolbar and updates its
selected count as `doc_ids` checkboxes are toggled, and adds an
`is-selected` highlight class to checked rows. Pure progressive
enhancement — every bulk action is a plain form POST that works
identically with JS disabled (the toolbar would just always be visible).

## 4. OpenTelemetry Implications
No PII new to this feature: bulk handlers touch the same fields
(`tags`, `ocr_status`, `blob_key`) existing single-document handlers
already touch under the same tenant-scoping and instrumentation
conventions. No new span parameter is unskipped.
