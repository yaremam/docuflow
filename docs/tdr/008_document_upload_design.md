# TDR 008: Document Upload & Tesseract OCR

## 1. Context & Architectural Requirements
Feature 007 (the `/documents` dashboard) built list/search/sort/view/edit
over documents that already exist in the database, deliberately deferring
how documents get there in the first place — its schema already carries
`ocr_status`/`ocr_text`/`ocr_error` so this feature needs no migration.
This is also the first feature to actually implement CLAUDE.md's "OCR
Engine Layer" section ("File parsing must run as decoupled asynchronous
workers using Tokio background green threads... Spans must track heavy
data transformations... keep raw byte arrays out of the logging targets"),
which nothing before this exercised.

## 2. Alternatives Evaluated

### Alternative A: An FFI OCR crate (e.g. bindings to libtesseract/leptonica)
- **Pros:** In-process calls, no subprocess overhead, structured error
  types from the binding crate itself.
- **Cons:** Requires `libtesseract-dev`/`libleptonica-dev` headers and
  linking at *build* time (not just runtime), on top of the shared
  libraries at runtime — meaningfully more Docker/build-stage surface than
  a single `apt-get install tesseract-ocr` in the runtime stage. No such
  binding crate is already a dependency of this project.

### Alternative B: Shell out to the `tesseract` CLI via `tokio::process::Command` (chosen)
- **Pros:** `tokio`'s already-enabled `"full"` feature includes `"process"`
  — zero new Cargo dependency. The project's only new system-level
  requirement is one binary on `PATH` in the runtime image (`tesseract-ocr`
  added to `Dockerfile`'s existing `apt-get install` line, the same
  treatment already given `ca-certificates`/`libssl3`), nothing at build
  time. Matches this project's general preference for minimal dependency
  footprints (see TDR 003 §2 on `argon2`/`tower-sessions` vs. heavier
  alternatives).
- **Cons:** Subprocess spawn overhead per document (acceptable — this
  always runs as detached background work, never on the request path);
  output is plain text on stdout rather than a structured result type, so
  errors are string-based (`OcrError(String)`).

**Chosen: Alternative B.**

---

### Alternative C: Pipe image bytes to `tesseract` over stdin (`tesseract - stdout`)
- **Pros:** No temp file, no filesystem cleanup to manage.
- **Cons:** TIFF (one of the four accepted upload types) has a history of
  trouble in Leptonica/tesseract's stdin (`-`) path for non-seekable
  input — a real file path avoids that uniformly across all four accepted
  types rather than needing per-format branching.

### Alternative D: Write to a real temp file, RAII-cleaned (chosen)
- **Pros:** Uniform handling across all accepted image types. Cleanup
  structured as a `Drop` guard (`ocr::TempFile`) rather than "remove at
  the end of the function" — correctness doesn't depend on every fallible
  early-return point remembering to also clean up.
- **Cons:** A brief window where a tenant's document bytes sit on disk —
  mitigated with `0600` permissions and a `Uuid`-based filename (no
  predictability/collision risk), and the window is only as long as the
  `tesseract` subprocess runs.

**Chosen: Alternative D.**

---

### Alternative E: A job-queue table (e.g. `ocr_jobs` polled by a worker loop)
- **Pros:** Durable across restarts — a crash mid-job doesn't lose the
  work, just delays it; naturally supports retry/backoff.
- **Cons:** Meaningfully more machinery (a new table, a polling worker,
  lease/visibility-timeout semantics) than this feature's actual
  requirement calls for at current scale, and CLAUDE.md's own OCR Engine
  Layer wording — "decoupled asynchronous workers using Tokio background
  green threads" — describes exactly Alternative F below, not a queue.

### Alternative F: Detached `tokio::spawn` per upload, matching the existing fire-and-forget mail-send pattern (chosen)
- **Pros:** Reuses the exact pattern already established in
  `handlers/password_reset.rs::forgot_password_submit` (`tokio::spawn(...
  .instrument(tracing::Span::current()))`) — no new architectural idiom
  introduced. Directly satisfies CLAUDE.md's wording. A `Semaphore`
  (`AppState.ocr_semaphore`, capacity 2) caps concurrent `tesseract`
  subprocesses so a burst of uploads can't exhaust the box also serving
  requests.
- **Cons:** Not durable — axum's graceful shutdown only drains in-flight
  HTTP connections, not detached spawned tasks (confirmed via
  `src/main.rs`), so a restart mid-OCR can strand a row at
  `ocr_status = 'processing'` forever. Mitigated with a boot-time sweep in
  `state::migrate` (`update documents set ocr_status = 'pending' where
  ocr_status = 'processing'`) — a half-fix, explicitly documented as such:
  it clears the stuck flag but doesn't retry the actual extraction, and it
  assumes a single running instance (would incorrectly steal another live
  instance's in-flight row if this app is ever horizontally scaled).
  Accepted as the right tradeoff for current scale; Alternative E is the
  natural next step if durability/retry becomes a real requirement.

**Chosen: Alternative F.**

---

### Alternative G: Accept only image types this round; reject PDF entirely
- **Pros:** Simplest possible scope — one code path, one set of accepted
  types.
- **Cons:** A large fraction of real bills/statements/contracts arrive as
  PDFs (downloaded or emailed), and Feature 007's `document_show.html`
  already ships a "Text extraction isn't available for this file type
  yet." placeholder that a PDF-rejecting scope would leave unused.

### Alternative H: Accept images (OCR'd) and PDF (stored, OCR marked `'skipped'`) (chosen)
- **Pros:** Lets users file away PDFs today (view/download, search by
  manually-entered tags/title) without blocking on the larger
  page-rasterization work full PDF OCR would need; the "skipped" state
  and its UI copy already existed from Feature 007, so this is the
  intended use of that placeholder, not a new concept. Confirmed directly
  with the user before implementation.
- **Cons:** A PDF's own text content isn't searchable via the tag system
  until a future feature adds real PDF OCR.

**Chosen: Alternative H.**

## 3. Structural Decision
No migration — schema is unchanged from Feature 007.

`src/ocr.rs` (new module) exposes `extract_text(bytes: &[u8]) ->
Result<String, OcrError>`: writes to `std::env::temp_dir()` via
`tokio::fs::write` (not `std::fs::write`, to avoid blocking the async
runtime thread) with `0600` permissions and a `Uuid`-based filename,
cleaned up by an RAII `TempFile` guard; runs
`tokio::process::Command::new("tesseract").arg(path).arg("stdout")`,
returning stdout as the extracted text on a zero exit status.

`BlobStore::stream_upload` (`src/blob.rs`) now returns
`Result<usize, BlobError>` (the total byte count it already tracked
internally) instead of `Result<(), BlobError>`, so the upload handler can
populate `file_size_bytes` without a second read; the one existing caller
(`profile::upload_picture`) discards the value. `BlobStore` also gains
`get_object(key) -> Result<Vec<u8>, BlobError>`, a plain buffered GET used
only by the detached OCR task to re-read an already-uploaded, already
size-bounded (≤20MB) object — deliberately not shared with the streaming
upload path, keeping "bounded streaming upload" and "backgrounded
full-buffer OCR read" as separate concerns rather than making the request
handler hold a whole file in memory to save one background-only round
trip.

`web::handlers::documents::create` (`POST /documents`) parses its
multipart body by explicit field-name dispatch, not assumed order: a
`title`/`tags`/`date_issued` field arriving after `"file"` has already
been consumed is rejected with `400` (an enforced contract, not a silently
dropped value) — the HTML form places those fields before the file input
so the common case never hits it, but a hand-crafted or reordered request
gets a clear rejection instead of silently losing data. Metadata fields
are validated by hand via `ProfileField`/`Tags`/`DateIssuedField`'s
existing `TryFrom<String>` impls (this isn't going through the `Form`
extractor's automatic serde validation, so it has to be done explicitly).
The document's `id` is minted once and reused as the blob-key suffix
(`documents/{tenant_id}/{id}`) rather than a second UUID.

`AppState` gains `ocr_semaphore: Arc<tokio::sync::Semaphore>`. The
detached OCR task's final `UPDATE`s (both the success and failure branch)
filter on `id = $1 AND tenant_id = $2`, matching the tenant-scoping
convention every other query in this file already follows, even though no
delete-document feature exists yet to make a race there reachable.

## 4. OpenTelemetry Implications
`new_form` and `create` carry `#[tracing::instrument(skip(state, tenancy,
multipart))]`; the detached `run_ocr` task carries
`#[tracing::instrument(skip(state))]` and is spawned via
`.instrument(tracing::Span::current())`, matching
`forgot_password_submit`'s existing pattern — so OCR failures logged
inside the task still correlate back to the request span that queued
them. No document title, tag, raw file bytes, or extracted OCR text is
ever captured as a span attribute or log field; only the extraction
*error message* (not the text itself) is logged on failure, and only to
`tracing::error!`, never stored beyond `documents.ocr_error`.
