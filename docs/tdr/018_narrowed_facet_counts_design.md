# TDR 018: Narrow Smart-Filter Facet Counts by Active Facets

## 1. Context & Architectural Requirements
Feature 015 shipped facet counts against the tenant's full document set,
explicitly deferring "narrow by other active facets" as a named
simplification (TDR 015 §2 Alternative B). This feature closes that gap
for the three panel facets (Tags, Date issued, Language). Feature 016's
saved-collection counts are a different shape entirely: a collection
already stores one *complete* filter combination (every dimension), and
its count is "how many documents match this whole saved query" — there's
no "other active facet" to narrow by, because a collection isn't one
facet value being toggled inside a live panel, it's the equivalent of the
*entire* panel state, frozen. `count_matching_documents` (feature 016)
keeps its current signature and behavior unchanged. Per CLAUDE.md:
zero-panic, tenant-scoped, PII-safe spans (unchanged from 015/016 — only
bind values change, not what's captured).

## 2. Alternatives Evaluated

### Alternative A: One `count(*)` query per facet option, each with that facet's own dimension overridden to a single candidate value and every other active dimension left as-is
- **Pros:** Reuses the exact `WHERE`-clause shape `count_matching_documents`
  already established — extracted into a lower-level `count_documents`
  helper taking explicit params instead of a `&ListQuery`, so both the
  existing per-collection count and the new per-facet-option counts share
  one query definition. Each call is trivial to reason about: "everything
  currently active, except this one dimension is pinned to this one
  candidate."
- **Cons:** A page with 10 tags + several years/months + 3 language
  options means ~15-20 small `count(*)` queries per `/documents` render,
  on top of the main list query and any collection counts. More database
  round trips than today.

### Alternative B: A single batched query per facet type (e.g. `GROUPING SETS` or a `UNION ALL` of per-candidate subqueries) computing every candidate's count in one round trip
- **Pros:** Far fewer queries — one per facet type (3 total) instead of
  one per candidate (~15-20).
- **Cons:** Meaningfully more complex SQL — each candidate needs its own
  dynamically-parameterized `WHERE` fragment inside one statement, which
  either means building the query string at runtime (losing
  `sqlx::query_as!`'s compile-time verification, the same tradeoff TDR
  007/015 already rejected once) or a fixed-shape `GROUPING SETS` query
  that doesn't naturally express "count with this dimension pinned, that
  one still filtered." Rejected for this round: DocuFlow is a personal
  document manager (tens to low hundreds of documents per tenant, not
  enterprise scale), so ~20 sub-millisecond indexed `count(*)` queries on
  a page load is a real but acceptable cost, not a correctness or
  usability problem. Worth revisiting only if this ever shows up as an
  actual latency complaint.

### Alternative C: Keep counts unfiltered by other facets (status quo)
- **Pros:** Zero new queries.
- **Cons:** This is exactly the backlog item being addressed — rejected.

## 3. Structural Decision
We choose **Alternative A**. `count_matching_documents` is refactored
into a thin wrapper around a new lower-level helper:
```rust
async fn count_documents(
    state: &AppState,
    tenant_id: Uuid,
    q_tags: Option<&[String]>,
    facet_tags: &[String],
    date_year: Option<i32>,
    date_month: Option<i32>,
    undated: bool,
    lang_values: &[String],
    lang_unset: bool,
) -> Result<i64, AppWebError>
```
— the exact `WHERE` clause `count_matching_documents` already used,
minus the `&ListQuery`-shaped wrapper around it. `count_matching_documents`
becomes `count_documents` called with a filter's full, un-overridden
state (used by feature 016, and by `list` itself for the "how many
documents total match everything currently active" summary line).

For each Tags facet candidate `t`: `count_documents(..., facet_tags:
&[t.clone()], date_year, date_month, query.undated, &lang_values,
lang_unset)` — every currently-active dimension *except* tags, which is
pinned to just this one candidate (not unioned with whatever tags are
already checked — see the note below on why this is the chosen
approximation for AND-multiselect facets).

For each Date issued candidate (a year, a year+month, or "Undated"):
`count_documents(..., &query.tags, <that candidate's year/month/undated>,
&lang_values, lang_unset)` — tags and language stay as currently active,
date is pinned to just this candidate.

For each Language candidate (`en`/`cyr`/`unset`): `count_documents(...,
&query.tags, date_year, date_month, query.undated, <that candidate alone>)`
— tags and date stay as currently active, language is pinned to just
this candidate.

**Note on Tags' AND-within-group semantics (TDR 015 §3):** the panel lets
a user check *multiple* tags at once (an AND-narrowing). A fully
"correct" narrowed count for an unchecked tag would be "documents
matching my currently-checked tags **plus** this one" (cumulative), not
"documents matching only this one tag" (replacement). This feature uses
replacement — the same rule applied uniformly to all three facets — as a
deliberate, documented simplification: it's one consistent mental model
("this number is what you'd see if this were your only choice in this
facet, alongside what's already active elsewhere") rather than a
per-facet-type special case, at the cost of a checked-multiple-tags count
occasionally reading a little differently than strict AND-cumulative
counting would. Tracked as a known approximation, not silently shipped as
if it were exact.

The **candidate sets themselves** (which tags appear at all, which years
appear at all) stay exactly as feature 015 already computes them — the
tenant's top-10-tags-by-total-count and every year with at least one
document, unfiltered. Only the number next to each candidate narrows
(AC-5) — keeps the panel's shape stable as a user filters, rather than
options appearing/disappearing.

## 4. OpenTelemetry Implications
No new spans, no new parameters entering any `#[tracing::instrument]`'d
function — this only changes which literal bind values the existing,
already-audited `list` span's internal queries receive. Nothing new to
verify in Jaeger beyond a spot check that the extra queries don't
introduce a new instrumented boundary by accident.
</content>
