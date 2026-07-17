# TDR 028: Collapse the Facet Scaffolding Into `src/web/facets.rs`

## 1. Context & Architectural Requirements
Not a user-facing feature — a codebase-health refactor, prompted by an
architecture review of `src/web/handlers/documents.rs` (1964 lines,
touched in 12 of the last 30 commits, grown by nearly every feature since
012). Two findings from that review, both load-bearing by the deletion
test:

1. Five facets (tags, date year/month, undated, language, doc_type) each
   repeated the same "discover candidates → narrow each one's count via
   `count_documents` → mark checked → push an option" shape (`documents.
   rs:676-952` before this refactor), plus a sixth near-copy for applied-
   filter chips ("given active state, drop one value, rebuild a link").
2. `ListQuery` (deserialized query params), `FacetFilters` (the narrowing
   shape `count_documents` takes), and the positional args `build_query_
   string`/`build_documents_url` took were three parallel shapes of the
   same "current filter state," with no single owner.

Per CLAUDE.md: zero-panic, TDD (tests first), no new PII in spans.

## 2. Alternatives Evaluated

### Alternative A: A `Facet` trait, one impl per dimension
- **Pros:** Each dimension gets a named home (`TagsFacet`, `YearFacet`, …).
- **Cons:** Rejected. There's exactly one production adapter per
  dimension — no runtime substitution ever happens — so a trait would be
  a hypothetical seam, not a real one ("one adapter = hypothetical seam,
  two = real"). More ceremony for no leverage gained.

### Alternative B: One generic async function, closures per call site, `Fn(&T) -> Fut` bound
- **Pros:** A single `narrow_counts` shared by all five facets — the
  original design coming out of the grilling session.
- **Cons:** Rejected on contact with the compiler, not on paper. A
  closure whose returned future borrows its own `&T` argument needs a
  higher-ranked lifetime bound that plain `Fn(&T) -> Fut` can't express;
  switching to `AsyncFn(&T) -> Result<i64, E>` (stable in this edition)
  fixed that error, but surfaced a second, deeper one — `implementation
  of Send is not general enough` — where axum's handler-future `Send`
  requirement can't be proven through the higher-ranked async-closure
  bound in current stable Rust. Not fixable without either boxing every
  future (`Pin<Box<dyn Future + Send>>`, extra allocation and ceremony
  on every one of the ~9 call sites) or destabilizing the whole handler's
  `Send`-ness. A known rough edge, not a design mistake to route around
  at any cost.

### Alternative C: Split the two halves — plain per-facet fetch loop, one shared pure assemble function
- **Pros:** `count_documents`'s narrowed-count fetch is left as a small
  (~10-line), SQL-specific loop at each of the 5 call sites — genuinely
  coupled to `FacetFilters`/Postgres anyway, and too small to be worth
  fighting the compiler over. What actually was the repeated *logic*
  (decide checked, build the right option struct) collapses into one
  function, `assemble_facet_options`, used five times — pure, no I/O,
  unit-testable with plain Rust values and no Postgres or axum.
- **Cons:** Doesn't dedupe the fetch loop's ~10 lines × 5. Accepted —
  see Alternative B's cons for why that boilerplate is cheaper to leave
  in place than to generalize.

## 3. Structural Decision
We choose **Alternative C**, plus folding the "three shapes of filter
state" into one.

**`src/web/facets.rs`** (new, no dependency on the `documents` table or
SQL):
- **`ActiveFilters`** — normalizes `ListQuery` once (`q_tags`/`search_text`
  derivation, `lang`/`doc_type` "unset"-sentinel splitting, `date_month`
  gated by `date_year`). Deliberately excludes `sort`: sort is display-
  order state, not a filter dimension `count_documents` or `count_
  matching_documents` ever need, so the two URL builders take it as a
  separate parameter instead of forcing every caller (including a bare
  count) to invent a placeholder sort value.
- **`assemble_facet_options<T, O>`** — turns already-fetched `(candidate,
  narrowed count)` pairs into caller-shaped options (`TagFacetOption`,
  `LanguageFacetOption`, …) via two closures (`is_checked`, `build`); pure,
  unit-tested directly (14 tests, no DB).

**`documents.rs`** gains two small connective helpers: `active_filters
(&ListQuery) -> ActiveFilters` (one normalization site, used by `list`,
`count_matching_documents`, and `save_collection` so they can't drift) and
`base_facet_filters(&ActiveFilters) -> FacetFilters<'_>` (the "no
dimension narrowed" view; every `count_documents` call site now does
`FacetFilters { one_field: override, ..base_facet_filters(&active) }`
instead of writing all 10 fields itself). `build_query_string`/
`build_documents_url` take `(&ActiveFilters, sort: &str)`. Applied-filter
chips are built by cloning `active` and mutating its raw public fields
directly (`tags.retain(...)`, `date_year = None`, …) before calling
`build_documents_url` — safe specifically because those two builders only
ever read `ActiveFilters`' raw fields, never its derived private ones
(`q_tags()`/`lang_values()`/…), which a raw-field mutation doesn't
recompute; a mutated clone must never be passed to `count_documents`/
`base_facet_filters` for that reason (documented on the mutation site).

Migrated incrementally, one facet at a time (tags → year/month/undated →
language → doc_type → chips + the two URL builders + the main query's
own bind params), running `tests/documents_filters.rs`'s 28 existing
HTTP-level tests green after every step — a pure refactor, so those tests
are the acceptance criteria; no new user-facing behavior, no new backlog
user story.

**Net effect:** `documents.rs` 1964 → 1854 lines; `src/web/facets.rs` adds
187 lines (plus 168 lines of unit tests) that used to be either hand-
copied five times or untestable without Postgres and axum.

## 4. Explicitly Deferred
- **Deduplicating the per-facet narrowed-count fetch loop itself** — see
  Alternative B/C; blocked on Rust's current async-closure/`Send`
  interaction, not a design choice.
- **Turning `documents.rs` into a directory module** (`documents/list.rs`,
  `documents/show.rs`, …) — a bigger restructure than this refactor's
  scope; `list`/`show` themselves are unchanged in shape, only their
  facet/filter internals moved out.

## 5. OpenTelemetry Implications
None. No new spans; no `#[tracing::instrument]` signature changed by this
refactor (unlike feature 027, which did need one skip-list fix).
