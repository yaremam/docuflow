# User Story: Rename a Saved Smart Collection

## 1. User Value Statement
As a **logged-in DocuFlow user with saved smart collections** (feature
016),
I want to **rename an existing collection in place**,
So that **I don't have to delete and re-save it just to fix a typo or
reword it as my filtering habits change.**

## 2. Strict Acceptance Criteria
- **AC-1:** Each row in the "My collections" panel gets a rename control
  (an inline edit affordance next to the existing delete control) — a
  minor tweak to the existing panel, not a new screen, so no mockup
  sign-off is required per CLAUDE.md §5.
- **AC-2:** Submitting a rename updates only `smart_collections.name` —
  the collection's saved `query` (what it filters) and `id` are
  unchanged; its position in the newest-first list is unchanged (no
  `updated_at` reordering).
- **AC-3:** The new name is validated with the exact same rule an
  initial save already uses (`CollectionName`: non-empty after trimming,
  max 100 characters) — one rejection path, not a second one.
- **AC-4:** Tenant-scoped: renaming a collection belonging to another
  tenant is `404`, identical to `delete_collection`'s existing guarantee.
- **AC-5:** No `.unwrap()`, `.expect()`, or `panic!()` introduced.
- **AC-6:** No collection name (old or new) enters trace spans or logs —
  matching `save_collection`/`delete_collection`'s existing
  `skip(...)` precedent.

## 3. Explicitly out of scope this round
- **Editing a collection's saved filters** (its `query`) — only the name
  is editable in place; changing what a collection filters still means
  delete-and-re-save.
- **A rename history or undo.**
