# TDR 017: Rename a Saved Smart Collection

## 1. Context & Architectural Requirements
Feature 016 shipped `smart_collections` (`id`, `tenant_id`, `name`,
`query`, `created_at`) with create and delete only — renaming was
explicitly deferred (backlog 016 §3). This feature adds exactly one more
mutation: updating `name` in place. Per CLAUDE.md: zero-panic,
tenant-scoped, and PII (the collection name) kept out of spans, matching
`save_collection`/`delete_collection`'s existing precedent.

## 2. Alternatives Evaluated

### Alternative A: A dedicated `POST /documents/collections/:id/rename` endpoint, reusing `CollectionName`
- **Pros:** Symmetric with `delete_collection` (same `Path<Uuid>` +
  tenant-scoped `UPDATE ... RETURNING` shape); reuses the exact
  `CollectionName` newtype `save_collection` already validates with, so
  there's only one definition of "a valid collection name" in the
  codebase. No new table state — `query`/`created_at` untouched.
- **Cons:** One more route/handler pair to maintain.

### Alternative B: Keep delete-and-re-save as the only path (no new endpoint)
- **Pros:** Zero new code.
- **Cons:** This is precisely the friction the backlog item exists to
  remove — re-saving loses the original `created_at` (reordering the
  newest-first list) and requires re-deriving the exact `query` string
  the user would otherwise have to reconstruct by hand (the UI's "Save
  this search" control only has the *current* page's filters available,
  not an arbitrary existing collection's). Rejected.

## 3. Structural Decision
We choose **Alternative A**.
```sql
update smart_collections set name = $3 where id = $1 and tenant_id = $2 returning id
```
`RenameCollectionForm { name: CollectionName }` — identical shape to
`SaveCollectionForm`'s `name` field, same validation, same 422 rejection
path on an empty/oversized name (matching `an_out_of_set_language_value_
is_rejected`-style precedent for a `Form`-extractor `TryFrom` failure).

**UI** (`documents_list.html`, feature 016's collection row): an inline
rename affordance next to the existing delete button — a small pencil
icon toggling the name `<span>` into a text `<input name="name">` plus a
submit button using the same `formaction`/`formmethod` override pattern
`save_collection`/`delete_collection` already use inside the page's one
shared GET form (TDR 016 §3). No JavaScript state beyond a plain
`<details>`/checkbox-driven CSS toggle for showing the input — consistent
with this project's "works with JS off" convention (worst case: the input
is always visible instead of toggled, still fully functional).

## 4. OpenTelemetry Implications
`#[tracing::instrument(skip(state, tenancy, form))]`, matching
`save_collection` — the submitted name (old or new) never enters a span
attribute.
