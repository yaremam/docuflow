# User Story: Smart Filters Panel

## 1. User Value Statement
As a **logged-in DocuFlow user with a growing pile of documents**,
I want to **narrow the `/documents` list by tag, date issued, and
language using a facet panel, alongside the existing search box**,
So that **I can find a bill or policy by browsing what I actually have
instead of having to remember and type the right search term.**

## 2. Strict Acceptance Criteria
- **AC-1:** `GET /documents` renders a left-hand "Smart filters" panel
  next to the results, alongside the existing search/sort toolbar (not
  replacing it). The panel has three facet groups: **Tags**, **Date
  issued**, and **Language**.
- **AC-2:** The Tags facet lists the tenant's top 10 tags by document
  count (most-used first), each with a count, as checkboxes. Checking one
  or more tags narrows the results to documents carrying *all* of the
  checked tags (an AND-narrowing filter, independent of and additive to
  the existing free-text search box, which keeps its current
  any-of-these-tags OR behavior unchanged).
- **AC-3:** The Date issued facet lists years with a document count,
  newest first; selecting a year narrows to documents issued in that
  year and reveals a month breakdown (with counts) for that year alone.
  Selecting a month further narrows to that year+month. A separate
  "Undated" option filters to documents with no `date_issued` at all, and
  can be selected together with a year/month (OR'd in) or on its own. At
  most one year (optionally narrowed to one month) is active at a time —
  selecting a different year replaces the previous selection rather than
  adding to it.
- **AC-4:** The Language facet lists English, Cyrillic, and "Not set"
  (the three values feature 014 already produces), each with a count, as
  checkboxes. Checking one or more narrows to documents matching any of
  the checked values — OR-within-group, unlike the Tags facet's
  AND-within-group behavior in AC-2 — see TDR 015 §3 for the full
  per-facet semantics table and the reasoning for the difference.
- **AC-5:** All three facets AND together, and AND with the existing `q`/
  `sort` toolbar (e.g. searching "verizon", sorting by date issued, and
  checking the "2026" year and "English" language all apply at once).
- **AC-6:** Every facet checkbox is a real link/checkbox inside the
  existing toolbar `<form method="get">` — selecting one resubmits the
  page with the new query params added. No client-side-only filtering;
  the feature works with JavaScript disabled (progressive enhancement,
  matching every other form on this project).
- **AC-7:** Active filters render as a row of removable chips above the
  results (e.g. "insurance ✕", "2026 ✕", "English ✕"). Each chip is a
  link that resubmits with just that one filter removed. A "Clear all"
  control resets every facet filter while leaving `q` and `sort` alone.
- **AC-8:** When facets narrow the list to zero documents, the page shows
  a distinct "No documents match these filters" message with a link to
  clear filters — not the existing "No documents yet" first-run empty
  state, which stays reserved for a tenant with literally zero documents.
- **AC-9:** With no facet params in the URL, `/documents` behaves exactly
  as it does today (existing `tests/documents_list.rs` coverage keeps
  passing unchanged) — the panel is additive, not a breaking change to
  the default view.
- **AC-10:** Facet counts (next to each checkbox) reflect the tenant's
  total documents for that facet value, not narrowed by whichever other
  facets are currently active — see TDR 015 §2/§3 for why this is an
  accepted v1 simplification rather than a bug.
- **AC-11:** Tenant scoping is unchanged — every new query (results and
  facet counts) filters by `tenant_id`, no new route.
- **AC-12:** No `.unwrap()`, `.expect()`, or `panic!()` introduced in the
  changed code.
- **AC-13:** No document title, tag value, filter selection, raw file
  bytes, or extracted OCR text enters trace spans or logs from the
  changed `list` handler — see TDR 015 §4 (this tightens, not just
  preserves, today's behavior: the existing `list` handler does not
  currently skip its `Query<ListQuery>` parameter).

## 3. Explicitly out of scope this round
- **Saved/named smart collections** (persisting a filter combination) —
  the separately-sequenced next feature (016).
- **Facet counts that shrink as other facets are applied** ("2 of these
  14 insurance docs are also from 2026"). Real faceted-search UIs
  typically recompute each facet's counts against every *other* active
  facet; this round computes every facet's counts against the tenant's
  full, unfiltered set instead, to avoid an explosion of extra per-facet
  queries. See TDR 015 §2, Alternative B.
- **Selecting more than one year/month at once** (e.g. "2026 or 2024").
  The date facet supports one active year (optionally narrowed to one
  month), plus an independently OR'd-in "Undated" toggle.
- **Any new tag/date/language vocabulary.** This reuses exactly what
  already exists (arbitrary user tags, `date_issued`, and feature 014's
  closed `en`/`cyr`/unset language set).
</content>
