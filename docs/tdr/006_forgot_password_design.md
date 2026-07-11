# TDR 006: Forgot Password

## 1. Context & Architectural Requirements
There is no account-recovery path today — a user who forgets their password
has no way back into their account. This feature adds a standard
email-a-reset-link flow, which requires two new capabilities the codebase
doesn't have yet: a single-use, expiring, out-of-band credential (the reset
token) and outbound email delivery (SMTP). Both need to fit the existing
anti-enumeration posture established by signup/login (TDR 003) and the
Zero-Panic-Safety / PII-sanitization rules from CLAUDE.md.

## 2. Alternatives Evaluated

### Alternative A: Store the raw reset token in the database
- **Pros:** Simplest possible lookup — compare the submitted token directly.
- **Cons:** A database leak would hand out live, directly-usable reset
  links for every account with an outstanding request — the token itself
  *is* the credential, so storing it in plaintext is equivalent to storing
  a password in plaintext.

### Alternative B: Hash the token with Argon2, like `Password`
- **Pros:** Reuses the exact hashing primitive already in the codebase.
- **Cons:** Argon2's deliberate slowness exists to defend a low-entropy,
  human-guessable secret against brute force. A reset token is
  CSPRNG-generated with 256 bits of entropy — there is nothing to
  brute-force. Argon2 would tax every legitimate reset-link click with
  needless CPU-bound latency for zero security benefit.

### Alternative C: Hash the token with SHA-256, store only the hash (chosen)
- **Pros:** A fast one-way hash defeats the actual threat (a DB leak handing
  out usable tokens) exactly as well as a slow one would, without taxing
  the legitimate path. `ResetToken::generate()` reuses the `uuid` crate's
  own CSPRNG (two concatenated `Uuid::new_v4()`s, hex-encoded) rather than
  adding a `rand`/`base64` dependency for one call site.
- **Cons:** None identified for this threat model.

**Chosen: Alternative C.**

---

### Alternative D: Send reset emails via a third-party transactional-email API (e.g. an HTTP-based provider SDK)
- **Pros:** Offloads deliverability/retry concerns to the provider.
- **Cons:** Requires a real API key/account to test locally at all, adding
  friction to development and CI; over-engineered for a project at this
  stage.

### Alternative E: Real SMTP via `lettre`, with Mailpit as the local dev mail-catcher (chosen)
- **Pros:** `lettre` speaks standard SMTP, so the exact same code path
  targets a real provider in production and Mailpit (a local dev-only
  mail-catcher with a web UI at `localhost:8025`) in development — selected
  purely by environment variables (`SMTP_HOST`/`SMTP_PORT`/`SMTP_INSECURE`),
  not a compile-time switch or a mocked-out code path. Nothing needs a real
  credential to develop or test against.
- **Cons:** One more Docker Compose service to run locally.

**Chosen: Alternative E**, per the user's explicit direction this feature
round.

---

### Alternative F: Fold password-reset handlers into `src/web/handlers/auth.rs`
- **Pros:** One fewer file.
- **Cons:** `auth.rs` is already focused on credential establishment
  (signup/login/logout); password reset is a genuinely distinct flow with
  its own table and its own `Mailer` dependency, and would push `auth.rs`
  well past the ~100–200 line range every other handler module in this
  codebase stays within.

### Alternative G: New `src/web/handlers/password_reset.rs` module (chosen)
- **Pros:** Keeps each handler module scoped to one flow, consistent with
  the existing `profile.rs`/`landing.rs`/`auth.rs` split.
- **Cons:** None identified.

**Chosen: Alternative G.**

## 3. Structural Decision
`migrations/*_create_password_reset_tokens.sql` adds a `password_reset_tokens`
table (`id`, `user_id` FK, `token_hash` unique, `expires_at`, `used_at`
nullable, `created_at`) — a separate table rather than columns on `users`,
since a user may have multiple outstanding/expired tokens over time and this
avoids overloading the `users` row with transient state.

`ResetToken` (in `src/web/forms.rs`, alongside `Password`/`EmailAddress`)
generates the token, and separately exposes `.hash()` for storage/lookup;
its `Debug` impl is redacted like `Password`'s.

`src/mailer.rs` (new module) wraps a `lettre::AsyncSmtpTransport`, built from
`SMTP_HOST`/`SMTP_PORT`/`SMTP_FROM`/`SMTP_INSECURE` env vars.
`SMTP_INSECURE=true` selects `lettre`'s plaintext `builder_dangerous`
transport (required for Mailpit, which has no TLS) instead of `relay()`
(TLS, for a real provider) — gated behind an explicit flag so a real
deployment can never silently fall back to an unencrypted connection.
`AppState` gains a `mailer: Mailer` field.

`src/web/handlers/password_reset.rs` provides four handlers:
- `forgot_password_form` / `forgot_password_submit`: the submit handler
  looks up the user by email but takes the *same* branch to the same
  `/forgot-password?sent=true` redirect either way (AC-2); only inside the
  "found" branch does it generate a token, insert its hash with a 1-hour
  expiry, and send the email. A mail-transport failure is logged and
  swallowed rather than propagated, so a transient SMTP outage can't turn
  into a response that's distinguishable from the "email not found" path.
- `reset_password_form`: validates the token via a plain `SELECT` (exists,
  `used_at is null`, `expires_at > now()`) without consuming it.
- `reset_password_submit`: hashes the new password *before* opening a
  transaction (Argon2 is deliberately slow — there's no reason to hold a
  database transaction open for it), then in **one explicit transaction**
  re-validates the token with `SELECT ... FOR UPDATE` (row-locking it so a
  second concurrent submit of the same still-unused token can't also pass
  validation before this one marks it used), updates `users.password_hash`,
  and marks the token `used_at`, committing both together. This is the
  first explicit multi-statement transaction in the codebase — signup's
  single-statement CTE was sufficient there because it only ever touches one
  row's worth of atomicity; this flow genuinely needs two independently
  updated tables committed together.
  Establishes a session immediately after commit (the token already proved
  control of the account's email — equivalent trust to a fresh login) and
  redirects to `/welcome`, soft-failing to `/login` on a session-store error
  exactly like `signup_submit` already does.

`AppWebError` gains `InvalidResetToken` (mapped to an explicit `400`, not
the generic `500` fallback) and `Mail(String)`.

Router: `/forgot-password` and `/reset-password` (GET+POST) join `pages`
directly, unprotected — same level as `/signup`/`/login`, reachable whether
or not the caller has a session (a logged-in user can still forget a
password they're not currently using).

## 4. OpenTelemetry Implications
`Mailer::send_reset_email` is `#[tracing::instrument(skip(self))]` since
both its parameters (recipient address, reset URL containing the raw token)
are sensitive — neither appears as a span attribute. `ResetToken`'s redacted
`Debug` impl means even an accidental `{:?}` in a log statement can't leak
the raw token. All four new handlers carry `#[tracing::instrument(skip(...))]`
with `state`/`form`/`query` parameters skipped, matching the existing
PII-redaction idiom from `Password`/profile handling — only the (non-PII)
control flow, not field values, is ever implicitly captured.
