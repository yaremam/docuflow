# TDR 014: Document Language Field

## 1. Context & Architectural Requirements
`documents` has no language column today. Feature 011 made OCR itself
language-agnostic within its supported set (`tesseract -l eng+rus` picks
the right script per block automatically, see TDR 011), which is why
DocuFlow has gotten this far without ever needing to know a document's
language — but the upcoming smart-filters panel wants to facet by
language, which means the language has to actually be recorded somewhere
first. This feature only adds the field and populates it; the facet UI is
a separate, already-sequenced-after feature. Per CLAUDE.md: zero-panic (no
confident match just leaves the field unset), PII-safe spans (OCR text
never enters a span), tenant-scoped queries throughout, and — per the
user's own framing of "compulsory" (confirmed 2026-07-13) — the field
must end up populated for the common case without ever blocking a save.

## 2. Alternatives Evaluated

### Alternative A: A small text-detection crate (`whatlang`), script-level for the non-Latin bucket, language-level for English
- **Pros:** `whatlang` is a pure-Rust, no-`unsafe`, no-system-dependency
  crate exposing both a fast script-only detector (`detect_script`,
  Latin/Cyrillic/etc. purely from Unicode character classes — no
  ambiguity between languages that share a script) and a full
  language-level detector (`detect`, with `.lang()`/`.confidence()`/
  `.is_reliable()`). Reuses `ocr_text` that `run_ocr` already has in
  memory at the exact point it writes `ocr_suggested_date_issued`
  (feature 012's `extract_issued_date` call site) — same shape of change,
  same instrumentation boundary, no new span. Using script detection for
  the non-Latin bucket means any Cyrillic-script document — regardless of
  which specific language it's actually written in — lands there, which
  is both the technically correct match for what `tesseract -l eng+rus`
  actually produces (one shared Cyrillic trained-data pack, not a
  per-language one) and the only practical option: a specific-language
  detector for that bucket would need real text in that language to
  validate against, which isn't available to write or test here (see
  below).
- **Cons:** A new Cargo dependency (small: no transitive dependencies of
  its own beyond `hashbrown`). Script-level detection for the non-Latin
  bucket is coarser than language-level would be — it can't and doesn't
  try to distinguish which specific Cyrillic-script language a document
  is in. That's an accepted tradeoff, not a gap to close later with more
  granularity in this same bucket (see §3).

### Alternative B: Full language-level detection for both buckets (matching a specific `Lang` variant on both sides)
- **Pros:** More precise in principle — the non-Latin bucket would only
  match documents in one specific language rather than any document using
  that script.
- **Cons:** Rejected during implementation (2026-07-13): validating this
  approach requires a real test fixture written in that specific
  language, which isn't something this project can produce or commit to
  its test suite. Independent of that constraint, it's also not a better
  technical fit — `tesseract -l eng+rus` OCRs any Cyrillic-script
  document through the same single trained-data pack regardless of which
  specific language it's actually in, so a language-specific detector
  would just as often misfire on other Cyrillic-script text as correctly
  match, without the actual OCR pipeline discriminating between them
  either.

### Alternative C: No auto-detection — a manual-only dropdown, defaulting to blank
- **Pros:** Zero new logic, zero new dependency.
- **Cons:** Directly contradicts the backlog item's "try to recognize
  automatically" requirement and the user's confirmed reading of
  "compulsory" (every document should end up with a value without a user
  having to act) — would leave every existing and new document blank
  forever unless someone manually visits and sets it.

## 3. Structural Decision
We choose **Alternative A**. Add nullable `documents.language text check
(language is null or language in ('en', 'cyr'))` via migration — same
`check`-constraint-on-a-plain-`text`-column idiom `ocr_status` already
uses, not a Postgres enum type (consistent, and avoids an
enum-alteration migration if a third bucket is ever added). The stored
code is `cyr` (script-based) rather than any specific language's own
identifier, matching what's actually being detected.

Add `src/language_detect.rs` with `pub fn detect(text: &str) -> Option<&'static str>`:
1. If `whatlang::detect_script(text) == Some(Script::Cyrillic)`, return
   `Some("cyr")` immediately — script identification doesn't need a
   confidence gate the way language identification does (a character
   either is or isn't in the Cyrillic Unicode ranges; there's no
   ambiguity between similarly-scored candidates the way there is
   between, say, English and French).
2. Otherwise, run full detection (`whatlang::detect`) and return
   `Some("en")` only when both `info.is_reliable()` and `info.lang() ==
   Lang::Eng` — Latin script alone would be too broad here (it covers
   dozens of languages DocuFlow has no OCR support for), so the Latin
   side stays language-specific where the Cyrillic side doesn't need to
   be.
3. Anything else returns `None`.

This asymmetry is deliberate: OCR only has one Latin-script trained-data
pack (`eng`) and one non-Latin one (`rus`, covering Cyrillic script
generally, not one specific language within it) — the detection strategy
for each bucket matches what the OCR pipeline underneath it actually is.

`run_ocr` (`src/web/handlers/documents.rs`) calls this once, right next to
its existing `extract_issued_date` call, and folds the result into the
same guarded write:

```sql
update documents set ocr_status = 'done', ocr_text = $3, ocr_suggested_date_issued = $4,
       language = coalesce(language, $5)
where id = $1 and tenant_id = $2
```

`coalesce(language, $5)` is the "never overwrite" guarantee from AC-2 —
cheaper than `accept_suggested_date`'s separate guarded-`UPDATE`-then-
fallback-existence-check pattern, because this is folded into an `UPDATE`
that's already unconditional on `id`/`tenant_id` (no separate "was this a
no-op or a 404" ambiguity to resolve, unlike a user-triggered action).

`web::forms` gets a `Language` newtype (`TryFrom<String>`, mirroring
`Tags`/`DateIssuedField`) accepting `""`, `"en"`, or `"cyr"` only — a
`document_show.html` `<select>` covers this closed set already, but the
newtype rejects anything else server-side too, matching the "type-driven
constraints" and "validate at boundaries" rules. `DocumentMetadataForm`
gains a `language: Language` field; `update` writes it unconditionally
(a manual edit is real user intent, not a guess — no `coalesce` needed on
that path, unlike `run_ocr`'s auto-write).

The upload form (`document_new.html`) gets no language field at all —
there's nothing to detect yet at that point (AC-4).

**Test fixture note:** the integration test proving the `cyr` bucket
works uses a genuine Ukrainian-language image fixture
(`tests/fixtures/ukrainian_sample.png`) rather than any other
Cyrillic-script language — script-level detection means the test doesn't
depend on which specific language the fixture is in, and Ukrainian is a
language this project can actually write and commit test content in.

## 4. OpenTelemetry Implications
`language_detect::detect` takes `text: &str` and, like
`extract_issued_date`, is called inline inside `run_ocr`'s existing
`#[tracing::instrument(skip(state))]` span — no new span. If ever worth a
span attribute, only the detected language code (`"en"`/`"cyr"`/absent)
is safe to record — never the OCR text itself, matching TDR 012 §4's
existing rule for this same span.
