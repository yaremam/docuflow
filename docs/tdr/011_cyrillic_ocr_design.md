# TDR 011: Cyrillic OCR Support

## 1. Context & Architectural Requirements
`src/ocr.rs::run_tesseract` invokes `tesseract <path> stdout` with no `-l`
flag, so Tesseract falls back to its default `eng` trained-data model for
every image and every rasterized PDF page. Documents containing Cyrillic
text (Russian bills, Ukrainian contracts, etc.) come back as garbled or
empty OCR text. We have no document-language field yet (that's a separate,
not-yet-built backlog item) and no per-document language selection at
upload time, so whatever this feature does must work without knowing the
document's language in advance.

## 2. Alternatives Evaluated

### Alternative A: Always run Tesseract in `eng+rus` multi-language mode
- **Pros:** One-line change (`-l eng+rus` on the existing `tesseract`
  invocation in `run_tesseract`) — Tesseract's multi-language mode loads
  both trained-data models and picks the best-matching script per line/
  block internally, so no upfront language detection or per-document
  metadata is needed. Works uniformly for the existing single invocation
  point (`run_tesseract`), which both `extract_text` and the PDF per-page
  loop already funnel through, so PDFs get Cyrillic support for free.
- **Cons:** Slightly slower per-page OCR (two models loaded and
  considered instead of one) and a small chance of a rare pathological
  document being mis-recognized as the wrong script where single-language
  mode would have guessed right. Neither is a concern at DocuFlow's
  current per-document, background-worker OCR volume.

### Alternative B: Detect language first, then pick a single Tesseract model
- **Pros:** Avoids the (minor) dual-model overhead; OCR always runs in
  the "correct" single-language mode.
- **Cons:** Requires a separate language-detection step before OCR even
  runs — either a second Tesseract pass (defeats the purpose) or a new
  dependency (e.g. a language-detection crate/CLI) that only exists to
  answer a question Tesseract's own multi-language mode already answers
  internally per-block. Adds a new failure mode (detection fails/is
  ambiguous on a short or mixed-script document) with no clear fallback
  other than "try both anyway," which is just Alternative A with extra
  steps.

### Alternative C: Add a document-language field, ask the user at upload
- **Pros:** Most accurate — the user simply tells us the language.
- **Cons:** This is explicitly the separately-tracked "add document
  language field" backlog item (its own UI, its own migration, its own
  mockup sign-off per CLAUDE.md's UI process). Bundling it into this
  feature would block a small OCR fix behind a larger metadata-and-UI
  feature it doesn't need to depend on.

## 3. Structural Decision
We choose **Alternative A**: change `run_tesseract`'s invocation to pass
`-l eng+rus`, and add the `tesseract-ocr-rus` trained-data package next to
`tesseract-ocr` in `Dockerfile`'s runtime stage (and document it as a local
dev/CI dependency, matching the existing `tesseract`/`pdftoppm` `PATH`
precedent). Because both `extract_text` (direct image uploads) and
`extract_text_from_pdf`'s per-page loop already call the same
`run_tesseract` helper, this single change covers both OCR paths with no
per-caller branching. If `rus` trained data is missing from `PATH`'s
tessdata directory, Tesseract exits non-zero on startup, which
`run_tesseract` already maps to `OcrError` -> `ocr_status = 'failed'` —
no new panic surface.

## 4. OpenTelemetry Implications
No new spans or attributes. This is a one-argument change inside the
existing `run_tesseract` function, which already runs inside
`ocr.rs::extract`/`extract_text`/`extract_text_from_pdf`'s existing
instrumentation boundaries — no new PII surface, no new span.
