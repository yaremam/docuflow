# User Story: Non-English Month Names in Date Extraction

## 1. User Value Statement
As a **logged-in DocuFlow user whose bills/contracts are in German, Dutch,
or Ukrainian**,
I want to **have the issued date recognized from a written-out month name
in my own language, not just English or numeric dates**,
So that **the same "suggest, then confirm" date-issued convenience
English/numeric documents already get isn't missing for my language.**

## 2. Strict Acceptance Criteria
- **AC-1:** `extract_issued_date` recognizes full month names in English,
  German, Dutch, and Ukrainian — the same 4 languages tesseract's
  `eng+deu+nld+ukr` multi-language pack already OCRs (feature 011/020).
- **AC-2:** It also recognizes common abbreviated forms in all 4
  languages (e.g. `Jan`, `Feb.`, Dutch `mrt`, Ukrainian `бер`) — full
  scope, not just the originally-scoped full names (see TDR 030 §1).
- **AC-3:** Ukrainian dates use the grammatically correct **genitive**
  month form (`15 січня 2024`, not the nominative `15 січень 2024`) —
  the form that actually appears in real Ukrainian dates.
- **AC-4:** No document-language detection step is introduced — a
  single combined pass tries all 4 languages' names together, matching
  how tesseract itself OCRs without per-document language selection.
- **AC-5:** Adding a 5th language later requires only one new `const`
  table plus one entry in a `LANGUAGES` registry — no change to the
  regex-building or lookup logic.
- **AC-6:** No `.unwrap()`, `.expect()`, or `panic!()` introduced — an
  unrecognized or malformed candidate is simply "no date found here,"
  same as today.

## 3. Explicitly out of scope this round
- **Per-document language detection driving which month-name table is
  tried** — considered and rejected, see TDR 030 §2 Alternative B.
- **A dedicated `tesseract-ocr-srp` (or any other non-OCR-supported
  language's) month names** — this only covers the 4 languages OCR is
  actually tuned for.
- **Ukrainian nominative-form matching** — only the genitive form that
  real dates use is covered; a bare nominative month name appearing
  outside a date context (e.g. a calendar header) is not a target.
