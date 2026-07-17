# TDR 031: Expiry Dates & Renewal Reminders

## 1. Context & Architectural Requirements
Backlog item: track a document's expiry date, suggest it from OCR text,
and warn the user before (or after) it lapses. Per CLAUDE.md: zero-panic,
mockup-before-code (see the signed-off mockup this TDR implements), no
new PII in spans.

## 2. Alternatives Evaluated

### Alternative A: Freeform date_expires, no doc_type gating (mirroring date_issued exactly)
- **Pros:** Simplest, fully consistent with the one precedent that
  already exists.
- **Cons:** Rejected by explicit direction — a bill/receipt structurally
  never has an expiry date, and showing the field/attempting a
  suggestion on every document adds clutter with no payoff for most
  doc_types.

### Alternative B: Scheduled reminder emails via the existing Mailer
- **Pros:** Matches the backlog item's original wording ("reminder
  emails via existing mailer/Mailpit stack").
- **Cons:** Rejected this round, redirected mid-design. The app has
  **no scheduling infrastructure anywhere** — every `tokio::spawn` in
  the codebase today is a one-shot, per-request task (OCR jobs, a
  password-reset email), never a recurring loop. Building email
  reminders would mean introducing the app's first "run this
  periodically" mechanism (an in-process `tokio::time::interval` loop,
  or an external trigger hitting a new admin endpoint) *and* a new
  HTML/templated email body (the `Mailer` today sends one plain-text
  message, no templating at all) in the same round as everything else
  below. Redirected to a live, pull-based in-app notification instead —
  no new infrastructure, and the underlying "what's expiring" query is
  written so a future email extension can reuse it directly.

### Alternative C: date_issued-style no-keyword expiry extraction
- **Pros:** Reuses `extract_issued_date`'s exact shape with zero new
  logic.
- **Cons:** Rejected. Most documents only print one or two dates. With
  no keyword anchoring, the expiry "suggestion" would very likely just
  re-find whatever `date_issued`'s extraction already claimed, or
  arbitrarily pick the next date-shaped text in the document — a
  suggestion that doesn't actually mean "this is when it expires."

### Alternative D: Keyword-anchored expiry extraction, reusing date_extract's date-shape recognizers
- **Pros:** A trigger phrase ("expires," "gültig bis," "vervaldatum,"
  "дійсний до," …) must appear near a candidate date before it's
  accepted — a real, distinct signal from `date_issued`'s. Reuses
  `find_iso`/`find_month_name`/`find_numeric` (now `pub(crate)`) as the
  "what does a date look like" building block, so the two extraction
  modules can't drift on what counts as a valid date.
- **Cons:** One new module, one new regex pass per document. Negligible
  cost — this only ever runs once per document in the background OCR
  worker, same as every other `date_extract`/`doc_type_extract` pass.

### Alternative E: date_expires facet mirrors date_issued (year/month breakdown)
- **Pros:** Visual/structural consistency with the existing facet.
- **Cons:** Rejected. A calendar breakdown answers "which year was this
  issued" — a historical fact. Expiry is action-oriented ("is this a
  problem soon"), which a year/month breakdown doesn't answer directly
  without mental math on every render.

### Alternative F: date_expires facet as status buckets (Expired/Expiring soon/Later/No expiry set)
- **Pros:** Directly answers the action-oriented question; reuses the
  OR-combined-checkbox interaction tags/language/doc_type already have
  (not date_issued's single-active-year rule, which doesn't fit four
  independent buckets). `src/web/facets.rs`'s `assemble_facet_options`
  (TDR 028) applies unchanged — this is exactly the "add a 5th/6th
  facet" case that refactor was designed to make cheap.
- **Cons:** None identified.

## 3. Structural Decision
We choose **Alternative B redirected** (live in-app notification, no
scheduler/email this round), **Alternative D** (keyword-anchored
extraction), and **Alternative F** (status-bucket facet).

**Schema**: `documents.date_expires date` + `documents.
ocr_suggested_date_expires date` (both nullable), mirroring `date_issued`/
`ocr_suggested_date_issued`'s exact shape.

**`src/expiry_extract.rs`** (new): `pub fn extract_expiry_date(text: &str)
-> Option<Date>`. A trigger-phrase alternation regex (case-insensitive)
finds each occurrence of a keyword; the ~30 characters immediately after
each match are tried against `date_extract::find_iso`/`find_month_name`/
`find_numeric` (all promoted from private `fn` to `pub(crate)`, alongside
the shared `cached_regex` helper) in that priority order, returning the
first valid date found near *any* trigger occurrence. Trigger phrases,
sourced rather than guessed (the same discipline feature 030's Ukrainian
month names used):
- **English**: `expires`, `expiry date`, `expiration date`, `valid until`.
- **German**: `gültig bis` ("valid until"), `ablaufdatum` ("expiration
  date") — confirmed via [Linguee/dict.cc German-English insurance
  terminology](https://www.linguee.de/englisch-deutsch/uebersetzung/expiry+date+of+insurance.html).
- **Dutch**: `geldig tot` ("valid until"), `vervaldatum` ("expiration
  date") — confirmed via [Linguee Dutch-English](https://www.linguee.com/dutch-english/translation/vervaldatum.html).
- **Ukrainian**: `дійсний до` ("valid until"), `термін дії` ("term of
  validity") — confirmed via [Reverso Context Ukrainian legal/insurance
  usage](https://context.reverso.net/translation/english-ukrainian/expiration+date).

**`run_ocr`** gains `ocr_suggested_date_expires = crate::expiry_extract::
extract_expiry_date(&text)` alongside its existing suggestion
computations, written in the same `UPDATE` as everything else OCR
produces.

**Eligibility gate**: a document is expiry-eligible when its *confirmed*
`doc_type` (not `ocr_suggested_doc_type`) is one of `insurance`,
`contract`, `bill`, `id`. Computed with a small `fn is_expiry_eligible
(doc_type: Option<&str>) -> bool` next to `doc_type_extract`'s own
taxonomy, reused by: whether `show()` renders the `date_expires` field/
suggestion box at all, and the facet's "No expiry set" bucket's
candidate population.

**Dashboard strip**: computed in `list()` (only relevant on the default,
unfiltered dashboard view — see note in §4) via one query — expiry-
eligible documents where `date_expires <= today + 14 days` (already-
expired included, no lower bound), ordered soonest-first (an expired
document sorts before one still 14 days out, since it's more urgent).
Rendered as a new `expiring_documents: Vec<ExpiringDocument>` template
field; absent from the page entirely when empty.

**Facet**: `ActiveFilters` gains `expiry_status: Vec<String>` (values
`"expired"`/`"soon"`/`"later"`/`"unset"`, OR-combined — same shape as
`lang`/`doc_type`'s already-established "list of strings, "unset" is a
sentinel" convention, reusing that exact pattern rather than inventing a
new one). `FacetFilters`/`count_documents` gain the matching dimension;
`list`'s 5 sort-mode `query_as!` arms gain the WHERE condition (mirroring
how `doc_type` was added in feature 024). The facet options themselves
are hand-built (not discovered from distinct column values, since the
four buckets are fixed, not data-driven) and run through `assemble_
facet_options` as usual.

## 4. Explicitly Deferred
- **Email/other-channel reminders**, **a configurable threshold**, **a
  date_expires calendar facet** — see backlog §3.
- **The dashboard strip's interaction with active search/facets** — it
  only needs to answer "what needs my attention right now," so it's
  computed from the tenant's full expiry-eligible set regardless of
  whatever facets/search are currently applied to the results list below
  it, not narrowed by them. Revisit if that reads as confusing in
  practice.

## 5. OpenTelemetry Implications
No new spans. `date_expires` and the OCR text it's matched against
follow the same PII rule `ocr_text`/`date_issued` already do — kept out
of any span; `run_ocr`'s span already isn't instrumented per-field, and
`list`/`show`'s existing `skip(...)` lists already cover the request
params this feature adds (no new query param carries free text — the
facet's values are the fixed strings `"expired"`/`"soon"`/`"later"`/
`"unset"`, not user-typed content).
