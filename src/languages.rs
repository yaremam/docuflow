//! ISO 639-1 language support for `documents.language` (feature 020) — replaces feature
//! 014's closed `en`/`cyr` script-bucket vocabulary with real language codes, validated
//! against the full ISO 639-1 set via the `isolang` crate, while OCR itself stays limited
//! to whichever trained-data packs `ocr::run_tesseract` actually has installed.

/// Languages DocuFlow can actually run OCR against — the Dockerfile installs exactly
/// these four `tesseract-ocr-*` trained-data packs (see `ocr::run_tesseract`), and
/// `language_detect::detect` never proposes a code outside this set. Every other ISO
/// 639-1 language is still a valid *manual* tag (see `other_options`), just without
/// OCR tuned for it yet.
pub const OCR_SUPPORTED: [&str; 4] = ["en", "de", "nl", "uk"];

/// One entry in the language `<select>` — always a real ISO 639-1 code paired with its
/// English display name.
pub struct LanguageOption {
    pub code: String,
    pub name: String,
}

/// The sole authority for whether a `documents.language` value is acceptable — both the
/// `Language` form newtype and the DB migration's CHECK constraint defer to this same
/// ISO 639-1 vocabulary rather than each maintaining their own copy of the list.
pub fn is_valid(code: &str) -> bool {
    isolang::Language::from_639_1(code).is_some()
}

/// Human-readable English name for a stored language code, e.g. `"de"` -> `"German"`.
/// Falls back to the raw code on a lookup miss — cheap display-only insurance for
/// pre-migration data, never hit for anything written after `is_valid` started gating
/// every write.
pub fn display_name(code: &str) -> String {
    isolang::Language::from_639_1(code)
        .map(|lang| lang.to_name().to_string())
        .unwrap_or_else(|| code.to_string())
}

/// The 4 OCR-supported languages, for the dropdown's first `<optgroup>`.
pub fn supported_options() -> Vec<LanguageOption> {
    OCR_SUPPORTED
        .iter()
        .map(|&code| LanguageOption {
            code: code.to_string(),
            name: display_name(code),
        })
        .collect()
}

/// Every other ISO 639-1 language, alphabetical by English name, for the dropdown's
/// second `<optgroup>` — manual tagging only, no OCR tuning behind these yet.
pub fn other_options() -> Vec<LanguageOption> {
    let mut opts: Vec<LanguageOption> = isolang::languages()
        .filter_map(|lang| lang.to_639_1().map(|code| (code, lang.to_name())))
        .filter(|(code, _)| !OCR_SUPPORTED.contains(code))
        .map(|(code, name)| LanguageOption {
            code: code.to_string(),
            name: name.to_string(),
        })
        .collect();
    opts.sort_by(|a, b| a.name.cmp(&b.name));
    opts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_codes_are_all_valid_iso_639_1() {
        for code in OCR_SUPPORTED {
            assert!(is_valid(code), "{code} should be a valid ISO 639-1 code");
        }
    }

    #[test]
    fn is_valid_rejects_the_old_script_bucket_and_garbage() {
        assert!(
            !is_valid("cyr"),
            "the retired feature-014 bucket must not validate as a real language"
        );
        assert!(!is_valid("xx"));
        assert!(!is_valid(""));
    }

    #[test]
    fn is_valid_accepts_a_language_outside_the_curated_ocr_set() {
        assert!(
            is_valid("fr"),
            "the full picker must accept languages DocuFlow can't OCR yet"
        );
    }

    #[test]
    fn display_name_matches_expected_english_names() {
        assert_eq!(display_name("de"), "German");
        assert_eq!(display_name("nl"), "Dutch");
        assert_eq!(display_name("uk"), "Ukrainian");
        assert_eq!(display_name("en"), "English");
    }

    #[test]
    fn other_options_excludes_the_curated_ocr_set_and_is_sorted() {
        let opts = other_options();
        assert!(opts
            .iter()
            .all(|opt| !OCR_SUPPORTED.contains(&opt.code.as_str())));
        assert!(
            opts.iter().any(|opt| opt.code == "fr"),
            "expected French among the manual-only options"
        );
        let mut sorted = opts.iter().map(|o| o.name.clone()).collect::<Vec<_>>();
        sorted.sort();
        assert_eq!(
            opts.iter().map(|o| o.name.clone()).collect::<Vec<_>>(),
            sorted
        );
    }
}
