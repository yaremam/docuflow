//! SMTP email delivery for password-reset links. Real SMTP via `lettre` —
//! in local dev this points at Mailpit (a dev-only mail-catcher with a web
//! UI, see `docker-compose.yml`), in production at a real provider,
//! selected purely by environment variables, not a compile-time switch.

use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::error::AppError;
use crate::web::error::AppWebError;

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

        // Mailpit (the local dev relay) speaks plaintext SMTP with no TLS
        // at all; a real provider needs TLS. `builder_dangerous` is
        // lettre's own documented name for "connect without TLS" — used
        // deliberately for the local relay, gated behind an explicit env
        // flag so a real deployment can never silently fall back to it.
        let builder = if std::env::var("SMTP_INSECURE").as_deref() == Ok("true") {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&host).map_err(|e| AppError::Mail(e.to_string()))?
        };
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
