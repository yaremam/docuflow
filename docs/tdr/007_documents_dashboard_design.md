# TDR 007: Documents Dashboard — List, Search, Sort, Detail, Edit

## 1. Context & Architectural Requirements
Every feature up to this point (auth, nav, profile, password reset) was
scaffolding around the account itself — nothing yet touched the product's
actual purpose, storing and finding a user's documents. The original ask
covered list/upload/search/sort/detail/edit in one breath, but was
explicitly split by the user into three separately-shippable features:
this dashboard (list/search/sort/view/edit over documents that already
exist), file upload + OCR, and phone-camera scanning. This TDR covers only
the first. No `documents` table, `DocumentId` type, or document handler
module existed before this feature.

Because upload isn't built yet, this feature's own tests seed rows
directly via `sqlx::query!` against the test pool (matching this
project's existing test-fixture convention) rather than through an
upload endpoint.

## 2. Alternatives Evaluated

### Alternative A: Dynamically build the `ORDER BY` clause from the `sort` query param
- **Pros:** One `sqlx::query_as!` call handles every sort mode; no
  per-mode duplication.
- **Cons:** `sqlx::query!`/`query_as!`'s compile-time verification checks
  a literal query string — building `ORDER BY` at runtime means falling
  back to a non-macro `sqlx::query` for the *entire* query, forgoing
  compile-time column/type checking for the whole list endpoint, not just
  the ordering. Also reopens a SQL-injection surface (even if the values
  are whitelisted before use) that the macro closes by construction.

### Alternative B: One literal `sqlx::query_as!` call per whitelisted sort mode (chosen)
- **Pros:** Every query stays compile-time verified end to end, matching
  CLAUDE.md's "Strict Compile-Time Verified Queries" rule without
  exception. The sort-mode set is small and fixed (5 values today), so the
  duplication is bounded and unlikely to grow unmanageably.
- **Cons:** Five near-identical match arms in `handlers/documents.rs::list`
  differing only in their `order by` clause — a real, accepted
  duplication cost, not an oversight.

**Chosen: Alternative B.**

---

### Alternative C: `documents` ↔ `tags` join table (many-to-many)
- **Pros:** Normalized; supports tag-level metadata (e.g. per-tag color)
  if ever needed; avoids Postgres-specific array operators.
- **Cons:** Unbounded added complexity (join table, migration, join
  queries) for a feature whose only current need is "does this document
  have any of these tags" — no requirement here calls for tag-level
  metadata or cross-tenant tag sharing.

### Alternative D: Native Postgres `text[]` column with a GIN index (chosen)
- **Pros:** `tags && $1` (array overlap) with a GIN index
  (`documents_tags_idx`) gives exactly the "any of these tags" search this
  feature needs, no join required, minimal schema footprint. Consistent
  with the project's general preference for using Postgres's native
  feature set over introducing structure the current requirements don't
  call for.
- **Cons:** Ties this column to Postgres-specific array operators; a
  hypothetical future move to a different database would need to
  reintroduce a join table then, not now.

**Chosen: Alternative D.**

---

### Alternative E: Build the OCR-related columns only when the OCR feature actually lands
- **Pros:** Smaller migration for this feature; no speculative schema.
- **Cons:** Feature 2 (upload + OCR) would then need its own migration
  purely to add `ocr_status`/`ocr_text`/`ocr_error` to an already-live
  `documents` table with existing rows, plus a backfill decision for rows
  inserted before that migration ran.

### Alternative F: Add `ocr_status`/`ocr_text`/`ocr_error` to the schema now, unused until Feature 2 (chosen)
- **Pros:** This feature's detail view already needs to render "OCR still
  processing" / "failed" / "not supported" states regardless of whether
  upload exists yet (rows can be seeded directly, e.g. for demos or
  support), so the columns are load-bearing for this feature's own
  acceptance criteria (AC-6), not purely speculative. Feature 2 then only
  needs to *write* these columns, no migration.
- **Cons:** A `check` constraint (`ocr_status in (...)`) and three columns
  ship before any code path writes anything but `'pending'`/`'done'` via
  direct test fixtures — acceptable, since the constraint itself is cheap
  and enforces the valid-state set from day one.

**Chosen: Alternative F.**

## 3. Structural Decision
`migrations/*_create_documents.sql` adds `documents` (`tenant_id`/`user_id`
FKs, `tags text[] not null default '{}'` with a GIN index, `date_issued
date`, and the OCR triad above with a `check` constraint on `ocr_status`).
`DocumentId(Uuid)` is added to `src/domain.rs` following the existing
`TenantId`/`UserId` `#[sqlx(transparent)]` pattern, though the handlers
currently pass `Uuid` directly at the query boundary (`sqlx::query_as!`'s
generated struct fields are plain `Uuid`) rather than threading
`DocumentId` through — consistent with `TenantId`/`UserId`'s own usage,
where the newtype exists for the type-safety boundary at the extractor,
not necessarily every internal call site.

`src/web/forms.rs` gains two newtypes: `Tags` (comma-separated input,
capped at 20 tags / 50 chars each, stored as `Vec<String>`) and
`DateIssuedField` (hand-parses `YYYY-MM-DD` since `time`'s `macros`/
`parsing` Cargo features aren't enabled elsewhere in this project — adding
them for one call site wasn't judged worth a new feature flag). The
existing `ProfileField` newtype is reused as-is for the document `title`,
since it has the same "trim, cap length, blank means clear" semantics
already.

`AppWebError` gains a `NotFound` variant mapped to a plain `404` (not a
redirect, unlike `Unauthenticated`) — deliberately generic so a
cross-tenant document-id guess is indistinguishable from a genuinely
nonexistent id (AC-7/AC-9).

`src/web/handlers/documents.rs` provides `list`/`show`/`update`, mounted on
the router's existing `protected` sub-router
(`/documents` GET, `/documents/:id` GET+POST). `show`/`update` both scope
their query with `where id = $1 and tenant_id = $2`, so a row that exists
but belongs to another tenant is indistinguishable at the SQL level from
one that doesn't exist at all — `fetch_optional` /
`rows_affected() == 0` both collapse to the same `AppWebError::NotFound`.

One real routing bug surfaced during implementation: the route was
initially written as `/documents/{id}` (axum 0.8/matchit 0.8 syntax), but
this project pins `axum = "0.7"` (`matchit = "0.7.3"`), which requires
`:id` — the mismatch silently 404'd for every caller, including the
document's own owner, rather than failing to compile. Fixed by using
`/documents/:id`; recorded in `docs/ARCHITECTURE.md`'s gotchas section
since it's a project-wide trap, not specific to this feature.

## 4. OpenTelemetry Implications
`list`, `show`, and `update` all carry
`#[tracing::instrument(skip(state, tenancy))]` (and additionally
`skip(form)` on `update`) — no document title, tag, extracted OCR text, or
other document content is ever captured as a span attribute, matching the
existing PII-redaction idiom. `TenantContext`'s span-attribute tagging
(`tenant.id`/`user.id`, see TDR 003/004) applies unchanged via the same
`route_layer` middleware.
