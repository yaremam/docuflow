//! Form types for the signup/login/profile flows.
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

/// Shared newtype for the free-text profile fields (name, address
/// components, phone). Unlike `Password`/`EmailAddress`, none of these
/// fields have distinct per-field behavior (no hashing, no format grammar),
/// so one newtype with a generous shared length cap is the pragmatic middle
/// ground between "raw `String`, no constraint at all" and "seven
/// near-identical single-field newtypes."
#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct ProfileField(String);

const PROFILE_FIELD_MAX_LEN: usize = 200;

#[derive(Debug, thiserror::Error)]
#[error("must be at most {PROFILE_FIELD_MAX_LEN} characters")]
pub struct ProfileFieldError;

impl TryFrom<String> for ProfileField {
    type Error = ProfileFieldError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.len() > PROFILE_FIELD_MAX_LEN {
            Err(ProfileFieldError)
        } else {
            Ok(Self(trimmed.to_string()))
        }
    }
}

impl ProfileField {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Blank after trimming means "clear this field" — maps to SQL `null`
    /// rather than storing an empty string.
    pub fn into_option(self) -> Option<String> {
        if self.0.is_empty() {
            None
        } else {
            Some(self.0)
        }
    }
}

/// The editable profile fields, all optional to submit (an absent/blank
/// field clears that column).
#[derive(Debug, serde::Deserialize)]
pub struct ProfileForm {
    #[serde(default)]
    pub first_name: ProfileField,
    #[serde(default)]
    pub last_name: ProfileField,
    #[serde(default)]
    pub street_address: ProfileField,
    #[serde(default)]
    pub city: ProfileField,
    #[serde(default)]
    pub postcode: ProfileField,
    #[serde(default)]
    pub country: ProfileField,
    #[serde(default)]
    pub phone: ProfileField,
}

/// Opaque password-reset token, generated at `/forgot-password` time and
/// carried in the emailed reset link. Unlike `Password`/`EmailAddress`
/// there's no grammar to enforce on the way back in — any string that
/// doesn't match a stored hash simply fails lookup — so this is a plain
/// wrapper rather than a `TryFrom<String>` newtype, used directly as a
/// `Form`/`Query` field.
#[derive(Clone, serde::Deserialize)]
pub struct ResetToken(String);

impl std::fmt::Debug for ResetToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResetToken(<redacted>)")
    }
}

impl ResetToken {
    /// Two concatenated v4 UUIDs, hex-encoded — 256 bits of CSPRNG entropy,
    /// inherently URL-safe. Reuses the `uuid` crate's own OS-RNG-backed
    /// generator rather than adding a `rand`/`base64` dependency for this
    /// one call site.
    pub fn generate() -> Self {
        let a = uuid::Uuid::new_v4().as_simple().to_string();
        let b = uuid::Uuid::new_v4().as_simple().to_string();
        Self(format!("{a}{b}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Hex-encoded SHA-256 of the token, for storage/lookup. A fast hash is
    /// the right choice here — unlike `Password` (a low-entropy, guessable
    /// human secret that Argon2's deliberate slowness defends against), a
    /// generated token already has 256 bits of entropy with nothing to
    /// brute-force; the only real threat is a database leak handing out
    /// live tokens, which a fast one-way hash defeats just as well as a
    /// slow one, without taxing every legitimate reset-link click.
    pub fn hash(&self) -> String {
        use sha2::Digest;
        sha2::Sha256::digest(self.0.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

