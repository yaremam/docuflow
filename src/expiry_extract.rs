//! Scans a document's OCR text for a single best-guess "expires on" date,
//! per TDR 031. Unlike `date_extract::extract_issued_date` (which accepts
//! any date-shaped text with no keyword requirement — reasonable since a
//! document usually only prints one date, the one it was issued), an
//! expiry date needs a nearby trigger phrase before a candidate is
//! accepted: most documents that have both an issued date and an expiry
//! date print the issued one first, so a keyword-free scan would just
//! re-find that same date, or arbitrarily grab whichever date-shaped text
//! comes next — neither means "this is when it expires."
//!
//! Reuses `date_extract`'s `find_iso`/`find_month_name`/`find_numeric` as
//! the "what does a date look like" building block, so the two extraction
//! modules can't disagree about what counts as a valid date.

use std::sync::OnceLock;

use time::Date;

use crate::date_extract::{cached_regex, find_iso, find_month_name, find_numeric};

/// Trigger phrases sourced against a bilingual dictionary/usage reference
/// per language (not guessed — see TDR 031 §3 for citations), covering
/// the same 4 languages tesseract OCRs (`eng+deu+nld+ukr`).
const TRIGGER_PHRASES: &[&str] = &[
    // English
    "expires",
    "expiry date",
    "expiration date",
    "valid until",
    // German — "gültig bis" (valid until), "ablaufdatum" (expiration date)
    "gültig bis",
    "ablaufdatum",
    // Dutch — "geldig tot" (valid until), "vervaldatum" (expiration date)
    "geldig tot",
    "vervaldatum",
    // Ukrainian — "дійсний до" (valid until), "термін дії" (term of validity)
    "дійсний до",
    "термін дії",
];

/// How many characters after a trigger phrase to search for a date —
/// generous enough to cover a label separator ("Ablaufdatum: ...") or a
/// short filler word between the phrase and the date itself ("термін дії
/// до 15 січня 2026 року" — "до" sits between the phrase and the date).
const WINDOW_CHARS: usize = 30;

fn trigger_alternation() -> &'static str {
    static PATTERN: OnceLock<String> = OnceLock::new();
    PATTERN.get_or_init(|| TRIGGER_PHRASES.join("|"))
}

/// Scans `text` for a single best-guess expiry date: finds each
/// occurrence of a trigger phrase (case-insensitive) and tries the
/// existing date-shape recognizers, in the same ISO/month-name/numeric
/// priority order `extract_issued_date` uses, against a bounded window of
/// text right after it. Returns the first valid date found near *any*
/// trigger occurrence; `None` if nothing recognizable follows any of
/// them — never panics.
pub fn extract_expiry_date(text: &str) -> Option<Date> {
    static TRIGGER_RE: OnceLock<Option<regex::Regex>> = OnceLock::new();
    let pattern = format!(r"(?i)(?:{})", trigger_alternation());
    let re = cached_regex(&TRIGGER_RE, &pattern)?;

    for m in re.find_iter(text) {
        let window: String = text[m.end()..].chars().take(WINDOW_CHARS).collect();
        if let Some(date) = find_iso(&window)
            .or_else(|| find_month_name(&window))
            .or_else(|| find_numeric(&window))
        {
            return Some(date);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Month;

    #[test]
    fn finds_a_date_after_an_english_trigger_phrase() {
        let date =
            extract_expiry_date("Policy details. Expires: 2026-07-31 unless renewed.").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2026, Month::July, 31).unwrap()
        );
    }

    #[test]
    fn finds_a_date_after_valid_until() {
        let date = extract_expiry_date("This card is valid until 15 March 2026.").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2026, Month::March, 15).unwrap()
        );
    }

    #[test]
    fn finds_a_date_after_german_gueltig_bis() {
        let date = extract_expiry_date("Gültig bis 15. März 2026").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2026, Month::March, 15).unwrap()
        );
    }

    #[test]
    fn finds_a_date_after_german_ablaufdatum() {
        let date = extract_expiry_date("Ablaufdatum: 2026-07-31").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2026, Month::July, 31).unwrap()
        );
    }

    #[test]
    fn finds_a_date_after_dutch_geldig_tot() {
        let date = extract_expiry_date("Geldig tot 15 maart 2026").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2026, Month::March, 15).unwrap()
        );
    }

    #[test]
    fn finds_a_date_after_dutch_vervaldatum() {
        let date = extract_expiry_date("Vervaldatum: 2026-07-31").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2026, Month::July, 31).unwrap()
        );
    }

    #[test]
    fn finds_a_date_after_ukrainian_diisnyi_do() {
        let date = extract_expiry_date("Дійсний до 15 січня 2026 року").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2026, Month::January, 15).unwrap()
        );
    }

    #[test]
    fn finds_a_date_after_ukrainian_termin_dii() {
        let date = extract_expiry_date("Термін дії до 15 січня 2026 року").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2026, Month::January, 15).unwrap()
        );
    }

    #[test]
    fn returns_none_with_no_trigger_phrase_present() {
        // A bare date with no expiry-indicating keyword shouldn't be
        // claimed as an expiry date — that's exactly the ambiguity this
        // module exists to avoid (TDR 031 §2 Alternative C).
        assert!(extract_expiry_date("Statement date: 2026-07-31").is_none());
    }

    #[test]
    fn returns_none_when_no_date_follows_the_trigger_phrase() {
        assert!(extract_expiry_date("This policy expires at the end of the term.").is_none());
    }

    #[test]
    fn skips_a_trigger_phrase_with_no_nearby_date_and_finds_a_later_one() {
        // First "expires" mention has no date within the window; a later
        // occurrence does — the scan keeps looking rather than giving up
        // after the first trigger match.
        let text =
            "Terms: coverage expires as described below. See details. Expiry date: 2026-07-31.";
        let date = extract_expiry_date(text).unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2026, Month::July, 31).unwrap()
        );
    }

    #[test]
    fn does_not_match_an_issued_date_alone() {
        // Distinguishes this module's keyword-anchored behavior from
        // extract_issued_date's no-keyword scan (AC-3).
        let text = "Invoice date: 2026-01-14. Total due: $142.18.";
        assert!(extract_expiry_date(text).is_none());
        assert_eq!(
            crate::date_extract::extract_issued_date(text),
            Some(Date::from_calendar_date(2026, Month::January, 14).unwrap())
        );
    }
}
