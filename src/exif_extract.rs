//! Reads a photographed document's EXIF capture date as a fallback
//! issued-date suggestion, per TDR 019 — used only when
//! `date_extract::extract_issued_date` found nothing in the OCR text, since
//! a photo's capture date is a weaker signal than a date actually printed
//! on the document (TDR 019 §1).
//!
//! Every candidate is validated through `time::Date::from_calendar_date`,
//! so a missing tag, unsupported container, or malformed EXIF payload never
//! panics — it's simply treated as "no date found here," matching
//! `date_extract`'s existing contract.

use std::io::Cursor;

use time::{Date, Month};

fn month_from_number(month: u8) -> Option<Month> {
    Month::try_from(month).ok()
}

/// Parses the EXIF ASCII date format (`"YYYY:MM:DD HH:MM:SS"` — colons in
/// the date portion, not dashes, unlike `date_extract`'s ISO parsing).
fn parse_exif_datetime(raw: &str) -> Option<Date> {
    let date_part = raw.trim().split(' ').next()?;
    let mut segments = date_part.split(':');
    let year: i32 = segments.next()?.parse().ok()?;
    let month: u8 = segments.next()?.parse().ok()?;
    let day: u8 = segments.next()?.parse().ok()?;
    Date::from_calendar_date(year, month_from_number(month)?, day).ok()
}

fn ascii_field_value(field: &exif::Field) -> Option<String> {
    match &field.value {
        exif::Value::Ascii(rows) => {
            let first = rows.first()?;
            Some(String::from_utf8_lossy(first).trim().to_string())
        }
        _ => None,
    }
}

/// Returns the document's EXIF capture date (`DateTimeOriginal`, falling
/// back to `DateTime`) if present and parseable — `None` for anything else
/// (no EXIF data, an unsupported container, or a garbled value), never a
/// panic or an error.
pub fn extract_issued_date(bytes: &[u8]) -> Option<Date> {
    let mut cursor = Cursor::new(bytes);
    let exif_data = exif::Reader::new().read_from_container(&mut cursor).ok()?;

    let field = exif_data
        .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
        .or_else(|| exif_data.get_field(exif::Tag::DateTime, exif::In::PRIMARY))?;

    parse_exif_datetime(&ascii_field_value(field)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_exif_ascii_datetime_format() {
        let date = parse_exif_datetime("2026:03:14 09:30:00").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2026, Month::March, 14).unwrap()
        );
    }

    #[test]
    fn ignores_a_leading_space_and_anything_after_the_date_and_time() {
        let date = parse_exif_datetime(" 2026:03:14 09:30:00 \0").unwrap();
        assert_eq!(
            date,
            Date::from_calendar_date(2026, Month::March, 14).unwrap()
        );
    }

    #[test]
    fn rejects_a_dash_separated_date_this_is_not_the_exif_format() {
        assert_eq!(parse_exif_datetime("2026-03-14 09:30:00"), None);
    }

    #[test]
    fn rejects_an_out_of_range_month() {
        assert_eq!(parse_exif_datetime("2026:13:14 09:30:00"), None);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_exif_datetime("not a date"), None);
        assert_eq!(parse_exif_datetime(""), None);
    }

    #[test]
    fn extract_issued_date_returns_none_for_non_exif_bytes() {
        assert_eq!(extract_issued_date(b"not an image at all"), None);
    }
}
