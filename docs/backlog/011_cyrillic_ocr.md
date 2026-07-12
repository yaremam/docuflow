# User Story: Cyrillic OCR Support

## 1. User Value Statement
As a **logged-in DocuFlow user with Cyrillic-script documents (Russian,
Ukrainian, Bulgarian, etc.)**,
I want to **have OCR actually recognize Cyrillic text instead of garbling
it as if it were Latin script**,
So that **I can find and search my Cyrillic bills, insurance policies, and
contracts by content, not just by title/tags I typed in manually.**

## 2. Strict Acceptance Criteria
- **AC-1:** `tesseract` is invoked with both the English and Russian
  trained-data models loaded (`-l eng+rus`) for every image OCR pass and
  every rasterized PDF page, so a document containing Cyrillic text is
  recognized correctly instead of being run through the English-only model.
- **AC-2:** A document containing only Latin text continues to OCR
  correctly (no regression) — multi-language mode does not require the
  caller to know or declare the document's language up front.
- **AC-3:** The `tesseract-ocr-rus` trained-data package is installed
  alongside `tesseract-ocr` in `Dockerfile`'s runtime stage, so the
  language pack is present without any code-level fallback/detection.
- **AC-4:** If the Russian trained-data pack is ever missing from an
  environment, `tesseract` failing to start surfaces as an
  `ocr_status = 'failed'` row with a populated `ocr_error`, never a panic.
- **AC-5:** No `.unwrap()`, `.expect()`, or `panic!()` introduced in the
  changed code, per CLAUDE.md's zero-panic rule.
- **AC-6:** No PII (extracted text, page bytes) enters trace spans or
  logs — this feature does not change `ocr.rs`'s existing PII boundary.

## 3. Explicitly out of scope this round
- **A document-language metadata field or automatic language detection.**
  That's the separately-tracked "add document language field" backlog
  item; this feature always runs `eng+rus` unconditionally rather than
  selecting a language per document.
- **Any script/language beyond Russian's Latin+Cyrillic pair** (e.g.
  Ukrainian-specific characters, Greek, CJK). Tesseract's `rus` model
  covers standard Cyrillic well enough for this round; broader script
  support is future work if requested.
- **Retroactive reprocessing** of documents OCR'd before this change ships
  — that's the separately-tracked "redo the OCR" backlog item, which
  depends on this one.
- **Issued-date extraction from OCR text** — separately tracked, depends
  on this feature only in build order, not in scope here.
