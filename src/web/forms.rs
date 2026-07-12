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

/// Two concatenated v4 UUIDs, hex-encoded — 256 bits of CSPRNG entropy,
/// inherently URL-safe. Reuses the `uuid` crate's own OS-RNG-backed
/// generator rather than adding a `rand`/`base64` dependency. Shared by
/// `ResetToken` and `ScanToken` below — both are "opaque one-time secret,
/// only its hash ever persisted" tokens with identical generation and
/// hashing needs, just embedded in different places (an emailed link vs. a
/// QR-encoded URL) and read back via different extractors (`Query` vs.
/// `Path`), which is a difference in how each type is *used*, not in how
/// the token itself is generated or hashed.
fn generate_hex_token() -> String {
    let a = uuid::Uuid::new_v4().as_simple().to_string();
    let b = uuid::Uuid::new_v4().as_simple().to_string();
    format!("{a}{b}")
}

/// Hex-encoded SHA-256 of `token`, for storage/lookup. A fast hash is the
/// right choice here — unlike `Password` (a low-entropy, guessable human
/// secret that Argon2's deliberate slowness defends against), a generated
/// token already has 256 bits of entropy with nothing to brute-force; the
/// only real threat is a database leak handing out live tokens, which a
/// fast one-way hash defeats just as well as a slow one, without taxing
/// every legitimate reset-link click or scan poll.
fn hash_hex_token(token: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
    pub fn generate() -> Self {
        Self(generate_hex_token())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn hash(&self) -> String {
        hash_hex_token(&self.0)
    }
}

/// Opaque phone-camera-scan handoff token: minted at `GET /scan` (desktop,
/// authenticated) and embedded in the QR-encoded URL the phone loads. Same
/// shape and rationale as `ResetToken` just above (256-bit CSPRNG value, only
/// its SHA-256 hash persisted) — kept as a distinct type since it's read
/// back via `Path` (`/scan/:token`) rather than `Query`, but shares the
/// actual generate/hash logic via `generate_hex_token`/`hash_hex_token`.
#[derive(Clone, serde::Deserialize)]
pub struct ScanToken(String);

impl std::fmt::Debug for ScanToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScanToken(<redacted>)")
    }
}

/// Wraps an already-known raw token string so callers (namely tests seeding
/// a `scan_sessions` row directly) can compute its `.hash()` without
/// hand-rolling the same SHA-256-hex logic a second time. Doesn't weaken
/// anything `ScanToken` already guarantees — it's `Deserialize` directly
/// from an arbitrary path segment, so it never promised its contents were
/// only ever a `generate()`-produced value.
impl From<String> for ScanToken {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl ScanToken {
    pub fn generate() -> Self {
        Self(generate_hex_token())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn hash(&self) -> String {
        hash_hex_token(&self.0)
    }
}

const TAGS_MAX_COUNT: usize = 20;
const TAG_MAX_LEN: usize = 50;

/// A document's free-form tags, submitted as one comma-separated `<input>`
/// value (no dedicated tags-input widget exists) and stored as a Postgres
/// `text[]`.
#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct Tags(Vec<String>);

#[derive(Debug, thiserror::Error)]
pub enum TagsError {
    #[error("no more than {TAGS_MAX_COUNT} tags are allowed")]
    TooMany,
    #[error("each tag must be at most {TAG_MAX_LEN} characters")]
    TagTooLong,
}

impl TryFrom<String> for Tags {
    type Error = TagsError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let tags: Vec<String> = value
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_string)
            .collect();

        if tags.len() > TAGS_MAX_COUNT {
            return Err(TagsError::TooMany);
        }
        if tags.iter().any(|tag| tag.len() > TAG_MAX_LEN) {
            return Err(TagsError::TagTooLong);
        }

        Ok(Self(tags))
    }
}

impl Tags {
    pub fn into_vec(self) -> Vec<String> {
        self.0
    }

    /// Repopulates the edit form's comma-separated text input.
    pub fn to_input_value(&self) -> String {
        self.0.join(", ")
    }
}

/// A document's user-entered real-world date (as opposed to its automatic
/// upload timestamp), submitted via `<input type="date">`'s wire format
/// (`YYYY-MM-DD`). Blank clears the field, mirroring `ProfileField`'s
/// convention. Parsed by hand rather than via `time`'s `macros`/`parsing`
/// features (not enabled in this project's `Cargo.toml`) to avoid adding a
/// new feature flag for one call site.
#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct DateIssuedField(Option<time::Date>);

#[derive(Debug, thiserror::Error)]
#[error("date issued must be blank or in YYYY-MM-DD format")]
pub struct DateIssuedFieldError;

impl TryFrom<String> for DateIssuedField {
    type Error = DateIssuedFieldError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(Self(None));
        }

        let (year_str, rest) = trimmed.split_once('-').ok_or(DateIssuedFieldError)?;
        let (month_str, day_str) = rest.split_once('-').ok_or(DateIssuedFieldError)?;

        let year: i32 = year_str.parse().map_err(|_| DateIssuedFieldError)?;
        let month: u8 = month_str.parse().map_err(|_| DateIssuedFieldError)?;
        let day: u8 = day_str.parse().map_err(|_| DateIssuedFieldError)?;
        let month = time::Month::try_from(month).map_err(|_| DateIssuedFieldError)?;
        let date = time::Date::from_calendar_date(year, month, day).map_err(|_| DateIssuedFieldError)?;

        Ok(Self(Some(date)))
    }
}

impl DateIssuedField {
    pub fn into_option(self) -> Option<time::Date> {
        self.0
    }
}

