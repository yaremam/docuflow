//! Detects a document's language from its OCR-extracted text (see TDR 014),
//! scoped to exactly the two buckets `tesseract -l eng+rus` (TDR 011)
//! actually produces: English, and Cyrillic script generally (not a
//! specific language within it — the shared `rus` trained-data pack OCRs
//! any Cyrillic-script document through the same model, so detection here
//! matches that at the script level rather than pretending to distinguish
//! specific languages the OCR pipeline itself doesn't).

use whatlang::{Lang, Script};

/// Returns `"en"`/`"cyr"` only when the extracted text confidently matches
/// one of the two supported buckets — anything else, or an unreliable
/// result, returns `None`, never a guess (see TDR 014 §3).
pub fn detect(text: &str) -> Option<&'static str> {
    // Script identification doesn't need a confidence gate the way full
    // language identification does below: a character either falls in the
    // Cyrillic Unicode ranges or it doesn't, with no ambiguity between
    // similarly-scored candidate languages to weigh.
    if whatlang::detect_script(text) == Some(Script::Cyrillic) {
        return Some("cyr");
    }

    let info = whatlang::detect(text)?;
    if info.is_reliable() && info.lang() == Lang::Eng {
        Some("en")
    } else {
        None
    }
}
