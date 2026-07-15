# TDR 022: Multi-Page Phone Scan

## 1. Context & Architectural Requirements
Feature 009's phone-camera handoff is strictly one photo per QR code — the
first `POST /scan/:token` creates a document and burns the session. The
multi-page limitation has been on ARCHITECTURE §8's deferred list since
009 shipped; the user picked it up 2026-07-15. Mockup signed off the same
day (artifact `3de2eb9c-cb14-4287-a366-8e771b21b420`) before any
template/handler work, per CLAUDE.md §5: capture-repeat-finish on the
phone, a "page ledger" progress strip on both sides, Finish as the quiet
outline action, QR hidden once pages exist, sliding expiry.

Hard requirements carried over from 009: no client-side JS (meta-refresh
polling only), phone-side tenancy resolved from the hashed token's
`scan_sessions` row (the documented `TenantContext` exception, TDR 009
§3), raw image bytes out of spans, zero-panic. New here: N phone photos
must become **one** document that the rest of the system can't tell apart
from a desktop-uploaded PDF — feature 010's OCR path already rasterizes
PDFs per page, so "combine into a PDF" means zero OCR-pipeline changes.

## 2. Alternatives Evaluated

### Alternative A: Accumulate pages client-side, upload once
- **Pros:** No interim server state; one POST.
- **Cons:** Requires JS (a file-list UI, or MediaStream capture) — the
  exact surface TDR 009 Alternatives C/E/G already rejected three times.
  The native `<input capture>` hands over one photo per camera round-trip
  by design.

### Alternative B: N documents plus a "bundle" grouping concept
- **Pros:** No PDF assembly; each page is an ordinary document.
- **Cons:** Changes the core data model (documents gain parenthood) and
  every downstream surface — list, facets, OCR text, collections — has to
  learn about bundles. A five-page contract *is* one document; modeling it
  as five with a paperclip is the wrong shape, and the backlog explicitly
  asks for "one entry in my archive".

### Alternative C (chosen): Server-side page accumulation, finalized into one PDF
- **Pros:** Each capture is exactly feature 009's upload (multipart POST,
  streamed to blob storage) — just to a per-session page key instead of a
  document key. Finish assembles the pages into a single PDF and pushes it
  through the **existing** ingest path (`insert_document_and_queue_ocr`
  with `application/pdf`), so OCR, dashboards, filters, and detail pages
  need zero changes. Interim pages live in blob storage (a new
  `scan_pages` table records order + keys), not in Postgres blobs or local
  disk.
- **Cons:** Interim state to manage: an abandoned session strands page
  blobs. Accepted at personal scale and explicitly deferred (backlog 022
  §3) — finalize deletes page blobs best-effort; expired leftovers are
  small and bounded by how often a human abandons a scan mid-way.

---

### Alternative D: Assemble the PDF by shelling out to `img2pdf`
- **Pros:** Purpose-built, losslessly embeds JPEG; matches the
  tesseract/pdftoppm CLI precedent (TDR 008/010).
- **Cons:** It's a Python tool — `apt-get install img2pdf` drags
  python3 + pikepdf + qpdf into the runtime image (~80-100 MB on
  bookworm-slim, which currently ships no Python at all), onto every dev
  machine, and onto the CI runner. The CLI precedent existed because
  Tesseract/Poppler have no credible Rust equivalents; PDF *writing* does.

### Alternative E (chosen): Pure-Rust assembly with `lopdf` (+ `image` for metadata/PNG)
- **Pros:** Matches TDR 009 Alternative H's *other* precedent (`qrcode`:
  pure-Rust crate, no system dependency, nothing added to the
  Dockerfile). JPEG pages — what phone cameras actually produce — are
  embedded **byte-for-byte** (DCTDecode stream), no recompression; the
  `image` crate only reads their dimensions. PNG pages (accepted since
  009, rare in practice) are decoded and re-encoded as JPEG q90 rather
  than hand-rolling PNG predictor→FlateDecode embedding.
- **Cons:** Two new Cargo dependencies and ~150 lines of owned PDF
  construction code (`src/pdf_assemble.rs`) instead of a maintained
  tool's. Accepted: the construction is a fixed, minimal shape (one
  image XObject per page, one `Do` content stream), covered by its own
  test that reparses the output with `lopdf` and asserts the page count.

**Page geometry:** each PDF page is sized at `pixels × 72/150` points.
`extract_text_from_pdf` runs `pdftoppm` at its default 150 DPI, so this
factor makes OCR rasterization reproduce the original photo's pixel
dimensions almost exactly — no resolution lost to a small page box, no
memory wasted rasterizing an inflated one.

## 3. Structural Decision
We choose **C + E**.

**Migration** (`create_scan_pages` + widen status): new `scan_pages`
(`id uuid pk`, `scan_session_id uuid fk → scan_sessions on delete
cascade`, `page_number int` — unique per session — `blob_key text`,
`content_type text`, `file_size_bytes bigint`, `created_at timestamptz`);
`scan_sessions.status` CHECK widened from `('pending','captured')` to
include `'capturing'` (≥1 page, not finished).

**`submit_scan` (`POST /scan/:token`) repurposed to append:** resolves a
`pending` *or* `capturing` unexpired session; streams the photo to blob
key `scan-pages/{user_id}/{session_id}/{page_number}`; inserts the
`scan_pages` row; flips the session to `capturing` **and slides
`expires_at` to `now() + 10 minutes`** (AC-5) in one guarded UPDATE.
Renders the phone "page added" state. No document is created here
anymore.

**New `finish_scan` (`POST /scan/:token/finish`, public group like its
siblings):** resolves a `capturing` unexpired session with its ordered
pages; downloads each page blob; assembles the PDF
(`pdf_assemble::images_to_pdf`); uploads it via a new
`BlobStore::upload_bytes` (plain `put_object` — the existing
`stream_upload` is shaped around a live multipart `Field`); calls the
existing `insert_document_and_queue_ocr` (`application/pdf`,
`phone-scan-{n}-pages.pdf`, no title/tags/date, exactly like 009); then
the 009-style guarded finalize: `update scan_sessions set
status='captured', document_id=$2 where id=$1 and status='capturing'`.
An early status check makes the common double-tap re-render the
"captured" state without re-assembling; a genuinely simultaneous pair of
finishes remains the same accepted, undefended race 009 documented for
double-submit. Page blobs are deleted best-effort after finalize;
`scan_pages` rows stay (they're how the captured screens know the page
count).

**Phone template states** (`ScanPhoneState`): `Capture` (pending, no
pages), `Capturing { page_count }` (the signed-off decision screen: page
ledger, capture-next form as `btn-stamp`, finish form as `btn-outline` —
two sibling `<form>`s, never nested), `Captured { page_count }`,
`Invalid`. **Desktop `new_scan`**: `pending` renders the QR exactly as
today; `capturing` renders the progress card (QR deliberately gone —
spent, and hiding it stops a second device joining mid-session);
`captured` redirects as today. Page counts ride the existing
meta-refresh; no new polling idiom.

**Single-page compatibility (AC-3):** one capture + Finish produces a
one-page PDF. 009's "photo in, `image/jpeg` document out" shape is gone
on this path by design — the scan entry point now always yields a PDF.

## 4. OpenTelemetry Implications
`submit_scan` keeps `skip(state, multipart)`; `finish_scan` takes no
body but skips `state` and never logs page bytes — downloaded page
buffers and the assembled PDF stay out of span fields entirely (the
assembler is *not* `#[instrument]`ed on its byte-carrying arguments,
only wrapped by the handler span). The raw token stays out of spans as
in 009; `scan_sessions.id`, page numbers, and page counts are the only
new correlation attributes — none are PII. The spawned OCR task is
feature 008/010's existing instrumented path, unchanged.

## 5. Test Strategy
New `tests/scan_multipage.rs` (reusing `scan_flow.rs`'s helper
patterns): two pages then finish → exactly one document,
`content_type = application/pdf`, blob bytes start `%PDF`, `ocr_status =
'pending'`, session `captured` with `document_id`; single page + finish;
finish with zero pages rejected; sequential double-finish creates no
second document; desktop `GET /scan?token=` shows the page count while
`capturing`; a page capture extends `expires_at` (read back from the
DB). `pdf_assemble` gets a direct test: two fixture images in, `lopdf`
reparse out, page count 2. Existing `scan_flow.rs` tests are updated
where 009's one-shot semantics changed (a lone POST no longer creates a
document) — deliberate behavior change, not regression.
