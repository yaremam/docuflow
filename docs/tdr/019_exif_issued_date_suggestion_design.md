# TDR 019: EXIF-Based Issued-Date Suggestion

## 1. Context & Architectural Requirements
Feature 012 scans OCR text for a date and writes it to
`documents.ocr_suggested_date_issued`, guarded so it never overwrites an
existing `date_issued`. This feature adds a second candidate source: a
photographed document's own EXIF capture timestamp. The two sources are
not equally trustworthy — an EXIF `DateTimeOriginal` is *when the photo
was taken*, not necessarily *when the document was issued* (photographing
a bill the day it arrives is common, but not guaranteed), whereas an
OCR-extracted date came from text actually printed on the document
itself. So EXIF is a **fallback**, only used when OCR text yielded
nothing (AC-2) — not a competing or averaged signal. Per CLAUDE.md:
zero-panic (missing/corrupt EXIF is not an error), PII-safe spans (raw
EXIF data, which can include GPS coordinates, never enters a span), and
`#[tracing::instrument(skip(file_bytes))]`-style byte-array hygiene for
any new function touching raw image bytes.

## 2. Alternatives Evaluated

### Alternative A: `kamadak-exif` (crate name `exif`), read-only, folded into `run_ocr`'s existing success branch as a fallback when OCR found no date
- **Pros:** `kamadak-exif` is the most established pure-Rust EXIF reader
  (widely used, no `unsafe`, minimal dependency footprint — just
  `mutate_once`). Reusing `ocr_suggested_date_issued` and the existing
  suggestion UI means zero new column, zero new template/endpoint code —
  this is genuinely additive to feature 012's pipeline, not a parallel
  one. `run_ocr` already has the raw file bytes in scope (fetched from
  blob storage before OCR runs) — no second blob fetch needed.
- **Cons:** The column is still literally named `ocr_suggested_date_issued`
  even though it can now hold an EXIF-sourced value — a naming wrinkle,
  not renamed in this round (see §3).

### Alternative B: A dedicated `exif_suggested_date_issued` column, its own suggestion UI ("From your photo: ...")
- **Pros:** Precise about a suggestion's source; lets OCR and EXIF
  candidates coexist and be shown/accepted independently.
- **Cons:** A second suggestion box competing for the same UI real estate
  as feature 012's, immediately raising "what if both exist, which do we
  show first" — exactly the priority question Alternative A already
  answers simply (OCR wins, EXIF is silent fallback). Meaningfully more
  UI/schema work for a "quick" backlog item with no signal this
  precision is actually wanted. Rejected for this round; the column
  could still be split out later if source-attribution ever becomes a
  real ask.

### Alternative C: `little_exif` (reader **and** writer) as the production dependency
- **Pros:** One crate instead of needing a second tool to *generate* an
  EXIF-bearing test fixture (this sandbox has no ImageMagick/PIL to do
  that any other way — see §4).
- **Cons:** `kamadak-exif` is the more established, widely-depended-on
  reader for this exact job; pulling in write support the production
  code never uses just to reuse one crate for two purposes isn't a
  reason to prefer the less-established option. Rejected: `little_exif`
  is used only as a one-off, outside-the-repo scratch tool to *generate*
  the committed test fixture bytes (§4) — it never becomes a project
  dependency.

## 3. Structural Decision
We choose **Alternative A**. New `src/exif_extract.rs`:
```rust
pub fn extract_issued_date(content_type: &str, bytes: &[u8]) -> Option<time::Date>
```
Parses via `exif::Reader::new().read_raw(bytes.to_vec())` (or the
equivalent byte-slice entry point), looks up `Tag::DateTimeOriginal`
first, falling back to `Tag::DateTime` if absent, and parses the EXIF
ASCII format (`"YYYY:MM:DD HH:MM:SS"`, colons in the date portion, not
dashes — distinct from `date_extract`'s ISO-dash parsing) into
`time::Date`. Any parse failure, missing tag, or unsupported container
returns `None` — never a panic, matching `date_extract::
extract_issued_date`'s existing "no confident match, no forced guess"
contract (TDR 012 §3).

`run_ocr` (`src/web/handlers/documents.rs`) is restructured so the raw
`bytes` fetched from blob storage stay in scope alongside `outcome` (today
they're consumed inside the `match` that produces `outcome`); on a
successful OCR pass:
```rust
let ocr_suggested_date_issued = crate::date_extract::extract_issued_date(&text);
let exif_suggested_date_issued = bytes.as_ref().ok()
    .and_then(|bytes| crate::exif_extract::extract_issued_date(&content_type, bytes));
let suggested_date_issued = ocr_suggested_date_issued.or(exif_suggested_date_issued);
```
— OCR wins when both exist (§1). The rest of the guarded `UPDATE`
(`ocr_suggested_date_issued = $4`, never overwriting an existing
`date_issued`) is unchanged from TDR 012.

**On not renaming the column (AC-3):** `ocr_suggested_date_issued` keeps
its name even though it's no longer exclusively OCR-sourced. Renaming
would touch the migration history, three existing TDRs' worked examples,
and every existing test referencing the column — real churn for a column
rename that changes no behavior. Flagged here explicitly rather than
left as a silent surprise for a future reader wondering why an
EXIF-sourced value lives in a column called `ocr_*`.

## 4. Test fixture: generating an EXIF-bearing image without ImageMagick/PIL
This sandbox has no image-editing tool that can *write* EXIF tags (the
existing `google-chrome --headless --screenshot` trick used for feature
014's Cyrillic fixture only rasterizes HTML — it can't embed metadata).
`little_exif` (Alternative C) is used **once, in a disposable scratch
Cargo project outside this repo**, purely to synthesize
`tests/fixtures/exif_dated_sample.jpg` — a plain JPEG with a
`DateTimeOriginal` tag set to a fixed, known test date and no OCR-legible
text (so the OCR-derived candidate is genuinely absent and the test
exercises the EXIF fallback path specifically, not the OCR path already
covered by feature 012's tests). The scratch project and `little_exif`
dependency are not committed anywhere in this repo — only the resulting
fixture bytes are.

## 5. OpenTelemetry Implications
`exif_extract::extract_issued_date` is called inline inside `run_ocr`'s
existing `#[tracing::instrument(skip(state, ...))]` span (matching
`date_extract`'s call site, TDR 012 §4) — no new span. Its own
`bytes: &[u8]` parameter is a plain function argument, not an
instrumented function's parameter, so no `skip` annotation is needed on
it directly, but the raw bytes must never be logged from inside it
either. Only whether an EXIF date was found (not the date's value, and
never the raw EXIF payload — which can include GPS coordinates) is
safe to consider recording, matching TDR 012 §4's existing rule for this
exact span.
</content>
