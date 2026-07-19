//! Scans a document's OCR text for a single best-guess payment amount, for
//! `bill`/`receipt` documents only (see `doc_type_extract::DocType`) — the
//! two doc types where "this document represents one purchase/payment" is
//! unambiguous, unlike an `insurance`/`contract` premium, which is a
//! recurring cost rather than a single spend.
//!
//! Unlike `date_extract::extract_issued_date` (keyword-free — a document
//! usually only prints one date) or `expiry_extract::extract_expiry_date`
//! (one flat trigger-phrase list, first match wins), this needs a *tiered*
//! trigger search: a bill routinely shows several numbers that look
//! amount-shaped — subtotal, tax, previous balance, total due — and
//! "subtotal" almost always appears *before* "total" in document order, so
//! a flat first-occurrence scan would systematically grab the wrong one.
//! Every strong trigger phrase ("total due", "amount due", the bare
//! "total", and their per-language equivalents) is searched across the
//! *entire* document first; only if none of those match anywhere does a
//! second pass try the weaker phrases ("subtotal" and friends).
//!
//! Number formatting is also language-aware, unlike `date_extract`'s
//! single combined pass: English writes `1,234.56` (comma thousands,
//! period decimal); German/Dutch/Ukrainian conventionally write `1.234,56`
//! (period thousands, comma decimal) — the opposite convention. The
//! document's own detected `language` column (already populated by the
//! time OCR extraction runs) picks which convention to try first; the
//! other is tried as a fallback, the same "try the likely shape, then the
//! other shape" pattern `date_extract::find_month_name` already uses for
//! day-first vs. month-first ordering.
//!
//! Amounts are returned as integer cents (`i64`), never a float — money
//! has no business accumulating floating-point rounding error, and cents
//! avoids pulling in a decimal crate for a single two-decimal-place value.

use std::sync::OnceLock;

use crate::date_extract::cached_regex;

const WINDOW_CHARS: usize = 30;

/// Unambiguous "this is the final amount" phrases. Searched across the
/// whole document before any weak-tier phrase is tried at all.
const STRONG_TRIGGER_PHRASES: &[&str] = &[
    // English
    "total due",
    "amount due",
    "total",
    // German — "Gesamtbetrag"/"Rechnungsbetrag" (total/invoice amount)
    "gesamtbetrag",
    "rechnungsbetrag",
    // Dutch — "totaalbedrag" (total amount), "te betalen" (to be paid)
    "totaalbedrag",
    "te betalen",
    // Ukrainian — "до сплати" (due/to be paid), "загальна сума" (total sum)
    "до сплати",
    "загальна сума",
];

/// Ambiguous phrases that can precede a subtotal or a generic figure, not
/// necessarily the final amount — only tried if no strong-tier phrase
/// matched anywhere in the document.
const WEAK_TRIGGER_PHRASES: &[&str] = &[
    // English
    "subtotal",
    "amount",
    // German — "Zwischensumme" (subtotal), "Betrag" (generic amount)
    "zwischensumme",
    "betrag",
    // Dutch — "subtotaal" (subtotal), "bedrag" (generic amount)
    "subtotaal",
    "bedrag",
    // Ukrainian — "проміжна сума" (subtotal), "сума" (generic sum)
    "проміжна сума",
    "сума",
];

/// Languages that conventionally write `1.234,56` (period thousands, comma
/// decimal) rather than English's `1,234.56` — see `languages::OCR_SUPPORTED`.
const EU_FORMAT_LANGUAGES: &[&str] = &["de", "nl", "uk"];

fn strong_trigger_regex() -> Option<&'static regex::Regex> {
    static PATTERN: OnceLock<String> = OnceLock::new();
    static RE: OnceLock<Option<regex::Regex>> = OnceLock::new();
    // `\b`-anchored: unanchored, bare "total" would also match inside
    // "Subtotal", defeating the entire point of the strong/weak tier split.
    let pattern =
        PATTERN.get_or_init(|| format!(r"(?i)\b(?:{})\b", STRONG_TRIGGER_PHRASES.join("|")));
    cached_regex(&RE, pattern)
}

fn weak_trigger_regex() -> Option<&'static regex::Regex> {
    static PATTERN: OnceLock<String> = OnceLock::new();
    static RE: OnceLock<Option<regex::Regex>> = OnceLock::new();
    let pattern =
        PATTERN.get_or_init(|| format!(r"(?i)\b(?:{})\b", WEAK_TRIGGER_PHRASES.join("|")));
    cached_regex(&RE, pattern)
}

fn find_amount_after_triggers(
    text: &str,
    trigger_re: &regex::Regex,
    language: Option<&str>,
) -> Option<i64> {
    for m in trigger_re.find_iter(text) {
        let window: String = text[m.end()..].chars().take(WINDOW_CHARS).collect();
        if let Some(cents) = find_amount_in_window(&window, language) {
            return Some(cents);
        }
    }
    None
}

fn find_amount_in_window(window: &str, language: Option<&str>) -> Option<i64> {
    let prefer_eu = language.is_some_and(|lang| EU_FORMAT_LANGUAGES.contains(&lang));
    if prefer_eu {
        find_amount_eu_format(window).or_else(|| find_amount_en_format(window))
    } else {
        find_amount_en_format(window).or_else(|| find_amount_eu_format(window))
    }
}

/// `1,234.56` — comma thousands separator, period decimal. Both parts
/// optional beyond the leading digits: `45`, `45.5`, and `1,234` all match.
fn find_amount_en_format(text: &str) -> Option<i64> {
    static RE: OnceLock<Option<regex::Regex>> = OnceLock::new();
    let caps = cached_regex(&RE, r"\b(\d{1,3}(?:,\d{3})*)(?:\.(\d{1,2}))?\b")?.captures(text)?;
    to_cents(&caps[1].replace(',', ""), caps.get(2).map(|m| m.as_str()))
}

/// `1.234,56` — period thousands separator, comma decimal (German/Dutch/
/// Ukrainian convention).
fn find_amount_eu_format(text: &str) -> Option<i64> {
    static RE: OnceLock<Option<regex::Regex>> = OnceLock::new();
    let caps = cached_regex(&RE, r"\b(\d{1,3}(?:\.\d{3})*)(?:,(\d{1,2}))?\b")?.captures(text)?;
    to_cents(&caps[1].replace('.', ""), caps.get(2).map(|m| m.as_str()))
}

/// Combines a (separator-stripped) integer part with an optional 1-or-2
/// digit decimal part into integer cents. A lone decimal digit ("45.5")
/// means 50 cents, not 5 — right-pads to two digits before parsing, never
/// panics on a garbled OCR digit.
fn to_cents(integer_part: &str, decimal_part: Option<&str>) -> Option<i64> {
    let whole: i64 = integer_part.parse().ok()?;
    let cents: i64 = match decimal_part {
        Some(d) if d.len() == 1 => format!("{d}0").parse().ok()?,
        Some(d) => d.parse().ok()?,
        None => 0,
    };
    let total = whole.checked_mul(100)?.checked_add(cents)?;
    (total > 0).then_some(total)
}

/// Scans `text` for a single best-guess payment amount, in integer cents.
/// `language` should be the document's detected `language` column (e.g.
/// `"en"`, `"de"`) — `None` (not yet detected) falls back to trying the
/// English number format first. Returns `None` if no trigger phrase has a
/// recognizable amount nearby — never panics.
pub fn extract_amount(text: &str, language: Option<&str>) -> Option<i64> {
    let strong =
        strong_trigger_regex().and_then(|re| find_amount_after_triggers(text, re, language));
    strong.or_else(|| {
        weak_trigger_regex().and_then(|re| find_amount_after_triggers(text, re, language))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_amount_after_total_due() {
        assert_eq!(extract_amount("Total Due: $45.00", Some("en")), Some(4500));
    }

    #[test]
    fn finds_amount_after_bare_total() {
        assert_eq!(extract_amount("Total 128.50", Some("en")), Some(12850));
    }

    #[test]
    fn strong_trigger_wins_over_an_earlier_subtotal() {
        // "Subtotal" appears first in document order, but the strong tier
        // ("Total due") is tried across the whole document before the weak
        // tier is tried at all — the subtotal must never win.
        let text = "Subtotal: 40.00\nTax: 5.00\nTotal due: 45.00";
        assert_eq!(extract_amount(text, Some("en")), Some(4500));
    }

    #[test]
    fn falls_back_to_subtotal_when_no_strong_trigger_matches() {
        assert_eq!(extract_amount("Subtotal: 40.00", Some("en")), Some(4000));
    }

    #[test]
    fn german_eu_format_amount_is_parsed_correctly() {
        // 1.234,56 EU-format — period thousands, comma decimal — must not
        // be misread as "1.234" dollars and "56" cents.
        assert_eq!(
            extract_amount("Gesamtbetrag: 1.234,56", Some("de")),
            Some(123456)
        );
    }

    #[test]
    fn dutch_te_betalen_trigger_is_recognized() {
        assert_eq!(extract_amount("Te betalen: 89,90", Some("nl")), Some(8990));
    }

    #[test]
    fn ukrainian_do_splaty_trigger_is_recognized() {
        assert_eq!(extract_amount("До сплати: 250,00", Some("uk")), Some(25000));
    }

    #[test]
    fn english_document_prefers_english_number_format() {
        // 1,234.56 read as English-first: thousands comma, decimal period.
        assert_eq!(extract_amount("Total: 1,234.56", Some("en")), Some(123456));
    }

    #[test]
    fn unknown_language_falls_back_to_english_number_format() {
        assert_eq!(extract_amount("Total: 45.00", None), Some(4500));
    }

    #[test]
    fn a_lone_decimal_digit_is_right_padded_not_misread() {
        // "45.5" is 45 dollars 50 cents, not 45 dollars 5 cents.
        assert_eq!(extract_amount("Total 45.5", Some("en")), Some(4550));
    }

    #[test]
    fn returns_none_with_no_trigger_phrase_present() {
        assert!(extract_amount("Thank you for your business", Some("en")).is_none());
    }

    #[test]
    fn returns_none_when_no_number_follows_the_trigger_phrase() {
        assert!(extract_amount("Total due: please contact billing", Some("en")).is_none());
    }

    #[test]
    fn a_zero_amount_is_not_a_valid_suggestion() {
        assert!(extract_amount("Total due: 0.00", Some("en")).is_none());
    }

    #[test]
    fn skips_a_trigger_phrase_with_no_nearby_amount_and_finds_a_later_one() {
        let text = "Total due: see attached statement. Amount due: 60.00";
        assert_eq!(extract_amount(text, Some("en")), Some(6000));
    }
}
