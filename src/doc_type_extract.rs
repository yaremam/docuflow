//! Scans a document's OCR text for a single best-guess document type, per
//! TDR 024. Like `date_extract`, this is a narrow keyword scanner, not a
//! general classifier — OCR'd bill/contract/insurance/receipt/ID text each
//! carries a handful of near-universal, distinctive phrases (a passport's
//! "identification", a policy's "coverage"), so a small fixed keyword list
//! per category is enough signal without needing any ML model or training
//! data.
//!
//! Categories are checked in most-distinctive-first order: `Bill`'s
//! keywords ("invoice", "amount due") are the most generic and would
//! false-positive on other categories that also mention a balance (an
//! insurance premium, a receipt subtotal), so it's checked last — a
//! document only falls through to `Bill` once nothing more specific
//! matched. `Other` has no keyword list at all: it's a manual-only
//! dropdown option for documents the ruleset doesn't recognize, never a
//! suggestion (see TDR 024 §2).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocType {
    Id,
    Insurance,
    Contract,
    Receipt,
    Bill,
}

impl DocType {
    pub fn as_str(self) -> &'static str {
        match self {
            DocType::Id => "id",
            DocType::Insurance => "insurance",
            DocType::Contract => "contract",
            DocType::Receipt => "receipt",
            DocType::Bill => "bill",
        }
    }
}

pub struct DocTypeOption {
    pub value: &'static str,
    pub label: &'static str,
}

/// The confirmed `doc_type` field's `<select>` options — a small fixed
/// taxonomy (see TDR 024 §2). `"other"` has no `DocType` variant and is
/// never suggested; it exists only as a manual catch-all for documents
/// the keyword ruleset doesn't recognize. A `const` slice rather than a
/// `Vec`-returning function: every field is already `&'static str`, so
/// there's no allocation to do, and `dropdown_options`/`label_for`/
/// `is_valid` below can all borrow it directly instead of each rebuilding
/// their own `Vec` per call.
const DOC_TYPE_OPTIONS: &[DocTypeOption] = &[
    DocTypeOption {
        value: "bill",
        label: "Bill",
    },
    DocTypeOption {
        value: "contract",
        label: "Contract",
    },
    DocTypeOption {
        value: "insurance",
        label: "Insurance",
    },
    DocTypeOption {
        value: "receipt",
        label: "Receipt",
    },
    DocTypeOption {
        value: "id",
        label: "ID",
    },
    DocTypeOption {
        value: "other",
        label: "Other",
    },
];

pub fn dropdown_options() -> &'static [DocTypeOption] {
    DOC_TYPE_OPTIONS
}

/// Resolves a stored/facet `doc_type` value (e.g. `"bill"`) to its
/// human-facing label (`"Bill"`) — the one place every list row, facet
/// option, applied-filter chip, and suggestion box looks this up, instead
/// of each repeating the same `dropdown_options().iter().find(...)` scan.
pub fn label_for(value: &str) -> Option<&'static str> {
    DOC_TYPE_OPTIONS
        .iter()
        .find(|opt| opt.value == value)
        .map(|opt| opt.label)
}

/// Whether `value` is one of `dropdown_options()`'s values — the sole
/// authority for `DocTypeField`'s validation (`src/web/forms.rs`), mirroring
/// how `languages::is_valid` backs `Language`.
pub fn is_valid(value: &str) -> bool {
    DOC_TYPE_OPTIONS.iter().any(|opt| opt.value == value)
}

/// The `doc_type` values that structurally have an expiry date at all —
/// insurance policies and contracts/subscriptions renew or lapse,
/// receipts/other essentially never carry one; `bill` and `id` are
/// judgment calls included by explicit direction (feature 031, TDR 031
/// §3). A `&'static str` slice (not a `Vec`) so it can double as both a
/// Rust-side predicate source and, converted once at the SQL call site,
/// a Postgres `text[]` bind parameter for the `expiry_status` facet's
/// "No expiry set" bucket (`src/web/handlers/documents.rs`).
pub const EXPIRY_ELIGIBLE_DOC_TYPES: &[&str] = &["insurance", "contract", "bill", "id"];

/// Whether a *confirmed* `doc_type` is one of `EXPIRY_ELIGIBLE_DOC_TYPES`.
/// Deliberately takes the confirmed `doc_type` column's value, never
/// `ocr_suggested_doc_type` — every call site should gate on a confirmed
/// fact, not an unaccepted guess (TDR 031 §3, AC-1).
pub fn is_expiry_eligible(value: &str) -> bool {
    EXPIRY_ELIGIBLE_DOC_TYPES.contains(&value)
}

const KEYWORDS: &[(DocType, &[&str])] = &[
    (
        DocType::Id,
        &[
            "passport",
            "driver license",
            "driver's license",
            "identification number",
            "date of birth",
        ],
    ),
    (
        DocType::Insurance,
        &[
            "insurance policy",
            "policy number",
            "coverage",
            "premium",
            "insured",
        ],
    ),
    (
        DocType::Contract,
        &[
            "agreement",
            "contract",
            "terms and conditions",
            "parties hereto",
        ],
    ),
    (
        DocType::Receipt,
        &["receipt", "thank you for your purchase", "subtotal"],
    ),
    (
        DocType::Bill,
        &["invoice", "amount due", "statement date", "account number"],
    ),
];

/// Scans `text` for a single best-guess document type, trying each
/// category in most-distinctive-first order and returning the first
/// keyword match. Returns `None` if nothing recognizable is found — never
/// panics, and never guesses `Other` (see the module doc comment).
pub fn extract_doc_type(text: &str) -> Option<DocType> {
    let lower = text.to_lowercase();
    KEYWORDS
        .iter()
        .find(|(_, keywords)| keywords.iter().any(|keyword| lower.contains(keyword)))
        .map(|(doc_type, _)| *doc_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_bill_from_invoice_language() {
        assert_eq!(
            extract_doc_type("INVOICE\nAmount due: $118.41"),
            Some(DocType::Bill)
        );
    }

    #[test]
    fn finds_insurance_from_policy_language() {
        assert_eq!(
            extract_doc_type("Your insurance policy renewal — Policy Number 88213"),
            Some(DocType::Insurance)
        );
    }

    #[test]
    fn finds_contract_from_agreement_language() {
        assert_eq!(
            extract_doc_type("Service Agreement between the parties hereto"),
            Some(DocType::Contract)
        );
    }

    #[test]
    fn finds_receipt_from_purchase_language() {
        assert_eq!(
            extract_doc_type("Receipt\nThank you for your purchase\nSubtotal: 24.00"),
            Some(DocType::Receipt)
        );
    }

    #[test]
    fn finds_id_from_passport_language() {
        assert_eq!(
            extract_doc_type("PASSPORT\nDate of birth: 01 JAN 1990"),
            Some(DocType::Id)
        );
    }

    #[test]
    fn insurance_keywords_take_priority_over_a_later_bill_style_amount_due() {
        assert_eq!(
            extract_doc_type("Insurance policy premium statement — Amount due $45.00"),
            Some(DocType::Insurance)
        );
    }

    #[test]
    fn returns_none_when_no_keywords_match() {
        assert_eq!(
            extract_doc_type("Hello world, nothing recognizable here"),
            None
        );
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(
            extract_doc_type("INVOICE amount DUE now"),
            Some(DocType::Bill)
        );
    }

    #[test]
    fn insurance_contract_bill_and_id_are_expiry_eligible() {
        assert!(is_expiry_eligible("insurance"));
        assert!(is_expiry_eligible("contract"));
        assert!(is_expiry_eligible("bill"));
        assert!(is_expiry_eligible("id"));
    }

    #[test]
    fn receipt_and_other_are_not_expiry_eligible() {
        assert!(!is_expiry_eligible("receipt"));
        assert!(!is_expiry_eligible("other"));
    }

    #[test]
    fn an_unrecognized_value_is_not_expiry_eligible() {
        assert!(!is_expiry_eligible("not-a-real-doc-type"));
    }
}
