//! Post-OCR language auto-detection (feature 011, generalized in feature 020) — runs
//! `whatlang` against the extracted text and proposes a `documents.language` value.
//!
//! Deliberately restricted to the 4 languages `ocr::run_tesseract` actually has
//! trained-data packs for (`languages::OCR_SUPPORTED`): detection only ever reflects
//! how well OCR read the page, never a guess at a language OCR wasn't tuned for. Any
//! other ISO 639-1 language is still a valid *manual* tag (see `languages::other_options`),
//! just never auto-proposed.

use whatlang::Lang;

pub fn detect(text: &str) -> Option<&'static str> {
    let info = whatlang::detect(text)?;
    if !info.is_reliable() {
        return None;
    }
    match info.lang() {
        Lang::Eng => Some("en"),
        Lang::Deu => Some("de"),
        Lang::Nld => Some("nl"),
        Lang::Ukr => Some("uk"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_english() {
        let text = "The quick brown fox jumps over the lazy dog near the riverbank every morning.";
        assert_eq!(detect(text), Some("en"));
    }

    #[test]
    fn detects_german() {
        let text = "Der Bescheid über die Steuererklärung wurde heute per Post zugestellt und muss geprüft werden.";
        assert_eq!(detect(text), Some("de"));
    }

    #[test]
    fn detects_dutch() {
        let text = "De jaarlijkse belastingaangifte moet voor het einde van de maand worden ingediend bij de belastingdienst.";
        assert_eq!(detect(text), Some("nl"));
    }

    #[test]
    fn detects_ukrainian() {
        let text = "Цей документ підтверджує оплату рахунку за електроенергію для квартири на вулиці Шевченка.";
        assert_eq!(detect(text), Some("uk"));
    }

    #[test]
    fn does_not_detect_a_language_outside_the_curated_ocr_set() {
        // Genuinely French text — whatlang can identify it, but since there's no French
        // OCR pack, detect() must never propose "fr" (it'd misrepresent what OCR read).
        let text = "Le document que vous avez reçu concerne votre déclaration de revenus pour l'année précédente.";
        assert_eq!(detect(text), None);
    }

    #[test]
    fn returns_none_for_unclear_text() {
        assert_eq!(detect("a b c 123"), None);
    }
}
