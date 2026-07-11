//! Form types for the signup/login flows.
//!
//! The newtypes exist so that PII redaction and minimal structural validation
//! are enforced at the type level, per CLAUDE.md's type-driven-constraints
//! and PII-sanitization rules.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use crate::web::error::AppWebError;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct EmailAddress(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid email address")]
pub struct EmailAddressError;

impl TryFrom<String> for EmailAddress {
    type Error = EmailAddressError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        // Deliberately minimal: non-empty, contains '@', and under the RFC 5321
        // practical length limit. Full grammar validation is a poor investment
        // here — the real backstops for a bad address are the `users.email`
        // UNIQUE constraint and eventual delivery failure, not client-side
        // regex perfectionism.
        if !value.is_empty() && value.contains('@') && value.len() <= 254 {
            Ok(Self(value))
        } else {
            Err(EmailAddressError)
        }
    }
}

impl EmailAddress {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Wraps the raw password so it can never be accidentally logged/Debug-printed
/// in full.
#[derive(Clone, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct Password(String);

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("password must be at least {0} characters")]
    TooShort(usize),
    #[error("password must be at most {0} characters")]
    TooLong(usize),
}

const PASSWORD_MIN_LEN: usize = 8;
const PASSWORD_MAX_LEN: usize = 256;

impl TryFrom<String> for Password {
    type Error = PasswordError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        // Length only, deliberately no composition rules (uppercase/digit/
        // symbol) — per NIST 800-63B, forced complexity rules push users
        // toward predictable patterns without meaningfully raising entropy.
        // The upper bound guards against hashing pathologically long input.
        if value.len() < PASSWORD_MIN_LEN {
            Err(PasswordError::TooShort(PASSWORD_MIN_LEN))
        } else if value.len() > PASSWORD_MAX_LEN {
            Err(PasswordError::TooLong(PASSWORD_MAX_LEN))
        } else {
            Ok(Self(value))
        }
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

impl Password {
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Hashes the password, off the async runtime — Argon2 is deliberately
    /// slow, CPU-bound work, and running it inline would stall whichever
    /// worker thread happens to be polling this future.
    pub async fn into_hash(self) -> Result<String, AppWebError> {
        let hash = tokio::task::spawn_blocking(move || {
            let salt = SaltString::generate(&mut OsRng);
            Argon2::default()
                .hash_password(self.as_bytes(), &salt)
                .map(|hash| hash.to_string())
        })
        .await??;
        Ok(hash)
    }

    /// Verifies the password against a stored PHC hash string, off the async
    /// runtime for the same reason as `into_hash`.
    pub async fn matches_hash(self, stored_hash: String) -> Result<bool, AppWebError> {
        let verified = tokio::task::spawn_blocking(move || {
            let parsed_hash = PasswordHash::new(&stored_hash)?;
            Ok::<bool, argon2::password_hash::Error>(
                Argon2::default()
                    .verify_password(self.as_bytes(), &parsed_hash)
                    .is_ok(),
            )
        })
        .await??;
        Ok(verified)
    }
}

/// Shared shape for both the signup and login forms — same fields, same
/// validation. Signup and login diverge in what they *do* with these values
/// (hash-and-insert vs. fetch-and-verify), not in the shape of the submission.
#[derive(Debug, serde::Deserialize)]
pub struct Credentials {
    pub email: EmailAddress,
    pub password: Password,
}
