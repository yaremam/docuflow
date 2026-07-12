//! Scans a document's OCR text for a single best-guess "issued" date, per
//! TDR 012. This is a narrow, fixed-shape scanner — not a general
//! natural-language date parser (see TDR 012 §2 Alternative B) — because
//! OCR'd bill/contract text is noisy, fixed-format machine/print text, and
//! a wider grammar would make false positives on incidental numbers
//! (account numbers, amounts) harder to rule out, not easier.
//!
//! Every candidate is validated through `time::Date::from_calendar_date`
//! (the same constructor `web::forms::DateIssuedField` already uses for
//! manually-typed dates) and a sane year range, so a garbled OCR digit
//! never panics and never produces an out-of-range suggestion — it's
//! simply treated as "no date found here."

use std::sync::OnceLock;

use time::{Date, Month, OffsetDateTime};

const MONTH_NAMES: &[(&str, Month)] = &[
    ("january", Month::January),
    ("february", Month::February),
    ("march", Month::March),
    ("april", Month::April),
    ("may", Month::May),
    ("june", Month::June),
    ("july", Month::July),
    ("august", Month::August),
    ("september", Month::September),
    ("october", Month::October),
    ("november", Month::November),
    ("december", Month::December),
];

fn sane_year(year: i32) -> bool {
    let current_year = OffsetDateTime::now_utc().year();
    (1900..=current_year + 1).contains(&year)
}

fn valid_date(year: i32, month: u8, day: u8) -> Option<Date> {
    if !sane_year(year) {
        return None;
    }
    let month = Month::try_from(month).ok()?;
    Date::from_calendar_date(year, month, day).ok()
}

/// Builds `regex` once per pattern and caches it for the life of the
/// process — cheap either way since this only ever runs once per document
/// in the background OCR worker (never a per-request hot path), but free
/// to avoid. Returns `None` (never panics) in the unreachable case a
/// hardcoded pattern fails to compile.
fn cached_regex(
    cache: &'static OnceLock<Option<regex::Regex>>,
    pattern: &str,
) -> Option<&'static regex::Regex> {
    cache
        .get_or_init(|| regex::Regex::new(pattern).ok())
        .as_ref()
}

/// `YYYY-MM-DD`, e.g. `2024-03-15`.
fn find_iso(text: &str) -> Option<Date> {
    static RE: OnceLock<Option<regex::Regex>> = OnceLock::new();
    let caps = cached_regex(&RE, r"\b(\d{4})-(\d{2})-(\d{2})\b")?.captures(text)?;
    let year: i32 = caps[1].parse().ok()?;
    let month: u8 = caps[2].parse().ok()?;
    let day: u8 = caps[3].parse().ok()?;
    valid_date(year, month, day)
}

fn month_name_alternation() -> &'static str {
    static PATTERN: OnceLock<String> = OnceLock::new();
    PATTERN.get_or_init(|| {
        MONTH_NAMES
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join("|")
    })
}

/// `Month D[,] YYYY` or `D Month YYYY`, e.g. `March 15, 2024` / `15 March
/// 2024` — tried as two shapes of the same underlying pattern (which
/// capture group is the day vs. the month name changes; everything else
/// about validating a match is identical).
fn find_month_name(text: &str) -> Option<Date> {
    static MONTH_FIRST: OnceLock<Option<regex::Regex>> = OnceLock::new();
    static DAY_FIRST: OnceLock<Option<regex::Regex>> = OnceLock::new();

    let month_first = cached_regex(
        &MONTH_FIRST,
        &format!(
            r"(?i)\b({})\.?\s+(\d{{1,2}}),?\s+(\d{{4}})\b",
            month_name_alternation()
        ),
    );
    let day_first = cached_regex(
        &DAY_FIRST,
        &format!(
            r"(?i)\b(\d{{1,2}})\s+({})\.?,?\s+(\d{{4}})\b",
            month_name_alternation()
        ),
    );

    for (regex, day_comes_first) in [(month_first, false), (day_first, true)] {
        let Some(caps) = regex.and_then(|re| re.captures(text)) else {
            continue;
        };
        let (day, month, year) = if day_comes_first {
            (
                caps[1].parse::<u8>().ok()?,
                lookup_month(&caps[2])?,
                caps[3].parse::<i32>().ok()?,
            )
        } else {
            (
                caps[2].parse::<u8>().ok()?,
                lookup_month(&caps[1])?,
                caps[3].parse::<i32>().ok()?,
            )
        };
        if let Some(date) = valid_date(year, month as u8, day) {
            return Some(date);
        }
    }

    None
}

fn lookup_month(name: &str) -> Option<Month> {
    let lower = name.to_lowercase();
    MONTH_NAMES
        .iter()
        .find(|(candidate, _)| *candidate == lower)
        .map(|(_, month)| *month)
}

/// `M/D/YYYY` or `M-D-YYYY`. Ambiguous when both numbers are `<= 12`
/// (assumed US `MM/DD/YYYY`); unambiguous whenever one of the two is `>
/// 12` (that one must be the day) — see TDR 012 §3.
fn find_numeric(text: &str) -> Option<Date> {
    static RE: OnceLock<Option<regex::Regex>> = OnceLock::new();
    let caps = cached_regex(&RE, r"\b(\d{1,2})[/-](\d{1,2})[/-](\d{4})\b")?.captures(text)?;
    let first: u8 = caps[1].parse().ok()?;
    let second: u8 = caps[2].parse().ok()?;
    let year: i32 = caps[3].parse().ok()?;

    // If `first` is unambiguously the day (>12), it must be the other way
    // around; otherwise (including the ambiguous both-<=12 case) assume US
    // MM/DD order — see the doc comment above.
    let (month, day) = if first > 12 {
        (second, first)
    } else {
        (first, second)
    };

    valid_date(year, month, day)
}

/// Scans `text` for a single best-guess issued date, trying each shape in
/// priority order (most unambiguous first) and returning the first valid
/// match. Returns `None` if nothing recognizable is found — never panics.
pub fn extract_issued_date(text: &str) -> Option<Date> {
    find_iso(text)
        .or_else(|| find_month_name(text))
        .or_else(|| find_numeric(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_iso_date() {
        let date = extract_issued_date("Invoice date: 2024-03-15 total due").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2024, Month::March, 15).unwrap()
        );
    }

    #[test]
    fn finds_month_name_then_day() {
        let date = extract_issued_date("Statement Date: March 15, 2024").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2024, Month::March, 15).unwrap()
        );
    }

    #[test]
    fn finds_day_then_month_name() {
        let date = extract_issued_date("Issued 15 March 2024 for service").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2024, Month::March, 15).unwrap()
        );
    }

    #[test]
    fn finds_numeric_slash_date_with_unambiguous_day() {
        // second number (15) is > 12, so it must be the day regardless of
        // MM/DD-vs-DD/MM assumptions.
        let date = extract_issued_date("Due 03/15/2024").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2024, Month::March, 15).unwrap()
        );
    }

    #[test]
    fn finds_numeric_dash_date() {
        let date = extract_issued_date("Due 03-15-2024").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2024, Month::March, 15).unwrap()
        );
    }

    #[test]
    fn ambiguous_numeric_date_assumes_us_month_day_order() {
        let date = extract_issued_date("Ref 03/04/2024").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2024, Month::March, 4).unwrap()
        );
    }

    #[test]
    fn iso_takes_priority_over_a_later_numeric_match() {
        let date = extract_issued_date("Ref 03/04/2024 iso 2024-03-15").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2024, Month::March, 15).unwrap()
        );
    }

    #[test]
    fn rejects_a_year_before_1900() {
        assert!(extract_issued_date("Founded 1850-01-01").is_none());
    }

    #[test]
    fn rejects_a_year_far_in_the_future() {
        let far_future = OffsetDateTime::now_utc().year() + 10;
        assert!(extract_issued_date(&format!("Due {far_future}-01-01")).is_none());
    }

    #[test]
    fn rejects_an_invalid_calendar_date() {
        // February 30th doesn't exist.
        assert!(extract_issued_date("Due 2024-02-30").is_none());
    }

    #[test]
    fn returns_none_when_no_date_is_present() {
        assert!(extract_issued_date("Hello world, no date here").is_none());
    }
}
