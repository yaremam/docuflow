# TDR 010: PDF Rasterization Strategy for OCR

## 1. Context & Architectural Requirements
Feature 008 accepts `application/pdf` uploads but marks them
`ocr_status = 'skipped'` — no text is ever extracted. `src/ocr.rs`
extracts text from raster images only, by shelling out to the `tesseract`
CLI binary (deliberately, per that module's own doc comment, to avoid
linking against `libtesseract`/`leptonica`). To OCR a PDF we first need to
turn each page into a raster image Tesseract can consume; no PDF- or
image-handling crate exists in `Cargo.toml` yet. Per CLAUDE.md, the
rasterization + OCR pass must run as a decoupled Tokio background worker,
must not panic on a malformed file, and must keep page bytes and raw text
out of trace/log output.

## 2. Alternatives Evaluated

### Alternative A: Shell out to `pdftoppm` (poppler-utils), one process per PDF
- **Pros:** Mirrors the exact pattern already established for Tesseract —
  a battle-tested CLI tool, invoked via `tokio::process::Command`, with no
  new native linking in the Rust build itself. `pdftoppm` writes one PNG
  per page directly to a temp directory, which then feeds straight into
  the existing `extract_text` per-page. Failure modes (corrupt/encrypted
  PDF, zero pages) surface as a non-zero exit code or missing output
  files, easy to map to `ocr_status = 'failed'`.
- **Cons:** Adds a second external binary dependency (poppler-utils)
  alongside `tesseract` that must be present on `PATH` in every
  environment (dev machine, CI, container image) — one more thing to
  document and install. Multi-page PDFs mean spawning one process and
  managing N temp files per document.

### Alternative B: In-process rasterization via `pdfium-render`
- **Pros:** No external CLI process to spawn or manage; a typed Rust API
  for page count and rendering; one dependency line in `Cargo.toml`
  instead of a system package to install.
- **Cons:** `pdfium-render` still requires the native `libpdfium` shared
  library at runtime (bundled or downloaded per-platform) — it does not
  actually avoid native linking, it just repackages it as a
  prebuilt-binary dependency with its own per-architecture packaging
  story. This cuts directly against the precedent set in `ocr.rs`
  (explicitly choosing CLI-shelling over native Tesseract bindings to
  dodge exactly this class of build complexity), and would introduce a
  second, inconsistent way of vendoring a native OCR/PDF dependency in
  the same codebase.

## 3. Structural Decision
We choose **Alternative A (shell out to `pdftoppm`)**, to stay consistent
with the precedent already set for Tesseract: prefer a well-tested CLI
tool over a native-library Rust binding, keeping the dependency story
uniform (two documented `PATH` binaries — `tesseract` and `pdftoppm` —
rather than one CLI tool and one bundled native library). The background
worker will, for `application/pdf` documents: write the uploaded bytes to
a temp file, invoke `pdftoppm -png` to rasterize each page to a temp PNG,
run the existing `extract_text` over each page image in page order, join
the results with a page-separator marker, and write the combined string
to `ocr_text`. Any failure (non-zero exit, zero output pages) maps to
`ocr_status = 'failed'` with a sanitized `ocr_error`, never a panic. Local
dev setup and CI images need `poppler-utils` installed alongside
`tesseract`.

## 4. OpenTelemetry Implications
The rasterization step lives inside the existing `run_ocr`
`#[tracing::instrument(skip(state))]` span (`documents.rs:437`), so no new
top-level span is introduced. Any new helper function that takes PDF
bytes or a page image must use `#[tracing::instrument(skip(...))]` on
those parameters, matching the existing `ocr.rs::extract_text` precedent
— page count is safe to record as a span attribute, but page image bytes
and extracted text are never attached to spans or logs.
