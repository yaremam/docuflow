//! Form types for the stubbed signup/login flows.
//!
//! Nothing here is persisted or hashed yet — that's scope for a follow-up
//! auth-persistence feature. The newtypes exist so that PII redaction and
//! minimal structural validation are in place from day one, per CLAUDE.md's
//! type-driven-constraints and PII-sanitization rules.

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct EmailAddress(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid email address")]
pub struct EmailAddressError;

impl TryFrom<String> for EmailAddress {
    type Error = EmailAddressError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        // Deliberately minimal for this stubbed pass: non-empty and contains '@'.
        // Full RFC-grade validation belongs to the follow-up auth-persistence feature.
        if !value.is_empty() && value.contains('@') {
            Ok(Self(value))
        } else {
            Err(EmailAddressError)
        }
    }
}

/// Wraps the raw password so it can never be accidentally logged/Debug-printed
/// in full. No hashing/persistence here yet — deferred to the auth-persistence
/// follow-up feature.
#[derive(Clone, serde::Deserialize)]
#[serde(transparent)]
pub struct Password(String);

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

/// Shared shape for both the signup and login forms — same fields, same
/// validation, until a follow-up feature gives them distinct semantics.
#[derive(Debug, serde::Deserialize)]
pub struct Credentials {
    pub email: EmailAddress,
    pub password: Password,
}

pub type SignupForm = Credentials;
pub type LoginForm = Credentials;
