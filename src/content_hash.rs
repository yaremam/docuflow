//! Content-fingerprinting for duplicate detection (feature 029): a plain
//! hex-encoded SHA-256 of a document's raw file bytes, matching the
//! `web::forms::hash_hex_token` hex-encoding style already used for
//! tokens elsewhere in this app. Not a security primitive — a fast,
//! well-distributed fingerprint is all "did I already upload this exact
//! file?" needs.

/// One-shot hash of an already-in-memory buffer — used by the phone-scan
/// path (an assembled PDF's bytes are already fully in memory) and by
/// `run_ocr`'s reprocess-time backfill (it already re-fetches the full
/// blob for OCR, so no second read is needed there either).
pub fn hash_bytes(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex_encode(sha2::Sha256::digest(bytes))
}

/// Finalizes a running `Sha256` hasher into the same hex encoding —
/// lets `BlobStore::stream_upload` compute the hash incrementally, in
/// the same pass it already reads chunks for its byte-count check, so a
/// direct desktop upload never needs a second read of its bytes.
pub fn finalize_hex(hasher: sha2::Sha256) -> String {
    use sha2::Digest;
    hex_encode(hasher.finalize())
}

fn hex_encode(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    #[test]
    fn hash_bytes_matches_the_standard_empty_string_vector() {
        assert_eq!(
            hash_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hash_bytes_matches_a_known_vector() {
        assert_eq!(
            hash_bytes(b"hello world"),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn hash_bytes_is_deterministic() {
        assert_eq!(
            hash_bytes(b"electric company bill"),
            hash_bytes(b"electric company bill")
        );
    }

    #[test]
    fn different_bytes_hash_differently() {
        assert_ne!(
            hash_bytes(b"electric company bill"),
            hash_bytes(b"water company bill")
        );
    }

    #[test]
    fn finalize_hex_matches_hash_bytes_for_the_same_content() {
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"chunk one ");
        hasher.update(b"chunk two");
        assert_eq!(finalize_hex(hasher), hash_bytes(b"chunk one chunk two"));
    }
}
