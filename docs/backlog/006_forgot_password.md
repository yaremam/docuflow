# User Story: Forgot Password

## 1. User Value Statement
As a **DocuFlow user who has forgotten their password**,
I want to **request a password-reset link by email and use it to set a new password**,
So that **I can regain access to my account without contacting support or losing my documents.**

## 2. Strict Acceptance Criteria
- **AC-1:** `GET /forgot-password` renders a form asking for an email address.
- **AC-2:** `POST /forgot-password` returns the identical response (status, body, and redirect target) regardless of whether the submitted email matches an account — the endpoint must not be usable to test which emails have accounts.
- **AC-3:** When the email matches an account, a single-use reset token is generated, its hash (never the raw token) is stored with a 1-hour expiry, and an email containing a reset link is sent via SMTP.
- **AC-4:** `GET /reset-password?token=...` validates the token (exists, unexpired, unused) without consuming it, rendering either the new-password form or an "invalid or expired" state.
- **AC-5:** `POST /reset-password` re-validates the token; on success it atomically updates the account's password and marks the token used, so the same token can never be used twice, then establishes a logged-in session and redirects to `/welcome`.
- **AC-6:** A previously-used or expired token submitted to `POST /reset-password` is rejected with `400` and never changes the password.
- **AC-7:** No `.unwrap()`, `.expect()`, or `panic!()` in the new handler/mailer code; a database, hashing, or mail-transport failure surfaces as a `Result`/`thiserror` error mapped to a proper HTTP response, never a panic.
- **AC-8:** No reset token, password, or email address ever appears as a span attribute or log field in plaintext (the raw token is redacted in `Debug`; only the hash is persisted).
