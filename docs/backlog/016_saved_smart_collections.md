# User Story: Saved Smart Collections

## 1. User Value Statement
As a **logged-in DocuFlow user who's built a useful filter combination**
on the `/documents` smart filters panel (feature 015),
I want to **save that combination as a named collection I can jump back
to in one click**,
So that **I don't have to re-check the same tags/dates/language every
time I want to see "my medical bills" or "this year's utilities."**

## 2. Strict Acceptance Criteria
- **AC-1:** `GET /documents` shows a "My collections" panel, above the
  existing smart-filters panel (feature 015), listing every saved
  collection for the tenant as a link plus a live count of how many
  documents currently match it. Newest-saved first.
- **AC-2:** Clicking a collection link navigates to that collection's
  exact saved `/documents?...` view (search text, sort, and every active
  facet from feature 015) — reusing `list`'s existing query parsing with
  no new filter-matching logic of its own.
- **AC-3:** A "Save this search as..." control (a name field + Save
  button) appears above the results whenever at least one filter is
  currently active (any feature-015 facet checked, or the free-text
  search box non-empty) — not shown on the bare, unfiltered view, where
  there'd be nothing meaningful to save.
- **AC-4:** Saving requires a non-empty name (reasonable max length,
  matching the project's existing short-text-field convention). The
  server re-derives and validates "was a filter actually active" from
  the submitted state itself — a crafted request that skips the UI can't
  save a no-op collection pointing at the bare unfiltered view.
- **AC-5:** Each collection has a one-click delete control — **no
  confirmation page**, unlike document deletion (TDR 016 §3 explains why
  this is a deliberate deviation from that precedent, not an
  inconsistency).
- **AC-6:** Collections are tenant-scoped throughout (list, apply,
  delete) — a collection from one tenant is never visible, appliable, or
  deletable by another.
- **AC-7:** A collection's live count reflects the tenant's current
  documents each time `/documents` renders — never a snapshot frozen at
  save time (a document added or deleted after saving is reflected next
  time the panel loads).
- **AC-8:** No `.unwrap()`, `.expect()`, or `panic!()` introduced in the
  changed code.
- **AC-9:** No document title/tag value, collection name, or saved
  filter query string enters trace spans or logs from the new handlers —
  matching feature 015's precedent of skipping filter-shaped parameters
  from `#[tracing::instrument]`.
- **AC-10:** No practical cap on the number of saved collections per
  tenant this round.

## 3. Explicitly out of scope this round
- **Renaming an existing collection.** Delete and re-save covers the
  same ground for now.
- **Reordering or pinning collections.** Always newest-saved-first.
- **Sharing a collection with other tenants/users**, or any
  multi-tenant-membership concept — ties to the same not-yet-built
  multi-user-tenant work already listed in `docs/ARCHITECTURE.md` §8.
- **Auto-updating a collection's saved filters** if feature 015 ever
  grows a new facet type — a collection is a frozen bookmark of a query
  string at save time; an old collection saved before a hypothetical new
  facet existed simply doesn't reference it, same as any bookmarked URL.
