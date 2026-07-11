//! `ResetToken` is a plain data type with no DB/HTTP dependency — verified
//! directly here rather than through a full app round-trip.

use docuflow::web::forms::ResetToken;

#[test]
fn generate_produces_64_hex_characters() {
    let token = ResetToken::generate();
    assert_eq!(token.as_str().len(), 64);
    assert!(token.as_str().chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn generate_is_not_deterministic() {
    let a = ResetToken::generate();
    let b = ResetToken::generate();
    assert_ne!(a.as_str(), b.as_str());
}

#[test]
fn hash_is_deterministic_and_64_hex_characters() {
    let token = ResetToken::generate();
    let hash_once = token.hash();
    let hash_again = token.hash();

    assert_eq!(hash_once, hash_again);
    assert_eq!(hash_once.len(), 64);
    assert!(hash_once.chars().all(|c| c.is_ascii_hexdigit()));
    // The hash must not just be the token re-encoded.
    assert_ne!(hash_once, token.as_str());
}

#[test]
fn different_tokens_hash_differently() {
    let a = ResetToken::generate();
    let b = ResetToken::generate();
    assert_ne!(a.hash(), b.hash());
}
