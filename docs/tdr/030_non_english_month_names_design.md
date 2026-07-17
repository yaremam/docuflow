# TDR 030: Non-English Month Names in Date Extraction

## 1. Context & Architectural Requirements
Deferred item from feature 012/ARCHITECTURE.md §8: `date_extract.rs`
recognizes English month names and numeric/ISO shapes only, even though
tesseract has OCR'd German/Dutch/Ukrainian text since features 011/020.
Scope grew once during the design conversation: the user asked for
abbreviated forms (`Jan`, `mrt`, `бер`) across all 4 languages too, not
just full names — English retroactively included, since English
previously had no abbreviation support either. Per CLAUDE.md: zero-panic,
narrow fixed-shape scanner (TDR 012 §1), not a general date parser.

## 2. Alternatives Evaluated

### Alternative A: One flat, hardcoded `MONTH_NAMES` list, as today
- **Pros:** Simplest possible change — just append more `(name, Month)`
  pairs to the existing list.
- **Cons:** Rejected as the long-term shape (though it's exactly what
  feature 012 shipped for English-only). The user explicitly asked for
  the *next* language to be a two-line change, not a hunt through one
  undifferentiated list to figure out which entries belong to which
  language, or a risk of silently duplicating/missing an abbreviation
  while editing a flat list by hand.

### Alternative B: Detect the document's language first, then only try that language's month names
- **Pros:** Fewer opportunities for one language's short abbreviation to
  coincidentally collide with another's.
- **Cons:** Rejected. `language_detect::detect` and `date_extract::
  extract_issued_date` are two independent scans over the same OCR text
  today, run in either order with no dependency — introducing one here
  would be a new coupling for a benefit that doesn't materialize in
  practice: tesseract itself already OCRs without knowing the document's
  language (TDR 011), so gating *extraction* on a detected language
  would make date-recognition pickier than the OCR pass that produced
  the text it's reading.

### Alternative C: A per-language `const` table registry, flattened generically
- **Pros:** Adding a language is additive only — one new `const` table,
  one new entry in a registry slice, nothing else in the file changes.
  Every language is still tried in one combined pass (keeps Alternative
  B's rejection reasoning intact) — the registry only organizes the
  *data*, not the control flow.
- **Cons:** None identified — this is a strict readability/maintenance
  improvement over Alternative A with no runtime cost (the registry is
  flattened once, same `OnceLock`-cached pattern the regex itself
  already uses).

## 3. Structural Decision
We choose **Alternative C**.

```rust
const ENGLISH: &[(&str, Month)] = &[("january", Month::January), ("jan", Month::January), ...];
const GERMAN: &[(&str, Month)] = &[("januar", Month::January), ("jan", Month::January), ...];
const DUTCH: &[(&str, Month)] = &[...];
const UKRAINIAN: &[(&str, Month)] = &[...]; // genitive forms — see below
const LANGUAGES: &[&[(&str, Month)]] = &[ENGLISH, GERMAN, DUTCH, UKRAINIAN];
```

`month_name_alternation()` and `lookup_month()` both iterate
`LANGUAGES.iter().flatten()` instead of a single named list — everything
downstream (the regex-building, the case-insensitive lookup) is
unchanged, generic over however many language tables exist.

**Month-name data, sourced rather than guessed** (Ukrainian grammar
carries real accuracy risk — a wrong table fails silently, never
matching, rather than erroring):
- **German**: full names nominative (German doesn't decline them);
  abbreviations `Jan/Feb/Mär/Apr/Mai/Jun/Jul/Aug/Sep/Okt/Nov/Dez` — note
  `Mai`/`März` are conventionally *not* abbreviated in practice (already
  short), included anyway since a form that never appears in real text
  is harmless, not incorrect.
- **Dutch**: full names; abbreviations `jan/feb/mrt/apr/mei/jun/jul/aug/
  sep/okt/nov/dec` — `mei` likewise unabbreviated in practice.
- **Ukrainian**: **genitive** case full names (`січня`, `лютого`, …) —
  the form real dates use ("15 січня 2024"), confirmed against
  [Ukrainian Lessons' dates guide](https://www.ukrainianlessons.com/dates-in-ukrainian/).
  Abbreviations follow the sourced "first three letters" convention
  (`січ`, `лют`, `бер`, …) — lower confidence than German/Dutch's
  well-established postal abbreviations, since Ukrainian abbreviation
  isn't formally standardized; worth revisiting if real-world OCR text
  shows a different common form. Because Ukrainian's genitive suffix
  only replaces the *end* of the nominative word, the first three
  letters are identical in both forms for all 12 months, so one
  abbreviated entry per month covers both without ambiguity.
- **English**: full names (already shipped) plus the now-added
  abbreviations `Jan/Feb/Mar/Apr/Jun/Jul/Aug/Sep(t)/Oct/Nov/Dec` (`May`
  already short).

Existing `\.?` optional-trailing-period handling in the regex pattern is
unchanged and covers abbreviations too (`Jan` and `Jan.` both match) —
no new regex feature needed, just more alternation branches.

**One real regex fix, found by the German test cases**: the day-first
pattern (`D Month YYYY`) had no allowance for a period *right after the
day* — but "15. März 2024" (day, period, month, year) is the standard
German way to write that date, not "15 März 2024". Added `\.?` right
after the day capture group in `find_month_name`'s day-first pattern;
this is a strict superset of what it accepted before (every date it used
to match, it still matches), so English/existing behavior is unaffected.

## 4. Explicitly Deferred
- **Ukrainian nominative-form matching** and **per-document language
  detection** — see backlog §3.
- **A 5th (or later) language** — the registry makes this additive, but
  none is being added in this round.

## 5. OpenTelemetry Implications
None. `date_extract::extract_issued_date` isn't instrumented (it's a
pure function called inline within `run_ocr`'s existing span, per TDR
012) and this change doesn't alter that.
