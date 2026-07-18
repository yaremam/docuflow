//! SMTP email delivery for password-reset links. Real SMTP via `lettre` —
//! in local dev this points at Mailpit (a dev-only mail-catcher with a web
//! UI, see `docker-compose.yml`), in production at a real provider,
//! selected purely by environment variables, not a compile-time switch.
//! See `deploy/docker-compose.yml` and `deploy/.env.example` for the
//! self-hoster-facing instructions on swapping in a real provider.

use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::error::AppError;
use crate::web::error::AppWebError;

/// The implicit-TLS ("SMTPS") convention; every other port — 587
/// (STARTTLS, the common real-provider default) included — upgrades from
/// plaintext via STARTTLS instead. Mailpit's plaintext port is handled
/// separately via `SMTP_INSECURE`, never through either TLS path.
const IMPLICIT_TLS_PORT: u16 = 465;

#[derive(Clone)]
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: String,
}

impl Mailer {
    pub fn from_env() -> Result<Self, AppError> {
        let host = std::env::var("SMTP_HOST").unwrap_or_else(|_| "localhost".to_string());
        let port: u16 = std::env::var("SMTP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(1025);
        let from = std::env::var("SMTP_FROM").unwrap_or_else(|_| "no-reply@docuflow.local".to_string());
        let credentials = paired_credentials(
            blank_as_none(std::env::var("SMTP_USERNAME").ok()),
            blank_as_none(std::env::var("SMTP_PASSWORD").ok()),
        )?;

        // Mailpit (the local dev relay) speaks plaintext SMTP with no TLS
        // at all; a real provider needs TLS, either implicit (465) or
        // STARTTLS (everything else — 587 in particular). `builder_dangerous`
        // is lettre's own documented name for "connect without TLS" — used
        // deliberately for the local relay, gated behind an explicit env
        // flag so a real deployment can never silently fall back to it.
        let mut builder = if std::env::var("SMTP_INSECURE").as_deref() == Ok("true") {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&host)
        } else if implicit_tls(port) {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&host).map_err(|e| AppError::Mail(e.to_string()))?
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host).map_err(|e| AppError::Mail(e.to_string()))?
        };
        if let Some(credentials) = credentials {
            builder = builder.credentials(credentials);
        }
        let transport = builder.port(port).build();

        Ok(Self { transport, from })
    }

    #[tracing::instrument(skip(self))]
    pub async fn send_reset_email(&self, to: &str, reset_url: &str) -> Result<(), AppWebError> {
        let from: Mailbox = self.from.parse().map_err(mail_err)?;
        let to: Mailbox = to.parse().map_err(mail_err)?;

        let email = Message::builder()
            .from(from)
            .to(to)
            .subject("Reset your DocuFlow password")
            .body(format!(
                "We received a request to reset your DocuFlow password.\n\n\
                 Reset it here (valid for 1 hour): {reset_url}\n\n\
                 If you didn't request this, you can ignore this email."
            ))
            .map_err(mail_err)?;

        AsyncTransport::send(&self.transport, email)
            .await
            .map_err(mail_err)?;
        Ok(())
    }
}

/// Shared conversion for `send_reset_email`'s three fallible steps (address
/// parsing, message building, transport send — three different lettre error
/// types, all mapped the same way) so they don't each hand-roll their own
/// `AppWebError::Mail(...)` wrapper.
fn mail_err(e: impl std::fmt::Display) -> AppWebError {
    AppWebError::Mail(e.to_string())
}

fn implicit_tls(port: u16) -> bool {
    port == IMPLICIT_TLS_PORT
}

/// Blank counts as unset — `deploy/docker-compose.yml` passes
/// `${SMTP_USERNAME:-}` through unconditionally so it's overridable via
/// `.env` alone, which means "not configured" arrives here as `Some("")`,
/// not `None`.
fn blank_as_none(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

/// `SMTP_USERNAME`/`SMTP_PASSWORD` only make sense as a pair — a lone one
/// set is a config error caught here at startup, rather than a silent
/// unauthenticated connection attempt that a real provider would reject
/// anyway, later and less legibly.
fn paired_credentials(username: Option<String>, password: Option<String>) -> Result<Option<Credentials>, AppError> {
    match (username, password) {
        (Some(username), Some(password)) => Ok(Some(Credentials::new(username, password))),
        (None, None) => Ok(None),
        _ => Err(AppError::InvalidConfig(
            "SMTP_USERNAME/SMTP_PASSWORD",
            "must both be set, or neither".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_465_selects_implicit_tls() {
        assert!(implicit_tls(465));
    }

    #[test]
    fn port_587_and_others_select_starttls() {
        assert!(!implicit_tls(587));
        assert!(!implicit_tls(1025));
    }

    #[test]
    fn blank_string_counts_as_unset() {
        assert_eq!(blank_as_none(Some(String::new())), None);
        assert_eq!(blank_as_none(None), None);
        assert_eq!(blank_as_none(Some("apikey".to_string())), Some("apikey".to_string()));
    }

    #[test]
    fn neither_credential_var_set_is_fine() {
        assert!(paired_credentials(None, None).unwrap().is_none());
    }

    #[test]
    fn both_credential_vars_set_produce_credentials() {
        let credentials = paired_credentials(Some("apikey".to_string()), Some("secret".to_string()))
            .unwrap()
            .unwrap();
        assert_eq!(credentials, Credentials::new("apikey".to_string(), "secret".to_string()));
    }

    #[test]
    fn username_without_password_is_rejected() {
        let result = paired_credentials(Some("apikey".to_string()), None);
        assert!(matches!(result, Err(AppError::InvalidConfig("SMTP_USERNAME/SMTP_PASSWORD", _))));
    }

    #[test]
    fn password_without_username_is_rejected() {
        let result = paired_credentials(None, Some("secret".to_string()));
        assert!(matches!(result, Err(AppError::InvalidConfig("SMTP_USERNAME/SMTP_PASSWORD", _))));
    }
}
