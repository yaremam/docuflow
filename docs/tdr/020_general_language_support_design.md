# TDR 020: General Language Support (German, Dutch, Ukrainian OCR + full-world tagging)

## 1. Context & Architectural Requirements
Feature 014 gave `documents.language` a closed, script-level vocabulary
(`en`/`cyr`) because that's all `tesseract -l eng+rus` (feature 011) could
actually back up with real OCR quality — TDR 014 §3 explicitly deferred
"a specific language within [the Cyrillic bucket]" as future scope. The
backlog (raised 2026-07-13, the same day) asks for two related but
distinct things: (a) proper Ukrainian OCR — a dedicated trained-data pack
instead of the generic Russian-trained Cyrillic bucket, which misses
Ukrainian-only letters like і/ї/є/ґ — plus German and Dutch, which feature
014 never covered at all; and (b) a language field flexible enough for
"any language there is," not just whatever OCR happens to support.

The user's explicit direction (confirmed 2026-07-13, in order):
- German and Dutch are firm requirements alongside Ukrainian; Serbian is
  explicitly out of scope for now.
- Russian OCR is dropped entirely ("fuck russian") — not kept alongside
  the new packs.
- OCR pack coverage: a **curated set** (eng/deu/nld/ukr) now, extensible
  later — not `tesseract-ocr-all` (rejected: ~4GB image growth for
  coverage this project doesn't need yet).
- The language field/dropdown itself: a **full ISO 639-1 picker** now
  (~180 languages), even though OCR quality only backs 4 of them — not a
  curated dropdown restricted to what OCR supports.

Per CLAUDE.md: zero-panic, PII-safe spans, tenant-scoped queries, and a
mockup-first sign-off for the two changed screens (the edit-page language
field and the list-page language facet) before any template/handler code
— see the signed-off artifact from this conversation.

## 2. Alternatives Evaluated

### Alternative A: Keep OCR engine as Tesseract, just widen the trained-data set
- **Pros:** Tesseract already supports 100+ languages including proper
  `deu`/`nld`/`ukr` packs; the Ukrainian quality problem was never a
  Tesseract weakness, just a missing pack + a script-bucket schema that
  couldn't record anything more specific. Zero architecture change —
  `ocr::run_tesseract` already shells out to one binary, per CLAUDE.md's
  OCR Engine Layer rule; this only changes its `-l` argument and the
  Dockerfile's `apt-get install` line.
- **Cons:** None specific to this project — the case for switching engines
  (e.g. PaddleOCR, cloud vision APIs) was raised and rejected in
  conversation: Python/PyTorch runtimes clash with the Rust-only,
  zero-panic architecture, and cloud OCR APIs mean shipping bills/
  insurance/contract images to a third party, which conflicts with this
  project's self-hosted stack (Postgres/MinIO/Mailpit) and its existing
  PII-sanitization discipline (CLAUDE.md §3).

### Alternative B: `documents.language` stays a CHECK-enumerated closed list, just with more values (e.g. `en`/`de`/`nl`/`uk`/`cyr`)
- **Pros:** Minimal migration change from feature 014's existing pattern.
- **Cons:** Directly contradicts the user's explicit "any language there
  is" requirement and the chosen full-ISO-639-1-picker UX — a handful of
  enumerated values can't back a ~180-language dropdown, and every future
  language would need its own migration, which is exactly what feature
  014's original CHECK-constraint choice was trying to avoid in the first
  place (TDR 014 §3).

### Alternative C (chosen): `documents.language` accepts any ISO 639-1 code, validated in application code against the `isolang` crate; OCR stays limited to a curated pack set
- **Pros:** Matches both explicit decisions at once — the field is
  genuinely open to "any language there is" (full ISO 639-1 table, via
  `isolang`'s `list_languages` feature) while OCR quality is honestly
  scoped to only the 4 languages that have real trained-data packs
  installed. Adding a 5th OCR-supported language later is a Dockerfile
  line + a `languages::OCR_SUPPORTED` entry + a `language_detect::detect`
  match arm — no schema change, no new migration, consistent with TDR
  014's original migration-avoidance rationale (more so now that ~180
  codes are valid instead of 2).
- **Cons:** Auto-detection can only ever propose 4 specific codes even
  though the field accepts ~180 — an intentional asymmetry (see §3), not
  a gap: it means "detected" and "manually tagged" are visibly different
  qualities of signal, which the UI mockup makes explicit via two
  `<optgroup>`s ("OCR-supported" vs. "All languages").

## 3. Structural Decision
We choose **Alternative A + C**.

**Migration** (`20260713205400_generalize_documents_language.sql`): drop
feature 014's `language in ('en', 'cyr')` CHECK, null out existing `cyr`
rows (they predate this feature and don't map to one specific language —
guessing Russian vs. Ukrainian would be worse than leaving them for
reprocess-OCR or a manual re-tag to repopulate), then add back a
shape-only guard (`language ~ '^[a-z]{2}$'`). The **real** validation
authority is application code, not the DB: `src/languages.rs`'s
`is_valid` (backed by `isolang::Language::from_639_1`), used by both the
`Language` form newtype (`web::forms`) and available for any other
call site — one source of truth for "is this a real ISO 639-1 code"
rather than the CHECK constraint and the newtype maintaining separate
copies of a ~180-entry list.

**`src/languages.rs`** (new module) is the one place that knows:
- `OCR_SUPPORTED: [&str; 4] = ["en", "de", "nl", "uk"]` — must stay in
  sync with `ocr::run_tesseract`'s `-l` argument and the Dockerfile's
  `apt-get install` line (all three changed together in this feature).
- `is_valid(code) -> bool` and `display_name(code) -> String` (English
  name via `isolang`, falling back to the raw code on a lookup miss —
  cheap insurance for pre-migration data, never expected to hit for
  anything written after this feature ships).
- `supported_options()` / `other_options()` — the two `<optgroup>` lists
  for `document_show.html`'s dropdown, the latter alphabetical by name
  and excluding whatever's in `OCR_SUPPORTED` to avoid duplicate entries.

**`src/ocr.rs`**: `run_tesseract`'s `-l eng+rus` becomes `-l
eng+deu+nld+ukr` — same multi-language-mode idiom TDR 011 established
(Tesseract picks the best-matching trained data per block internally),
just a wider pack list. **`Dockerfile`**: `tesseract-ocr-rus` is replaced
with `tesseract-ocr-deu tesseract-ocr-nld tesseract-ocr-ukr` (Russian
pack fully retired, per the user's explicit direction — not kept
alongside the new ones).

**`src/language_detect.rs`**: rewritten from feature 014's
script-then-language-bucket logic to a flat match on `whatlang`'s
detected `Lang`, restricted to the 4 OCR-supported codes:

```rust
match info.lang() {
    Lang::Eng => Some("en"),
    Lang::Deu => Some("de"),
    Lang::Nld => Some("nl"),
    Lang::Ukr => Some("uk"),
    _ => None,
}
```

This is deliberately never widened to whatever `whatlang` itself can
detect (it supports ~69 languages) — detection must only ever reflect
what OCR actually read, not guess at a language OCR wasn't tuned for
(confirmed in the signed-off mockup's design note). A document can still
be manually tagged as any of the ~180 other languages via the full
picker; it just never gets auto-proposed.

**`document_show.html`**: the `<select>` gains two `<optgroup>`s —
"OCR-supported" (the 4 curated languages) and "All languages" (everything
else, alphabetical) — populated from `DocumentShowTemplate`'s new
`supported_language_options`/`other_language_options` fields
(`Vec<languages::LanguageOption>`). The upload form still gets no
language field at all (TDR 014 AC-4 stands unchanged).

**`documents.rs`'s `list` handler**: feature 018's 3 hardcoded per-facet
count queries (`en`/`cyr`/`unset`) are replaced with the same
discover-then-narrow-count pattern the tag facet already uses (`select
distinct language ... group by`-style discovery, then `count_documents`
narrowed per candidate) — necessary now that the candidate set is
per-tenant and unbounded rather than a fixed 3 values.
`LanguageFacetOption.value`/`.label` move from `&'static str` to owned
`String` accordingly. `documents_list.html`'s facet loop needed **no**
template change — it already iterated `language_facets` generically.

## 4. OpenTelemetry Implications
No new spans. `language_detect::detect` stays called inline inside
`run_ocr`'s existing `#[tracing::instrument(skip(state))]` span, same as
TDR 014 §4 — if ever worth a span attribute, only the detected code
(`"en"`/`"de"`/`"nl"`/`"uk"`/absent) is safe to record, never OCR text.

## 5. Test Fixtures
`tests/fixtures/german_sample.png` and `dutch_sample.png` were generated
the same way as feature 014's `ukrainian_sample.png` (no `PIL`/
ImageMagick available on this host — a tiny inline-styled HTML file
rendered via `google-chrome --headless --screenshot`), each genuinely
German/Dutch text (not just Latin-script filler) so the corresponding
`language_detect` + end-to-end OCR tests exercise real detection rather
than a fixture that happens to pass. `tests/documents_upload.rs`'s
existing Cyrillic-script OCR test now soft-skips on `tesseract-ocr-ukr`
rather than `-rus`, matching the retired pack.
