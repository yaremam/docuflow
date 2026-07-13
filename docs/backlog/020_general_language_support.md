# User Story: General Language Support (German, Dutch, Ukrainian OCR + full-world tagging)

## 1. User Value Statement
As a **DocuFlow user with documents in German, Dutch, and Ukrainian, plus
other languages I may want to tag by hand**,
I want to **get accurate OCR for those three languages, and be able to
label any document with the language it's actually written in, not just
a coarse English/Cyrillic-script bucket**,
So that **my documents are searchable and filterable by their real
language, and text in German/Dutch/Ukrainian is actually read correctly
instead of forced through a mismatched trained-data pack.**

## 2. Strict Acceptance Criteria
- **AC-1:** `tesseract` runs with `-l eng+deu+nld+ukr` (feature 011's
  `eng+rus` widened, Russian dropped) — German, Dutch, and Ukrainian text
  is recognized using its own trained-data pack, not the generic Cyrillic
  bucket feature 014 shipped with.
- **AC-2:** `documents.language` accepts any real ISO 639-1 code (~180
  languages), not a closed enumerated list — validated in application
  code (`src/languages.rs`, backed by the `isolang` crate), not a
  DB-level CHECK enum.
- **AC-3:** Auto-detection (post-OCR, same `run_ocr` write path as feature
  014) only ever proposes one of the 4 OCR-supported codes (`en`/`de`/
  `nl`/`uk`) or leaves the field blank — it never guesses at a language
  OCR wasn't tuned for, even though `whatlang` itself can identify more.
- **AC-4:** `GET /documents/{id}`'s language `<select>` offers every ISO
  639-1 language, grouped into two `<optgroup>`s: "OCR-supported" (the 4
  curated languages) and "All languages" (everything else, alphabetical,
  manual-tagging only). Mockup signed off 2026-07-13 before this or any
  template code was written, per CLAUDE.md's UI process.
- **AC-5:** `/documents`'s language filter facet shows one checkbox per
  language actually present among the tenant's documents (discovered via
  query, like the tag facet), not a fixed 3-option list — since the field
  is now open-ended, the candidate set can no longer be hardcoded.
- **AC-6:** Existing `language = 'cyr'` rows (feature 014's retired
  generic bucket) are cleared to `null` by the migration, not guessed at
  — they predate per-language values and don't map to one specific
  language. Feature 013's reprocess-OCR, or a manual re-tag, repopulates
  a real code.
- **AC-7:** Russian OCR support (feature 011's `tesseract-ocr-rus`) is
  fully removed, per explicit user direction (2026-07-13) — not kept
  alongside the new packs.
- **AC-8:** No `.unwrap()`/`.expect()`/`panic!()` introduced, per
  CLAUDE.md's zero-panic rule. No PII (OCR text) enters trace spans,
  matching TDR 012 §4/TDR 014 §4's existing precedent for this span.
- **AC-9:** Tenant scoping unchanged — same existing tenant-scoped
  `documents` routes, no new endpoints.

## 3. Explicitly out of scope this round
- **Serbian OCR.** Explicitly requested to be skipped in this round
  (2026-07-13) — the language field accepts `sr` (it's a valid ISO 639-1
  code like any other), but there's no dedicated `tesseract-ocr-srp`
  pack, so Serbian text OCRs through whichever of the 4 curated packs
  happens to match best, not a Serbian-tuned one. Tracked in
  `docs/backlog/todo.md`.
- **Bundling every Tesseract-supported language pack
  (`tesseract-ocr-all`).** Considered and rejected (2026-07-13): ~4GB of
  image growth for OCR coverage this project doesn't need yet. The
  curated set (eng/deu/nld/ukr) is designed to be extended one pack at a
  time later — a Dockerfile line + a `languages::OCR_SUPPORTED` entry +
  a `language_detect::detect` match arm, no schema change.
- **Retroactive re-detection for documents OCR'd before this ships.**
  Same precedent as every prior OCR pipeline change (010/011/012/014) —
  an existing `done` document keeps whatever `language` it already has
  (or `null`, post-migration, for former `cyr` rows) until reprocessed.
- **Per-word or per-region language mixing.** Unchanged from feature 014
  — a document is assigned at most one language for the whole document.
