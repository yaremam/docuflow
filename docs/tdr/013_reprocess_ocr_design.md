# TDR 013: Reprocess OCR

## 1. Context & Architectural Requirements
Every OCR-eligible document runs through `run_ocr`
(`src/web/handlers/documents.rs:500`) exactly once, spawned right after
insert (`create`) or phone capture (`scan::submit_scan`), via
`insert_document_and_queue_ocr`. A document's `ocr_status`/`ocr_text`/
`ocr_suggested_date_issued` then sit however that one pass left them,
forever — feature 010's PDF support, feature 011's Cyrillic support, and
feature 012's issued-date suggestion all only apply to documents OCR'd
*after* each shipped (see ARCHITECTURE.md §8, "Retroactive OCR
reprocessing"). We want a way to re-run today's pipeline against an
already-stored document on demand, reusing the existing OCR machinery
rather than building a second one. Per CLAUDE.md: zero-panic, tenant-scoped
queries, and no OCR text/file bytes entering spans beyond what `run_ocr`
already permits.

## 2. Alternatives Evaluated

### Alternative A: A `POST /documents/{id}/reprocess_ocr` handler that atomically flips `ocr_status` back to `pending` and re-spawns `run_ocr`
- **Pros:** Reuses `run_ocr` verbatim — no new OCR-invocation code path, no
  new span, no new PII surface. A single guarded `UPDATE ... where
  ocr_status not in ('pending', 'processing') returning blob_key,
  content_type` both makes the "don't queue a second job on an in-flight
  document" check (AC-4) and the state transition atomic in one
  round-trip, the same guarded-`UPDATE` idiom `accept_suggested_date`
  already uses for its own "don't overwrite an already-set `date_issued`"
  guarantee. Works uniformly for `done`, `failed`, and `skipped` rows —
  there's no special case for "this row was never OCR'd" vs. "this row was
  OCR'd but the pipeline has since improved."
- **Cons:** No record of *why* a document is being reprocessed, or what
  pipeline version last touched it — a user could click it on a document
  that gains nothing from a fresh pass. Accepted (see backlog's
  out-of-scope list) rather than building version-tracking infrastructure
  for a per-document manual action.

### Alternative B: A background sweep that automatically re-queues affected documents when a pipeline feature ships
- **Pros:** No user action needed — every `skipped` PDF or pre-Cyrillic
  `done` document gets fixed on its own.
- **Cons:** Needs a way to know which documents are "affected by" a given
  pipeline change (a schema field recording which pipeline version
  produced each OCR result, absent today) to avoid either reprocessing
  every document on every deploy or reprocessing nothing. Also needs
  throttling against `state.ocr_semaphore` well beyond a single document's
  worth of work. Explicitly out of scope this round (see backlog);
  Alternative A's per-document button is a strict subset of what this
  would eventually need and doesn't foreclose building it later.

### Alternative C: Separate "Retry OCR" (failed-only) and "Reprocess OCR" (done/skipped-only) actions with distinct endpoints/labels
- **Pros:** Slightly more precise language — "retry" for a failure,
  "reprocess" for an intentional re-run.
- **Cons:** Rejected during mockup review (signed off 2026-07-13): the two
  cases are mechanically identical (re-run `run_ocr` against the stored
  file) and a `failed` document benefits from the exact same button a
  `skipped`/`done` one does. Two endpoints/labels for one action is
  duplication with no behavioral difference to justify it, and would need
  its own tiny TDR-worthy tie-break rule for what happens if both could
  apply (a `failed` document that's also old enough to predate Cyrillic
  support).

## 3. Structural Decision
We choose **Alternative A**. Add `POST /documents/{id}/reprocess_ocr`
(`src/web/handlers/documents.rs`, router entry next to
`accept_suggested_date` in `router.rs`):

```sql
update documents set ocr_status = 'pending', updated_at = now()
where id = $1 and tenant_id = $2 and ocr_status not in ('pending', 'processing')
returning blob_key, content_type
```

- **No row returned:** fall back to an existence check (same pattern as
  `accept_suggested_date`) — 404 if the document doesn't exist for this
  tenant, otherwise a no-op redirect (it was already `pending`/
  `processing`).
- **Row returned:** spawn `run_ocr(state.clone(), id, tenant_id, blob_key,
  content_type).instrument(tracing::Span::current())` exactly as
  `insert_document_and_queue_ocr` already does, then redirect to
  `/documents/{id}?reprocessing=true`. `run_ocr` itself flips
  `ocr_status` to `processing` once it acquires an `ocr_semaphore` permit,
  same as a fresh upload — the handler only needs to get the row back to
  `pending` and hand off.

`document_show.html`'s "Extracted text" card gains an action row (per the
signed-off mockup): a "Reprocess OCR" button when `ocr_status` is `done`,
`failed`, or `skipped`; a status pill ("Reprocessing…") when `pending`/
`processing` instead, relying on the page's existing 5-second meta-refresh
(feature 008) to flip it back once the pass lands — no new JS. No
confirmation page, matching AC-5: reprocessing can't destroy anything a
second attempt can't recreate, unlike delete.

## 4. OpenTelemetry Implications
The new handler gets `#[tracing::instrument(skip(state, tenancy))]`,
matching every sibling handler in this file — it takes no OCR text or file
bytes as parameters, only `id`/`tenant_id` (already-safe span attributes
elsewhere in this file). The spawned `run_ocr` call is the same function,
same instrumentation boundary, same "only whether extraction succeeded is
span-safe" rule as every other call site — no new PII surface.
