# User Story: Search-Hit Highlighting & Snippets

## 1. User Value Statement
As a **logged-in DocuFlow user searching the `/documents` dashboard by
free text**,
I want to **see why each result matched, right in the results list, and
have my search term highlighted when I open the document**,
So that **I don't have to open every result and re-read the whole OCR
text to find the part that actually matched.**

## 2. Strict Acceptance Criteria
- **AC-1:** A search-results row whose *OCR text* matched the free-text
  box (`q`) shows a short excerpt of that text underneath, with the
  matched word(s) marked.
- **AC-2:** A row that matched only via the tags facet's OR-condition
  (feature 023 AC-3), not via its OCR text, shows no excerpt — there's no
  text hit to show, and a snippet built from the start of an unrelated
  document would be misleading.
- **AC-3:** No `q` active means no excerpt on any row — identical to
  pre-027 rendering.
- **AC-4:** Opening a document from a search-results page's link carries
  the free-text `q` along; on `/documents/{id}`, every occurrence of the
  matched word(s) in the full extracted-text box is marked, and a small
  indicator states what's being highlighted.
- **AC-5:** `/documents/{id}` also accepts a `q` typed directly into its
  own URL (not just carried from a search) — highlighting is a pure
  function of that param, independent of how the page was reached.
- **AC-6:** A document whose OCR text doesn't contain any of `q`'s words
  renders its extracted text exactly as before — no stray marks, no
  misleading "highlighting" indicator.
- **AC-7:** Visiting `/documents/{id}` with no `q` at all is byte-for-byte
  unchanged from pre-027 rendering.
- **AC-8:** OCR'd text can itself contain literal `<`, `>`, `&` (misread
  or genuine characters on the source document) — none of that ever
  renders as live HTML. Only the highlighting markup this feature adds is
  unescaped.
- **AC-9:** No `.unwrap()`, `.expect()`, or `panic!()` introduced.
- **AC-10:** No new PII in spans/logs — `q` already had to be kept out of
  `list`'s span (feature 023 AC-9); `show`'s span gets the same treatment
  now that its query struct carries a free-text value too.

## 3. Explicitly out of scope this round
- **Fuzzy / typo-tolerant matching** in the highlight itself — inherits
  023's exact-word-after-tokenization limitation; a typo'd search that
  still matches via `websearch_to_tsquery` highlights correctly, but nothing
  new is added for near-misses (see ARCHITECTURE §8, TDR 023/025).
- **Relevance-ranked sort or a match-count indicator** — still explicitly
  out of scope per TDR 023 §3; a snippet is not a ranking signal here.
- **Highlighting inside the small dashboard thumbnail image** — text
  found by OCR is highlighted in the *extracted-text* box only, never
  drawn onto the image/PDF preview itself.
