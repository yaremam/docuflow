# TDR 029: Duplicate Detection

## 1. Context & Architectural Requirements
Backlog item: warn a user who uploads a file they've already uploaded
before. Per CLAUDE.md: zero-panic, streaming/bounded-memory blob
handling, complete tenant data isolation, no new PII in spans.

## 2. Alternatives Evaluated

### Alternative A: Block the upload when a duplicate is detected
- **Pros:** No duplicate rows can ever exist.
- **Cons:** Rejected. The file's hash is only fully known once every
  byte has streamed through (`BlobStore::stream_upload` already computes
  its running byte-count the same way — one pass, not two); blocking
  would mean uploading the whole blob to S3 first and then deleting it
  again once the duplicate is confirmed, plus removing the user's
  ability to intentionally re-upload the same file (e.g. a corrected
  re-scan). A warn-only design needs none of that rollback machinery.

### Alternative B: Warn only, still create the document
- **Pros:** Matches the backlog's own wording ("warn ..."); no upload
  round-trip is ever wasted; consistent with this app's existing
  suggestion-based philosophy (feature 012/024: never silently block,
  always let the user decide).
- **Cons:** A user who ignores the warning can end up with real
  duplicate rows — accepted; that's the point of a *warning*, not a
  *guard*.

### Alternative C: A persistent, dismissible duplicate marker (dashboard badge / detail-page chip)
- **Pros:** Stays visible if the user doesn't notice it immediately.
- **Cons:** Rejected. No dismiss affordance exists anywhere in this
  codebase today (checked: no template/handler has one), and inventing
  one — plus a new visual element — is a bigger lift than the backlog
  item calls for. A one-shot flash message, the same convention already
  used for "Uploaded — extracting text now," needs no new UI concept and
  no mockup (a copy/color change to an existing message class).

### Alternative D: Pass which document matched via a `?duplicate_of=` redirect query param
- **Pros:** Simple for the direct desktop-upload path, where the
  redirect happens in the same request that just computed the hash.
- **Cons:** Rejected. The phone-scan path's redirect doesn't work that
  way — `finish_scan` (the request that actually inserts the document)
  runs on the *phone*, while the desktop browser discovers the new
  document via a *separate*, later polling request that only then
  redirects to `/documents/{id}?uploaded=true`. A query param would have
  to be persisted somewhere (on `scan_sessions`, say) just to survive
  that gap — extra state for something the database can already answer
  directly.

### Alternative E: `show()` performs its own "does a duplicate exist?" lookup whenever `uploaded=true`
- **Pros:** Both ingestion paths already redirect to the exact same
  `/documents/{id}?uploaded=true` URL shape — `uploaded=true` already
  means "you just got here from creating this document," so gating a
  live lookup on that flag is a one-shot check for free, with no new
  query param and no path-specific plumbing. Since `content_hash` is
  computed *before* either path's document row is inserted (see §3), the
  column is guaranteed populated by the time any redirect — immediate or
  polled — lands on the show page.
- **Cons:** One extra `select` on `show()`, but only when `uploaded=true`
  (i.e., once per document, ever) — negligible.

## 3. Structural Decision
We choose **Alternative B** (warn, never block) + **Alternative E** (a
live lookup gated on the existing `uploaded` flag, no new query param).

**Schema**: `documents.content_hash text` (nullable — historical rows
stay `null` until reprocessed, same precedent as `thumbnail_status`/
`ocr_status`), plus a **non-unique** index on `(tenant_id, content_hash)`
— non-unique because Alternative B explicitly allows duplicates to
exist; a unique constraint would reject the very inserts this feature
is designed to let through with just a warning.

**Hashing, hex-encoded SHA-256** (matching `web::forms::hash_hex_token`'s
existing local idiom): computed once per ingestion path, synchronously,
before the document row is inserted, so it's always ready in time for
the very first `show()` render:
- **Desktop upload** (`stream_document_to_blob` → `BlobStore::
  stream_upload`): the streaming upload already reads the multipart
  field in a chunked loop to track a running byte count for the
  `max_bytes` check — a `Sha256` hasher updates in that same loop,
  finalized once the stream ends. No second read of the bytes.
- **Phone scan** (`finish_scan`): the assembled PDF's bytes are already
  fully in memory (`pdf_bytes`, built by `pdf_assemble.rs`) before the
  upload call — a single `Sha256::digest(&pdf_bytes)` there.
- **Reprocess** (`run_ocr`, feature 013's existing background task,
  shared by both a fresh OCR-eligible upload and a manual reprocess):
  already re-fetches the full blob via `get_object` for OCR — the same
  bytes get hashed there too, unconditionally overwriting `content_hash`
  on every pass (not `coalesce`d — nothing user-editable can conflict
  with it, so simply keeping it in sync with the actual blob is both
  simpler and self-healing). This is what backfills `content_hash` for
  any document uploaded before this feature shipped, the moment it's
  next reprocessed for any other reason (AC-7).

**Lookup**: `show()`, only when `query.uploaded` is `true` and the
current document's `content_hash` is `Some`, runs one extra query — the
oldest other document in the same tenant sharing that hash:
```sql
select id, title, original_filename, created_at
from documents
where tenant_id = $1 and content_hash = $2 and id != $3
order by created_at asc
limit 1
```
A hit renders the existing `document_show.html` flash-message slot with
the `form-error`/`confirm-warning` (red/`--danger`) styling already used
elsewhere, instead of adding a new visual treatment.

## 4. Explicitly Deferred
- **Perceptual/near-duplicate matching** — see backlog §3.
- **Bulk backfill sweep** for pre-existing documents — see backlog §3;
  `run_ocr`'s reprocess path is the only backfill mechanism.

## 5. OpenTelemetry Implications
No new spans. `content_hash` is a content fingerprint, not the content
itself, but — same rule already applied to `ocr_text`/free-text search —
it's kept out of any span regardless; the handlers that touch it
(`show`, `run_ocr`, `stream_document_to_blob`) already skip their
bulkier parameters (`state`, `tenancy`/`blob_bytes`) for the same reason
and this fits the same pattern, not a new one.
