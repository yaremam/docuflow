# User Story: EXIF-Based Issued-Date Suggestion

## 1. User Value Statement
As a **logged-in DocuFlow user who photographs a bill or letter with
their phone** (feature 009's scan flow, or a direct image upload),
I want to **have the photo's capture date considered as a possible
issued date when OCR text alone doesn't contain one**,
So that **I still get a one-click date suggestion for documents whose
OCR text doesn't include a recognizable date, instead of always having
to type it in by hand.**

## 2. Strict Acceptance Criteria
- **AC-1:** When an uploaded image carries an EXIF `DateTimeOriginal` (or
  `DateTime`) tag, that date is a candidate issued-date suggestion.
- **AC-2:** OCR-text-derived dates (feature 012) still take priority
  when present — EXIF only fills the suggestion when
  `date_extract::extract_issued_date` found nothing in the OCR text. A
  photo's capture date is a weaker signal than a date actually printed
  on the document (see TDR 019 §1).
- **AC-3:** This reuses the exact existing `ocr_suggested_date_issued`
  column and "Suggested issued date — Use this date" UI from feature 012
  — no new column, no new template, no new endpoint. The suggestion box
  doesn't distinguish its source (OCR text vs. EXIF) to the user.
- **AC-4:** PDFs and any image with no EXIF data (or no date tag within
  it) simply get no EXIF-sourced suggestion — falls back to whatever
  feature 012 already does (OCR-derived, or nothing), exactly as before
  this feature existed.
- **AC-5:** No `.unwrap()`, `.expect()`, or `panic!()` introduced —
  unparseable or absent EXIF data is not an error, it's just "no
  suggestion from this source."
- **AC-6:** No raw file bytes or EXIF contents (camera make/model, GPS
  coordinates if present, etc.) ever enter trace spans or logs — only
  whether a candidate date was found, matching `extract_issued_date`'s
  existing PII-safe precedent (TDR 012 §4).
- **AC-7:** Tenant scoping is unchanged — this runs inside the existing
  tenant-scoped `run_ocr` background task, no new route.

## 3. Explicitly out of scope this round
- **GPS/location metadata, camera make/model, or any other EXIF field.**
  Only a capture-date tag is read.
- **Distinguishing the suggestion's source in the UI** ("from the photo"
  vs. "from the text") — AC-3.
- **EXIF orientation-based image rotation, or any other EXIF-driven
  display behavior.** Out of scope; this is a date-suggestion feature
  only.
</content>
