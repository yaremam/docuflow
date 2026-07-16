# User Story: Full-Text Search Over OCR'd Document Text

## 1. User Value Statement
As a **logged-in DocuFlow user searching the `/documents` dashboard**,
I want to **find a document by typing a word that appears in its scanned
text**,
So that **I don't have to remember or guess the exact tag I filed it
under — the words on the bill/contract/insurance page itself are enough.**

## 2. Strict Acceptance Criteria
- **AC-1:** Typing a word into the existing search box (`q`) that appears
  in a document's OCR'd text, but not in that document's tags, still
  returns that document.
- **AC-2:** A document whose OCR text and tags both fail to match `q` is
  still excluded — this doesn't turn `q` into "match everything."
- **AC-3:** `q` is still an OR-search: a document matches if it satisfies
  *either* the existing tag-overlap condition *or* the new OCR full-text
  condition (or both) — matching TDR 015 §3's existing "OR" framing for
  `q`, just widened to a second way of matching.
- **AC-4:** A multi-word `q` (e.g. `electric company`) matches a document
  only if its OCR text contains all of those words (`websearch_to_tsquery`
  bare-word default is AND-of-terms) — not merely one of them.
- **AC-5:** The `tags` smart-filter facet (checkbox panel, feature 015)
  keeps its current AND-narrowing behavior, independent of `q` — a
  document matching `q` via OCR text still has to satisfy every checked
  `tags` facet value too.
- **AC-6:** Feature 018's per-facet-option narrowed counts (Tags/Date
  issued/Language) reflect the OCR-widened `q` match the same way they
  already reflect the tag-only match today — no facet count regresses to
  ignoring `q`.
- **AC-7:** A document with no OCR text yet (`ocr_status` not `done`, or
  `ocr_text` null) never matches on the full-text half of `q` — it can
  still match via tags, unaffected.
- **AC-8:** No `.unwrap()`, `.expect()`, or `panic!()` introduced.
- **AC-9:** No new PII in spans/logs — `q` (and the OCR text it's matched
  against) never enters a trace; `list`'s existing `skip(state, tenancy,
  query)` instrumentation already covers this.

## 3. Explicitly out of scope this round
- **Fuzzy / typo-tolerant / substring matching (`pg_trgm`).** This ships
  exact-word (post-tokenization) matching only; see TDR 023 §2
  Alternative C.
- **Relevance-ranked sort.** The 5 existing sort modes (date
  uploaded/issued, tags) are unchanged — no "best match first" option.
- **Search-hit highlighting / snippets in results.** Deferred to the
  later "thumbnails + in-browser preview" backlog item, which already
  anticipated pairing with full-text search.
- **A separate free-text field distinct from the tag box.** One box,
  OR-combined matching — see TDR 023 §2 Alternative B.
