# TDR 023: Full-Text Search Over OCR'd Document Text

## 1. Context & Architectural Requirements
The `/documents` search box (`q`) has, since feature 007, only ever done a
comma-separated tag OR-match (`parse_tag_search`) — it never looks at
`documents.ocr_text`, so a user can't find a bill by typing a word that
appears in the scanned text but isn't one of its tags. This was the top
pick from the 2026-07-15 backlog brainstorm. Per CLAUDE.md: zero-panic,
tenant-scoped queries, compile-time-verified `sqlx` macros throughout, no
new PII surface in spans.

## 2. Alternatives Evaluated

### Alternative A: A second, OR'd condition on the existing `q` box — `ocr_search @@ websearch_to_tsquery(...)` alongside the existing tag overlap
- **Pros:** Zero new query params, zero template facet-panel changes. `q`
  already has documented OR-search semantics (TDR 015 §3: a document
  matches if any comma-split term overlaps its tags); adding "or its OCR
  text full-text-matches the whole `q` string" is an extension of that
  same idea, not a new concept. A user typing into the one box they
  already know gets a strictly larger set of matches.
- **Cons:** Two different matching rules (array-overlap OR vs. tsquery
  AND-of-words) live behind one input, which the two facets already
  documented as OR'd against each other (TDR 015 §3) rather than combined
  into one predicate — that precedent carries over cleanly here too.

### Alternative B: A dedicated, separate free-text-search field, distinct from the tag box
- **Pros:** Keeps "search by tag" and "search by content" conceptually
  separate for the user.
- **Cons:** A second input, second facet panel field, second set of
  chips/URL params to maintain across `build_query_string`,
  `build_documents_url`, saved collections (feature 016), and every
  existing test asserting on `q`'s shape. Rejected: the todo item framed
  this as extending the existing box ("today the search box only parses
  comma-separated tags"), and a single box is simpler for a personal-scale
  tool with one user per tenant.

### Alternative C: `pg_trgm` trigram/fuzzy matching instead of (or in addition to) `tsvector`
- **Pros:** Tolerates typos and partial/substring matches, no stemming
  concerns across languages.
- **Cons:** A different index type (GIN over trigrams, not tsvector),
  different operators (`%`, `<->`) and a similarity-threshold tuning
  question that `tsvector`'s boolean `@@` doesn't have. Deferred out of
  v1 as unnecessary scope for "can I find a document containing this
  word" — tracked in ARCHITECTURE.md §8 as a possible future enhancement
  if exact-word tsvector matching proves too strict in practice.

## 3. Structural Decision
We choose **Alternative A**. A generated, indexed column:
```sql
alter table documents
  add column ocr_search tsvector
  generated always as (to_tsvector('simple', coalesce(ocr_text, ''))) stored;

create index documents_ocr_search_idx on documents using gin (ocr_search);
```
`'simple'` config, not `'english'`: this app OCRs English, German, Dutch,
Ukrainian, and generic Cyrillic text (feature 020's general language
support). `'english'` would run every token through the English snowball
stemmer regardless of the document's actual language — misleading for the
non-English documents this app explicitly supports. `'simple'` just
lowercases and tokenizes with no stemming, applied uniformly. The accepted
v1 tradeoff: no English stemming benefit (e.g. "invoice" won't match
"invoices").

`websearch_to_tsquery`, not plain `to_tsquery`: gives quoted-phrase and
`OR`/`-exclude` syntax for free, and — unlike `to_tsquery` — never errors
on arbitrary user input (no special-character escaping needed), which
matters since this text comes straight from an HTML form field.

A new `free_text_search(q: &str) -> Option<&str>` helper
(`src/web/handlers/documents.rs`, next to `parse_tag_search`) trims `q`
and returns `None` when empty — the same emptiness rule
`query_has_active_filters` already applies to `q`. This is threaded
through as a new final `search_text: Option<&str>` parameter on
`count_documents` and each of `list`'s 5 per-`Sort` `query_as!` arms
(reusing the exact per-sort-arm literal-query pattern TDR 007/015/018
already established, to keep compile-time query verification). The one
line each of those 6 queries already had —
```sql
and ($2::text[] is null or tags && $2)
```
— becomes:
```sql
and (($2::text[] is null or tags && $2)
     or ($9::text is not null and ocr_search @@ websearch_to_tsquery('simple', $9)))
```
appended as a new final bind param (`$9`) in each arm rather than
renumbering `$3`-`$8`, minimizing the diff. Every other facet condition
(tags AND-narrow, date, language) is untouched — the new predicate only
widens what `q` alone can match, exactly the same way the existing tag
overlap does.

**Why this doesn't disturb TDR 018's facet-narrowing pattern:** `q`'s
matching (both halves: tag-overlap and now OCR full-text) has always been
"whatever's currently active in the search box" pinned across every
facet-option count query — `count_documents`'s new `search_text` param is
passed unchanged at every one of its ~8 call sites in `list`'s
facet-discovery loops, the same way `tag_filter` already was. No new
per-facet dimension, no new candidate-set query — `q` was never itself a
"facet" with its own candidate list, so there's nothing new to narrow.

**Generated `STORED` column, not query-time `to_tsvector(ocr_text)`:**
indexable via GIN, and Postgres backfills every existing row
automatically when the column is added via `ALTER TABLE` — no separate
backfill migration needed at this app's personal scale.

## 4. OpenTelemetry Implications
No new spans, no new parameter entering any `#[tracing::instrument]`'d
function's captured args — `list`'s `#[tracing::instrument(skip(state,
tenancy, query))]` already fully skips the whole `ListQuery` (including
`q`, which is the only new source value here), so `search_text` — derived
from `q` and never separately logged — introduces no new PII surface.
Spot-checked in Jaeger, not just assumed, per the feature-021-era lesson
that extracting/threading a value through a `#[tracing::instrument]`'d
call path can silently widen a span's captured fields.
