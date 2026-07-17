//! Turns a Postgres `ts_headline` result into HTML safe to render
//! unescaped in a template. OCR'd text can contain literal `<`/`>`/`&`
//! (misreads or genuine content on the source document), so trusting
//! `ts_headline`'s output as ready-made HTML would be an XSS hole if its
//! `StartSel`/`StopSel` were set to literal `<mark>`/`</mark>` — this
//! module uses control-character markers instead, and only ever emits
//! `<mark>` tags it wraps around already-escaped text itself (feature
//! 027; see TDR 027 §2 Alternative C for why).

/// `ts_headline`'s `StartSel`/`StopSel` markers — control characters that
/// can't occur in real OCR'd text. Even in the vanishingly unlikely case
/// one does, the fallback is a missed/bogus mark, never unescaped markup.
const HEADLINE_START: char = '\u{1}';
const HEADLINE_STOP: char = '\u{2}';

/// `ts_headline` options for a short single-fragment excerpt around the
/// first match — the search-results snippet line.
pub const SNIPPET_OPTIONS: &str =
    "MaxFragments=1,MinWords=12,MaxWords=30,StartSel=\u{1},StopSel=\u{2}";

/// `ts_headline` options that mark every match across the whole input
/// with no truncation — the document detail page's full OCR text.
pub const FULL_TEXT_OPTIONS: &str = "HighlightAll=true,StartSel=\u{1},StopSel=\u{2}";

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Renders a `ts_headline` result (built with [`SNIPPET_OPTIONS`] or
/// [`FULL_TEXT_OPTIONS`]) as safe HTML: everything outside the
/// `HEADLINE_START`/`HEADLINE_STOP` markers is escaped as plain text, the
/// marked spans are escaped too and wrapped in `<mark>`.
pub fn render_marked(headline: &str) -> String {
    let mut html = String::with_capacity(headline.len() + 16);
    let mut in_mark = false;
    let mut segment = String::new();
    for ch in headline.chars() {
        match ch {
            HEADLINE_START if !in_mark => {
                html.push_str(&escape_html(&segment));
                segment.clear();
                in_mark = true;
            }
            HEADLINE_STOP if in_mark => {
                html.push_str("<mark>");
                html.push_str(&escape_html(&segment));
                html.push_str("</mark>");
                segment.clear();
                in_mark = false;
            }
            _ => segment.push(ch),
        }
    }
    // An unterminated mark (shouldn't happen from real `ts_headline`
    // output) degrades to plain escaped text rather than losing content.
    html.push_str(&escape_html(&segment));
    html
}

/// Whether a `ts_headline` result actually marked anything — used to
/// gate a "highlighting matches for ..." indicator so it's never shown
/// for a `q` that doesn't appear in this particular document.
pub fn has_match(headline: &str) -> bool {
    headline.contains(HEADLINE_START)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_a_single_marked_span_in_mark_tags() {
        let headline = format!("Springfield {HEADLINE_START}Electric{HEADLINE_STOP} statement");
        assert_eq!(
            render_marked(&headline),
            "Springfield <mark>Electric</mark> statement"
        );
    }

    #[test]
    fn wraps_multiple_separate_marked_spans() {
        let headline = format!("Springfield {HEADLINE_START}Electric{HEADLINE_STOP} {HEADLINE_START}Company{HEADLINE_STOP} annual");
        assert_eq!(
            render_marked(&headline),
            "Springfield <mark>Electric</mark> <mark>Company</mark> annual"
        );
    }

    #[test]
    fn plain_text_with_no_markers_is_only_escaped() {
        assert_eq!(
            render_marked("Acme Water Utility statement"),
            "Acme Water Utility statement"
        );
    }

    #[test]
    fn escapes_html_metacharacters_outside_marked_spans() {
        let headline = format!("Balance < 5 & due {HEADLINE_START}now{HEADLINE_STOP}");
        assert_eq!(
            render_marked(&headline),
            "Balance &lt; 5 &amp; due <mark>now</mark>"
        );
    }

    #[test]
    fn escapes_html_metacharacters_inside_marked_spans_too() {
        let headline = format!("{HEADLINE_START}A & B{HEADLINE_STOP}");
        assert_eq!(render_marked(&headline), "<mark>A &amp; B</mark>");
    }

    #[test]
    fn has_match_is_false_for_plain_text() {
        assert!(!has_match("Acme Water Utility statement"));
    }

    #[test]
    fn has_match_is_true_once_something_is_marked() {
        let headline = format!("Springfield {HEADLINE_START}Electric{HEADLINE_STOP}");
        assert!(has_match(&headline));
    }
}
