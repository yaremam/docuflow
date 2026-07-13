# TDR 016: Saved Smart Collections

## 1. Context & Architectural Requirements
Feature 015 added a facet panel to `/documents` whose entire state already
lives in the URL's query string (`q`, `sort`, `tags`, `date_year`,
`date_month`, `undated`, `lang`) — `list` parses it fresh on every
request, and `build_documents_url` (added in 015) already knows how to
turn that state back into a query string for the applied-filter chips.
This feature adds the ability to persist one of those query strings under
a name, tenant-scoped, per CLAUDE.md's multi-tenancy rule. Per CLAUDE.md:
zero-panic, PII kept out of spans (the same filter-shaped data feature
015 already excludes), and — per the signed-off mockup (2026-07-13) — the
"ledger and stamp" visual identity reused, no new design direction.

## 2. Alternatives Evaluated

### Alternative A: Store the collection as the raw `/documents` query string (`smart_collections.query text`)
- **Pros:** A collection is exactly "a bookmark of a URL" — applying one
  is a plain link to `/documents?{query}`, reusing 100% of `list`'s
  existing parsing, validation, and facet logic with zero new
  filter-matching code. Saving is equally simple: the query string is
  already sitting in the current page's URL/form state, so there's
  nothing to decompose or re-assemble. Naturally forward-compatible — if
  feature 015 (or a later feature) ever adds a new facet param, old and
  new collections alike keep working with no migration, exactly like any
  browser bookmark.
- **Cons:** Not relationally introspectable — can't write a query like
  "show me every collection that includes tag X" without parsing the
  stored string first. Not a real concern for this feature's scope
  (collections are only ever applied whole, never queried by their
  contents).

### Alternative B: Structured columns mirroring `ListQuery` (`tags text[]`, `date_year int`, `date_month int`, `undated bool`, `lang text[]`, `q text`, `sort text`)
- **Pros:** Relationally queryable; a collection's contents are visible
  directly in a `select *`.
- **Cons:** Every future facet added to `list` (feature 015's own §2
  Alternative B already names one candidate: narrowed facet counts don't
  need this, but a genuinely new facet type would) requires a matching
  migration and column here too, or old collections silently can't
  express the new option. Also duplicates validation logic (feature 015
  already validates/normalizes `date_month` only mattering alongside
  `date_year`, etc.) — with two representations of "the same filter
  state," a mismatch between how `list` interprets its own query params
  and how this table's columns get turned back into one becomes a real
  bug class. Rejected: more moving parts for no capability this feature's
  ACs actually need.

### Alternative C: A confirmation page before deleting a collection, matching `document_delete.html`'s pattern
- **Pros:** Consistency with the one existing delete flow in the app.
- **Cons:** Document deletion is destructive and irreversible (the
  underlying file is gone from blob storage). Deleting a saved
  collection touches nothing but a bookmark — the documents it matched
  are completely unaffected, and recreating an identical collection is a
  few clicks away with the same filter panel. Matching the friction of
  an irreversible action to a trivially-reversible one would be
  over-cautious for what it protects. Rejected in favor of a single POST
  with no confirm step (AC-5).

## 3. Structural Decision
We choose **Alternative A**. New table (see migration
`20260713162422_create_smart_collections.sql`):
```sql
create table smart_collections (
    id uuid primary key,
    tenant_id uuid not null references tenants(id),
    name text not null,
    query text not null default '',
    created_at timestamptz not null default now()
);
create index smart_collections_tenant_id_idx on smart_collections (tenant_id);
```
`query` holds exactly the query-string portion `build_documents_url`
already produces (minus the leading `/documents?`) — e.g.
`tags=insurance&tags=medical&date_year=2026`.

**Routes** (`protected` group, `src/web/router.rs`):
- `POST /documents/collections` — creates a collection from the submitted
  `name` + `query` fields. The `query` field is a single hidden input
  inside the "Save this search" form (`documents_list.html`), populated
  server-side from the same filter state `list` already computed for the
  applied-filter chips — no client-side URL-building. The handler
  re-derives "was a filter actually active" itself (parses `query` the
  same way `list` parses the real query string, via a small shared
  helper) and rejects an empty/no-op save with `400`, rather than trusting
  the client to only ever submit a non-empty `query` (AC-4).
- `POST /documents/collections/:id/delete` — tenant-scoped delete, `404`
  if the id doesn't belong to the tenant, redirect back to `/documents`
  on success. No confirmation page (§2 Alternative C).

**Rendering** (`list` handler): one more query,
```sql
select id, name, query, created_at
from smart_collections where tenant_id = $1 order by created_at desc
```
Each collection's live count reuses the exact same facet/search-matching
`WHERE` clause `list` already builds for the main results — the
collection's stored `query` string is parsed into the same
`tags`/`date_year`/`date_month`/`undated`/`lang`/`q` shape `ListQuery`
already is (via `serde_html_form::from_str`, the exact same crate/version
`axum-extra`'s own `Query` extractor resolves to — added as a direct
dependency, pinned to match, rather than a fresh hand-rolled parser),
then bound into a single
`count(*)` query using that same `WHERE` clause (no `ORDER BY` variance
to worry about — counting needs none of `list`'s five sort arms). This
reuses 015's established `WHERE`-condition literal rather than inventing
a second way to express "does this document match this filter set"
(AC-7).

The "Save this search" control's visibility (AC-3) reuses `list`'s
existing `clear_filters_href.is_some() || !q.is_empty()` computation — no
new "is anything active" logic, just exposing the boolean already
implicit in that `Option`.

## 4. OpenTelemetry Implications
Both new handlers are `#[tracing::instrument(skip(state, tenancy, form))]`
/ `skip(state, tenancy)` — matching feature 015's `list` precedent, a
collection's `name` and `query` are exactly the same class of
user-chosen, filter-shaped data (tag names, search text) already excluded
from `list`'s spans, so they're skipped here too rather than newly
introduced as span attributes. Verified post-implementation by querying
Jaeger for the `save_collection`/`delete_collection` spans and checking
every tag.
