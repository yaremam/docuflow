# TDR 025: Dashboard Thumbnails + Side-by-Side Document Preview

## 1. Context & Architectural Requirements
Backlog item: dashboard thumbnails, and a document-page preview shown
side by side with its OCR text (explicitly earmarked in ARCHITECTURE.md
§8 as the future partner for feature 023's full-text search — search-hit
highlighting is deferred again here, see §3). Before this feature,
`document_preview` (`src/web/handlers/documents.rs`) was a pure
passthrough: it presigned the *original* blob and let the browser scale
it down via CSS for the dashboard row, and PDFs got a static generic
icon, never a real preview. Per CLAUDE.md: zero-panic, streaming/bounded-
memory blob handling, no new PII surface.

## 2. Alternatives Evaluated

### Alternative A: Generate a real thumbnail in the existing OCR background task, reusing bytes it already has
- **Pros:** `run_ocr` already downloads the full file (`state.blob.
  get_object`) for direct image uploads, and already rasterizes PDF page 1
  via `pdftoppm` (`ocr::extract_text_from_pdf`) to run tesseract against
  it. Generating a thumbnail from those same in-memory bytes needs no new
  blob fetch, no new PDF rasterization pass, and no new background job or
  semaphore — it rides the same `state.ocr_semaphore`-bounded task feature
  008 already established for exactly this "don't do heavy work inline in
  a request handler" reason (CLAUDE.md's OCR Engine Layer rule).
- **Cons:** `ocr::extract`'s return type has to change (`String` →
  `(String, Vec<u8>)`) to hand the raster bytes back out — a small,
  contained signature change with exactly one caller (`run_ocr`).

### Alternative B: A separate thumbnail-generation background job, triggered independently of OCR
- **Pros:** Decouples thumbnail generation from OCR's success/failure.
- **Cons:** Rejected — a second blob fetch (or a second PDF rasterization
  pass) for information `run_ocr` already has in memory moments earlier is
  pure waste at this app's scale, and a second semaphore/job-tracking
  column is more machinery for no real benefit over Alternative A.

### Alternative C: Resize on the fly at request time (no stored thumbnail blob)
- **Pros:** No extra blob storage, no `thumbnail_status` column.
- **Cons:** Rejected — re-decoding and re-resizing a full-size image (or
  re-rasterizing a PDF) on every dashboard render is repeated, unbounded
  work per request; a background-generated, stored thumbnail is a fixed
  cost paid once.

### Alternative D: `documents.thumbnail_status`, mirroring `ocr_status`, plus a deterministic derived blob key
- **Pros:** No live "does this key exist" blob `HEAD` check on every
  dashboard render — the status column alone tells the template whether
  to use the thumbnail or fall back, same shape as `ocr_status`'s
  `pending`/`processing`/`done`/`failed` states (only 
  `pending`/`done`/`failed` are actually reachable here — there's no
  separate "processing" phase distinct from OCR's own). A fixed key
  suffix (`{blob_key}-thumb`) means no new column is needed to *find* the
  thumbnail, only to know whether it's ready.
- **Cons:** None identified — this is the same tradeoff `ocr_status`
  already made for the OCR pipeline.

## 3. Structural Decision
We choose **Alternative A** for generation timing/data reuse and
**Alternative D** for tracking/storage.

**`ocr::extract`** (`src/ocr.rs`) now returns `Result<(String, Vec<u8>),
OcrError>` — the text, plus raster bytes suitable for thumbnailing: the
original bytes for a direct image upload (`extract_text` case), or PDF
page 1's already-rasterized PNG (`extract_text_from_pdf`, read back off
disk right before its temp dir is cleaned up).

**`src/thumbnail.rs`** (new, pure/unit-testable): `generate(bytes: &[u8])
-> Result<Vec<u8>, ThumbnailError>` — decodes with the format
auto-detected from the byte signature (`image::ImageReader::
with_guessed_format`, so it doesn't need to know the source content type),
`.thumbnail(200, 200)` to resize preserving aspect ratio, re-encodes as an
80%-quality JPEG. Mirrors `pdf_assemble.rs`'s existing decode-then-
re-encode-as-JPEG pattern rather than a second image-handling approach.
Needed adding `webp`/`tiff` to the `image` crate's enabled features
(`Cargo.toml`) — previously only `jpeg`/`png` were on, but two of the four
directly-uploadable content types are `webp`/`tiff`.

**Schema**: `documents.thumbnail_status text` (nullable). `run_ocr` calls
a new `generate_and_store_thumbnail` helper right after a successful OCR
pass, which uploads the resized JPEG via the existing `BlobStore::
upload_bytes` under `thumbnail_blob_key(blob_key)` (`format!("{blob_key}
-thumb")`) and returns `"done"`/`"failed"` for the same `UPDATE` that
already writes `ocr_status = 'done'`. A thumbnail failure is logged and
recorded but never fails the OCR pass itself — best-effort, matching the
"a wrong/missing guess is a fallback, not an error" spirit already
established for the date/doc_type suggestions.

**Serving**: `document_preview` (shared by `list`/`show`) gains a
`thumbnail_status: Option<&str>` parameter and returns an additional
`Option<String>` presigned thumbnail URL, only `Some` when the status is
`"done"`. `documents_list.html`'s per-row thumbnail now prefers this URL
(for *both* images and PDFs) before falling back to the pre-025 rendering
(inline `<img>` for direct images, generic PDF icon otherwise) — so a
document with `thumbnail_status` still `null`/`"failed"` looks exactly as
it did before this feature.

**Side-by-side layout**: `document_show.html`'s two stacked cards
("Original file", "Extracted text" — previously both children of one
`.detail-text` column) become two independent siblings of `.detail-meta`
inside `.detail-grid`. `static/style.css`: ≥800px lays out preview+OCR
text side by side (metadata wraps to a full-width row below); ≥1100px
fits all three as one row. Below 800px, everything stays stacked
single-column, unchanged from before.

## 4. Explicitly Deferred (see also ARCHITECTURE.md §8)
- **Search-hit highlighting/snippets** in the OCR text box — this was the
  reason ARCHITECTURE.md paired this feature with feature 023, but no
  query-term-context threading is added here; it's a distinct follow-up.
- **Thumbnails for documents uploaded before this feature shipped** — no
  backfill sweep; `thumbnail_status` stays `null` until a document is
  reprocessed (feature 013's existing per-document button), same
  historical-rows precedent `ocr_status`/`ocr_suggested_doc_type` already
  have.

## 5. OpenTelemetry Implications
No new spans, no new parameter entering any `#[tracing::instrument]`'d
function's captured args: `generate_and_store_thumbnail`'s new
`#[tracing::instrument(skip(state, thumbnail_source))]` explicitly skips
the one new byte-buffer parameter it takes, so raw image bytes never
reach a span. Spot-checked in Jaeger, not just assumed.
