# User Story: Document Language Field

## 1. User Value Statement
As a **logged-in DocuFlow user with documents in more than one language**
(English bills alongside Cyrillic-script ones, say),
I want to **have each document's language recognized automatically and
recorded on the document**,
So that **I can eventually filter my documents by language, and never
have to type a language in by hand for the common case.**

## 2. Strict Acceptance Criteria
- **AC-1:** Once a document's OCR pass completes successfully (`ocr_status
  = 'done'`), its extracted text is analyzed and, if confidently
  recognized as English or Cyrillic-script, `documents.language` is set
  to `en` or `cyr` respectively — written in the same `UPDATE` `run_ocr`
  already issues for `ocr_text`/`ocr_suggested_date_issued`.
- **AC-2:** `documents.language` is never overwritten once a value already
  exists there, whether that value came from auto-detection or a manual
  edit — a later OCR pass (e.g. after a reprocess) only ever writes into
  it `where language is null`.
- **AC-3:** `GET /documents/{id}` shows a "Language" field on the editable
  metadata form (English / Cyrillic / blank), pre-filled with whatever's
  in the DB. `POST /documents/{id}` (the existing metadata-save endpoint)
  accepts and persists a manually chosen value the same way it already
  does for title/tags/date issued.
- **AC-4:** There is no language field on the upload form (`document_new.
  html`) — nothing is knowable about a document's language before OCR has
  even run, so asking at upload time would only invite a guess.
- **AC-5:** Saving a document's metadata never fails or blocks because
  `language` is blank — "compulsory" here means every document is
  expected to end up with a value (via auto-detection, or a manual pick
  for the cases that don't), not that any save is rejected for lacking
  one. (Confirmed with the user 2026-07-13 — the alternative, a hard
  validation gate, was explicitly rejected as inconsistent with every
  other metadata field on this form.)
- **AC-6:** The recognized value set is closed to exactly two buckets —
  English and Cyrillic-script (`tesseract -l eng+rus`, see TDR 011) — not
  an open-ended per-language list. The Cyrillic bucket is script-level
  (any Cyrillic-script text, regardless of which specific language),
  matching what the single shared `rus` trained-data pack actually OCRs;
  the English bucket stays language-specific, since Latin script alone
  would sweep in languages OCR has no support for. Text that's neither
  reliably English nor Cyrillic-script leaves `language` unset rather
  than forcing it into the wrong bucket.
- **AC-7:** No `.unwrap()`, `.expect()`, or `panic!()` introduced in the
  changed code, per CLAUDE.md's zero-panic rule — text that doesn't
  confidently match a supported language simply leaves `language` unset,
  never an error.
- **AC-8:** No PII (OCR text itself) enters trace spans or logs from the
  new detection step — only whether a language was recognized, and which
  one, is safe to record, matching `extract_issued_date`'s precedent
  (TDR 012 §4).
- **AC-9:** Tenant scoping is unchanged — language is just another column
  on the existing tenant-scoped `documents.update` path; no new route.

## 3. Explicitly out of scope this round
- **The smart-filters language facet.** This feature only makes the field
  exist and get populated; filtering `/documents` by it is the
  separately-tracked, already-sequenced-after-this smart filters panel
  feature.
- **Retroactive detection for documents OCR'd before this ships.** Same
  situation as every prior OCR pipeline improvement (010/011/012) — an
  existing `done` document keeps `language = null` until reprocessed via
  the feature 013 "Reprocess OCR" button, which will pick up language
  detection for free since it re-runs `run_ocr` end to end.
- **Any language beyond English/Cyrillic-script.** Tied directly to what
  `tesseract -l eng+rus` can produce; adding a third OCR language pack
  (its own backlog-worthy change, see TDR 011) is a prerequisite for
  widening this field's value set, not something this feature does on
  its own.
- **Real OCR quality, and distinct field values, for Ukrainian and
  Serbian.** Raised and discussed during scoping (2026-07-13): since
  detection here is script-level (see AC-6), Cyrillic-script Ukrainian
  or Serbian text *does* land in the `cyr` bucket — but OCR quality for
  it stays degraded (the shared `rus` trained-data pack misses
  Ukrainian-only letters like і/ї/є/ґ), and neither language gets its
  own distinct field value or dedicated trained-data pack. Serbian's
  common Latin-script form doesn't land in `en` either (falls to
  "unset," a safe default, not an error). Proper support — dedicated
  trained-data packs and distinct per-language values instead of the
  generic bucket — is tracked separately in `docs/backlog/todo.md`.
- **Per-word or per-region language mixing.** A document is assigned at
  most one language for the whole document, even if it contains a mix
  (e.g. an English cover letter with a Cyrillic-script attachment) — no
  segment-level detection.
