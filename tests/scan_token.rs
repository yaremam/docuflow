//! `ScanToken` is a plain data type with no DB/HTTP dependency — verified
//! directly here rather than through a full app round-trip, mirroring
//! `tests/reset_token.rs`'s treatment of `ResetToken`.

use docuflow::web::forms::ScanToken;

#[test]
fn generate_produces_64_hex_characters() {
    let token = ScanToken::generate();
    assert_eq!(token.as_str().len(), 64);
    assert!(token.as_str().chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn generate_is_not_deterministic() {
    let a = ScanToken::generate();
    let b = ScanToken::generate();
    assert_ne!(a.as_str(), b.as_str());
}

#[test]
fn hash_is_deterministic_and_64_hex_characters() {
    let token = ScanToken::generate();
    let hash_once = token.hash();
    let hash_again = token.hash();

    assert_eq!(hash_once, hash_again);
    assert_eq!(hash_once.len(), 64);
    assert!(hash_once.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(hash_once, token.as_str());
}

#[test]
fn different_tokens_hash_differently() {
    let a = ScanToken::generate();
    let b = ScanToken::generate();
    assert_ne!(a.hash(), b.hash());
}
