# TDR 027: Search-Hit Highlighting & Snippets

## 1. Context & Architectural Requirements
Backlog item deferred twice already — feature 023 (full-text search)
explicitly punted it to "the thumbnails + preview" item (TDR 023 §3), and
feature 025 punted it again once that item shipped without adding it
(TDR 025 §4): "no query-term-context threading is added here; it's a
distinct follow-up." Both prerequisites — a `tsvector`/`tsquery` full-text
index (023) and the side-by-side OCR text box (025) — are now in place.
Per CLAUDE.md: zero-panic, no new PII in spans, mockup-before-code (see
the signed-off mockup this TDR implements).

Two surfaces change:
1. `/documents` results list — a snippet line under a row whose *OCR
   text* matched `q`.
2. `/documents/{id}` detail page — full OCR text with every match marked,
   when `q` is present on that route.

## 2. Alternatives Evaluated

### Alternative A: Hand-roll snippet extraction and match-highlighting in Rust
- **Pros:** Full control over excerpt length/boundaries; no new SQL.
- **Cons:** Rejected. Re-implements word-boundary-aware tokenization and
  match-finding that Postgres's text-search engine already does correctly
  against the exact same `'simple'` config the `ocr_search` index uses
  (feature 023) — a hand-rolled version could disagree with what actually
  matched (e.g. stemming/normalization edge cases), and it's meaningfully
  more code to keep correct across the app's multilingual OCR text
  (feature 020).

### Alternative B: Postgres `ts_headline`, output trusted directly as HTML
- **Pros:** One function call gets both snippet extraction (fragment
  mode) and full-document match-marking (`HighlightAll=true`) for free,
  reusing the existing `websearch_to_tsquery('simple', ...)` call already
  used to *match* documents — same tokenization, so "what matched" and
  "what's highlighted" can't disagree.
- **Cons:** Rejected as-is. `ts_headline`'s `StartSel`/`StopSel` are
  normally set to literal HTML (`<mark>`/`</mark>`) and the result trusted
  verbatim — but the input is OCR'd text from a scanned bill/contract,
  which can contain literal `<`, `>`, `&` (misreads or genuine content).
  Rendering that string unescaped is an XSS hole: a document whose OCR'd
  text happens to contain `<script>` would inject it. AC-8 rules this out.

### Alternative C: `ts_headline` with control-character delimiters, Rust escapes everything else
- **Pros:** Keeps Alternative B's correctness (Postgres finds the matches,
  using the same tokenization as the actual search match), but resolves
  the XSS problem: `StartSel`/`StopSel` are set to `\u{1}`/`\u{2}` (control
  characters that can't occur in real OCR'd text, and even in the
  vanishingly unlikely case they do, the fallback is just a missed/bogus
  mark, not unescaped markup) instead of raw HTML. A small Rust function
  (`src/highlight.rs`) then splits the returned string on those markers,
  HTML-escapes every plain segment, and wraps only the segments between
  markers in `<mark>...</mark>` — so the *only* unescaped HTML in the
  output is markup this code adds itself, never anything from the source
  text.
- **Cons:** One extra small Rust module; a `ts_headline` call per matching
  row in `list`, and one extra scalar query in `show` when `q` is present.
  Both are bounded (page-of-results size; a single document), not O(corpus).

## 3. Structural Decision
We choose **Alternative C**.

**`src/highlight.rs`** (new, pure/unit-testable): two `&'static str`
option-string constants for `ts_headline`'s options argument —
`SNIPPET_OPTIONS` (`MaxFragments=1,MinWords=12,MaxWords=30`, plus the
delimiter markers) for the results-list excerpt, and `FULL_TEXT_OPTIONS`
(`HighlightAll=true`, same markers) for the detail page's complete text.
`render_marked(headline: &str) -> String` scans for the `\u{1}`/`\u{2}`
markers, HTML-escaping every segment (the same 5 characters — `&<>"'` —
Askama's default auto-escaper covers) and wrapping marked segments in
`<mark>`. Both call sites pass the result through this before handing it
to the template with Askama's `|safe` filter — the only place in this
change that opts out of Askama's normal auto-escaping, and only after
this function has already escaped everything that isn't its own markup.

**List page** (`documents.rs::list`): each of the 5 sort arms' `query_as!`
gains one column — `case when $9::text is not null and ocr_search @@
websearch_to_tsquery('simple', $9) then ts_headline('simple', ocr_text,
websearch_to_tsquery('simple', $9), $12) end as ocr_snippet` — reusing
`$9` (the existing free-text bind) and adding `$12` bound to
`highlight::SNIPPET_OPTIONS`. The `case when` condition is the same
OCR-match boolean already used in the row's own `WHERE` clause (feature
023), so a row that only matched via the tags facet's OR-condition gets
`null` here, not a misleading from-the-top excerpt (AC-2). No change to
`count_documents` — counts don't render a snippet. `DocumentListItem`
gains `ocr_snippet_html: Option<String>` (`row.ocr_snippet.as_deref().
map(highlight::render_marked)`); `documents_list.html` renders it with
`|safe` under the existing `.doc-row-meta` line. Each row's link into the
detail page also gains `?q=<url_encoded active search text>` (via the
existing `url_encode` helper) whenever a free-text search is active — the
carry-along mechanism AC-4 depends on, using the same request-level
`search_text` already computed in `list`, not a new per-row field.

**Detail page** (`documents.rs::show`): `ShowQuery` gains `q: Option<String>`.
Since `query: Query<ShowQuery>` is not currently in this handler's
`#[tracing::instrument(skip(state, tenancy))]` skip-list — safe while
`ShowQuery` was three booleans, not once it carries free-text — the
instrument attribute becomes `skip(state, tenancy, query)`, mirroring
`list`'s existing skip-list and closing the same PII gap AC-9/AC-10 of
feature 023 already closed for the list page (see the tracing-PII-on-
refactor lesson from prior features). When `q` is present and non-empty
and the row has `ocr_text`, a second scalar query — `select ts_headline
('simple', $1, websearch_to_tsquery('simple', $2), $3)`, passing the
already-fetched `ocr_text` and `highlight::FULL_TEXT_OPTIONS` — produces
the fully-marked text; without `q` (or with `q` but no `ocr_text`), the
plain text is run through `render_marked` too (with no markers present,
this is just the escape pass, byte-identical output to what Askama's
default auto-escaping already produced pre-027 — AC-7). Either way, the
template struct's new `ocr_text_html: Option<String>` field is always
populated when `ocr_text` is `Some`, and the template's single `{% if let
Some(ocr_text_html) = ocr_text_html %}` branch renders it with `|safe` —
no dual escaped/unescaped branches to keep in sync. A `has_highlight: bool`
(whether the marked-up string actually contained a `\u{1}` before
escaping) gates the small "Highlighting matches for &ldquo;...&rdquo;"
indicator — present only when something in this document actually matched,
never for a `q` that doesn't appear in this particular document (AC-6).

**Styling**: `.doc-row-snippet` and `mark` reuse the existing "ledger and
stamp" tokens only — `color-mix(in srgb, var(--stamp) 22%, var(--paper-
raised))`, the same formula already used for chip/suggestion-box
backgrounds elsewhere in `static/style.css`. No new color introduced.

## 4. Explicitly Deferred
- **Fuzzy/typo-tolerant highlighting** — see backlog §3; inherits 023's
  `pg_trgm`-deferral as-is.
- **A match-count or relevance signal** — still out of scope per TDR 023
  §3; a snippet is not a ranking feature.

## 5. OpenTelemetry Implications
No new spans. One existing `#[tracing::instrument]` (`show`) has its
skip-list widened from `(state, tenancy)` to `(state, tenancy, query)` —
a fix, not an addition: `ShowQuery` now carries a free-text `q` value that
must not enter the span, the same rule `list`'s instrumentation already
applied to its own `q` (feature 023 AC-9).
