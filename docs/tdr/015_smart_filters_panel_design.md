# TDR 015: Smart Filters Panel

## 1. Context & Architectural Requirements
`GET /documents` (`src/web/handlers/documents.rs::list`) already supports a
free-text tag search (`q`, OR-overlap against `tags`) and five fixed sort
orders, each spelled out as its own `sqlx::query_as!` literal (feature
007/013 precedent — `ORDER BY` can't be parameterized inside the macro
without losing compile-time verification, so the small fixed set of sort
modes is enumerated rather than built at runtime). Feature 014 added
`documents.language`, explicitly deferring "filter by it" to this feature.
This feature adds three facets (tags, date issued, language) as additional,
AND-combined narrowing on top of `q`/`sort`, plus the counts needed to
render them. Per CLAUDE.md: zero-panic, tenant-scoped throughout, PII kept
out of spans, and (per the mockup sign-off, 2026-07-13) the "ledger and
stamp" visual identity reused rather than a new one.

## 2. Alternatives Evaluated

### Alternative A: Extend the existing per-sort-arm `query_as!` pattern with more `WHERE` params; facet counts as separate, unfiltered-by-other-facets queries
- **Pros:** Directly extends the established, compile-time-verified
  pattern instead of introducing a second query-building mechanism
  alongside it. Every new condition is an `($n::type is null/empty or
  ...)` clause, the same idiom `tag_filter` already uses — no new concept
  for a future reader. Facet counts computed once per facet, against the
  tenant's full document set, are 4 cheap, independent, easy-to-reason-
  about queries (not counts that shift meaning depending on which other
  filters happen to be active).
- **Cons:** The five sort arms each grow more `WHERE` params (8 total,
  up from today's 2), which is more literal SQL duplication than a
  dynamic query builder would produce. Facet counts don't shrink as
  other facets are applied — a real UX gap addressed in Alternative B,
  deliberately deferred (AC-10).

### Alternative B: `sqlx::QueryBuilder` for dynamic `WHERE`/`ORDER BY`, with facet counts recomputed against every *other* active facet (true faceted-search counts)
- **Pros:** Eliminates the 5-way sort-arm duplication entirely — one
  query, built conditionally. Recomputing each facet's counts excluding
  its own filter (so "Tags" counts reflect the active date/language
  filters, etc.) is the behavior users of Amazon/Lightroom-style facet
  panels actually expect.
- **Cons:** `QueryBuilder` loses `sqlx::query_as!`'s compile-time column
  and type verification — a real regression against this project's
  established "strict compile-time verified queries" stack choice
  (CLAUDE.md §1), not a style preference. True per-facet counts also mean
  3 extra queries *per facet, per request* (once with the facet's own
  filter excluded), a meaningfully bigger change than this round's scope.
  Rejected for this iteration; the count-narrowing gap is tracked as a
  named limitation (AC-10) rather than silently shipped as if it were the
  real thing.

### Alternative C: Client-side (JS) faceted filtering over a fully-loaded document list
- **Pros:** No new server-side query logic; instant filtering with no
  round trip.
- **Cons:** Breaks with JavaScript disabled, contradicting AC-6 and every
  other form on this project (progressive enhancement is a standing
  convention, not new to this feature). Also loads every document up
  front regardless of tenant size, which the current paginated-by-query
  approach avoids. Rejected.

## 3. Structural Decision
We choose **Alternative A**.

### Query params (all additive to existing `q`/`sort`, all optional)
```rust
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    q: String,
    sort: Option<String>,
    #[serde(default)]
    deleted: bool,
    #[serde(default)]
    tags: Vec<String>,        // facet tag checkboxes (repeated `tags=` params)
    date_year: Option<i32>,
    date_month: Option<i32>,  // ignored unless date_year is also set
    #[serde(default)]
    undated: bool,
    #[serde(default)]
    lang: Vec<String>,        // "en" | "cyr" | "unset", repeated `lang=` params
}
```
`Vec<String>` for `tags`/`lang` deserializes from repeated same-named query
params (`tags=insurance&tags=medical`) the way `serde_urlencoded` (which
`axum::Query` uses) already handles — no new extractor.

### Per-facet semantics (this is the one non-obvious design call, so it's
spelled out explicitly rather than left to be inferred from the SQL):

| Facet | Within-group | Reasoning |
|---|---|---|
| Tags (facet checkboxes) | **AND** (`tags @> $selected`) | Checking two tags is a user saying "documents in *both* categories" — narrowing, matching how a folder hierarchy would work. Deliberately different from the free-text `q` box, which stays OR (`tags && $q_tags`) — the box is for "documents about any of these," the panel is for "documents in exactly this combination." |
| Date issued | **Single active year** (optionally + one month) **OR**'d with "Undated" | Matches Lightroom's date navigator: one node of the tree is open at a time, but "Undated" is a genuinely separate bucket a user reasonably wants unioned in (e.g. "this year's stuff, plus whatever never got a date"). |
| Language | **OR** (`language = any($selected)`, "Not set" = `language is null`) | Standard faceted-search convention (check two boxes, see documents matching either) — no reason to require a document be *simultaneously* two languages, unlike tags where co-occurrence is the whole point. |

All three facets **AND** together, and AND with `q`/`sort` (unchanged).

### The five sort arms gain the same six new bind params each:
```sql
select id, title, original_filename, content_type, blob_key, tags, date_issued, ocr_status, created_at
from documents
where tenant_id = $1
  and ($2::text[] is null or tags && $2)                          -- q (unchanged)
  and (cardinality($3) = 0 or tags @> $3)                          -- facet tags (AND)
  and (
    ($4::int4 is null and $6 = false)
    or ($4::int4 is not null
        and extract(year from date_issued)::int4 = $4
        and ($5::int4 is null or extract(month from date_issued)::int4 = $5))
    or ($6 and date_issued is null)
  )                                                                 -- date issued
  and (
    (cardinality($7) = 0 and $8 = false)
    or (language = any($7))
    or ($8 and language is null)
  )                                                                 -- language
order by created_at desc  -- (only this clause differs per arm, as today)
```
`$7`/`$8` split the `lang` param into a `text[]` of `{en,cyr}` values and a
separate `unset: bool` before binding — cleaner than teaching every arm's
SQL to special-case a sentinel string inside the array.

### Facet counts (4 small queries, independent of the main list query and of each other — see Alternative B for why they don't factor in currently-active filters):
- **Tags:** `select unnest(tags) as tag, count(*) from documents where tenant_id = $1 group by tag order by count(*) desc, tag limit 10`.
- **Years:** `select extract(year from date_issued)::int4 as year, count(*) from documents where tenant_id = $1 and date_issued is not null group by year order by year desc`.
- **Months** (only queried when `date_year` is set, to avoid fetching a full year×month matrix up front): `select extract(month from date_issued)::int4 as month, count(*) from documents where tenant_id = $1 and extract(year from date_issued) = $2 group by month order by month`.
- **Undated count:** `select count(*) from documents where tenant_id = $1 and date_issued is null`.
- **Language:** `select language, count(*) from documents where tenant_id = $1 group by language` — grouped in Rust into `en`/`cyr`/`unset (language is null)` buckets.

### Template / UI
`templates/documents_list.html` gains the panel (`.filters-panel`, three
`.filter-group`s) and an `.applied-filters` chip row above `.doc-list`,
matching the signed-off mockup
(https://claude.ai/code/artifact/246cda18-37f2-49f4-b347-402df56defb1)
exactly for markup/classes — `static/style.css` gains the corresponding
rules (`.filters-layout`, `.filters-panel`, `.filter-group`, `.filter-tree`,
`.filter-chip`, `.filters-toggle`, and friends), reusing the existing
`--ink`/`--paper`/`--stamp`/`--line` tokens, no new palette. Facet
checkboxes and chip links are plain `<a>`/inputs inside the existing
`<form method="get" action="/documents">` — checking a box submits the
form (a tiny inline `onchange="this.form.submit()"` — still a real GET
form submission, not fetch/JS state, so it degrades to "check the box,
click Apply" with JS off exactly like every checkbox-in-a-form does).

The zero-facet-match empty state (AC-8) is a second `{% if %}` branch in
the template, distinguishing "no documents in the tenant at all" (existing
`documents.is_empty()` with no facets active) from "facets narrowed to
zero" (`documents.is_empty()` with at least one facet param present).

## 4. OpenTelemetry Implications
Today's `list` handler is `#[tracing::instrument(skip(state, tenancy))]`
with `Query(query): Query<ListQuery>` **not** skipped — meaning `q` (the
tenant's own search text) already enters the span via `Debug`. This
feature adds `tags`/`lang`/`date_year`/`date_month`/`undated` to the same
struct, which would multiply how much user-chosen filter content lands in
Jaeger if left as-is. Per CLAUDE.md's PII-sanitization rule and
[[feedback-tracing-pii-on-refactor]] (this project's own precedent for
exactly this failure mode — a struct gaining fields silently increases
what's captured on an already-`#[instrument]`ed function), this feature
adds `query` to the `skip(...)` list:
```rust
#[tracing::instrument(skip(state, tenancy, query))]
pub async fn list(...)
```
This is a net tightening of existing behavior, not just a hold-the-line —
`q` stops appearing in spans too, closing a pre-existing minor gap while
fixing it for the new fields. Verified post-implementation by querying
Jaeger directly for a `list` span with several facets active and checking
every tag key (not just the ones expected) for tag/language/date content.
</content>
