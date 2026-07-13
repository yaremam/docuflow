# User Story: Narrow Smart-Filter Facet Counts by Active Facets

## 1. User Value Statement
As a **logged-in DocuFlow user narrowing my documents with the smart
filters panel** (feature 015),
I want to **see each remaining facet option's count reflect the filters
I've already applied**,
So that **the numbers next to Tags/Date issued/Language tell me what
I'll actually get if I check that box next, not an unrelated total.**

## 2. Strict Acceptance Criteria
- **AC-1:** Each Tags facet option's count reflects documents matching
  that tag *combined with* whichever Date issued/Language facets (and
  free-text search) are currently active — not the tenant's unfiltered
  total.
- **AC-2:** Each Date issued facet option's (year, month, or "Undated")
  count reflects documents matching that date option combined with
  whichever Tags/Language facets (and search) are currently active.
- **AC-3:** Each Language facet option's count reflects documents
  matching that language combined with whichever Tags/Date issued facets
  (and search) are currently active.
- **AC-4:** With no facets active, every count is unchanged from today
  (equal to the tenant's unfiltered total) — narrowing only has an
  effect once at least one other facet is checked. Existing test
  coverage (`filters_panel_shows_tag_counts`) keeps passing unmodified.
- **AC-5:** The *set* of tags/years shown in each facet stays the
  tenant's full set regardless of active filters — only the numbers next
  to them narrow, not which options appear. A tag or year with a
  narrowed count of zero still shows (as `0`, not hidden), so a user can
  see "this combination doesn't exist" rather than the option vanishing.
- **AC-6:** Feature 016's saved-collection counts are **unchanged** — a
  collection's count already reflects its own complete saved filter
  (every dimension at once), which isn't the same shape as "one facet
  option's count against every *other* active facet." There's nothing to
  narrow there; see TDR 018 §1 for why this is a deliberate no-op, not an
  oversight.
- **AC-7:** No `.unwrap()`, `.expect()`, or `panic!()` introduced.
- **AC-8:** No new PII in spans/logs — this only changes which bind
  values existing, already-audited count queries receive.

## 3. Explicitly out of scope this round
- **Batching the extra count queries into fewer round trips.** Each
  facet option gets its own `count(*)` query — more queries per page
  load than today, deliberately accepted for personal-scale document
  counts rather than a more complex batched query. See TDR 018 §2
  Alternative B.
- **Narrowing which tags/years appear**, not just their counts (AC-5).
</content>
