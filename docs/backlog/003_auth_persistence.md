# User Story: Real Signup/Login with Postgres-Backed Sessions

## 1. User Value Statement
As a **Public Visitor creating a DocuFlow account**,
I want to **sign up with an email and password that are actually persisted and hashed, and log in to a real authenticated session**,
So that **my account is durably tied to me and isolated from every other tenant, across requests, instead of the current signup/login stubs that discard everything.**

## 2. Strict Acceptance Criteria
- **AC-1:** `POST /signup` with a valid, not-already-registered email and password creates exactly one row in `tenants` and one row in `users` (the new user's tenant id equals their own id, per the 1:1 tenancy model), stores only an Argon2id password hash — never the plaintext — and establishes an authenticated session (`Set-Cookie`) so the user is logged in immediately after signup.
- **AC-2:** `POST /signup` with an email that already exists in `users` creates no new row in either table, and the failure response does not let a caller reliably distinguish "email already taken" from other validation failures via status code, body shape, or response timing.
- **AC-3:** `POST /login` with a correct email/password for an existing user establishes a new session (`Set-Cookie`); a wrong password for a real email and a login attempt for a non-existent email both produce the exact same status code and body — no user-enumeration oracle.
- **AC-4:** The session cookie is `HttpOnly` and `SameSite`-restricted, backed by Postgres via `tower-sessions` (not an in-memory or client-signed-only store), such that a second request presenting the same cookie is recognized as the same authenticated user without re-submitting credentials.
- **AC-5:** `POST /logout` invalidates the session server-side (deletes the Postgres-backed session record, not just clearing the cookie), such that replaying the same cookie value after logout is no longer authenticated.
- **AC-6:** Every authenticated request has its `TenantId` and `UserId` extracted via an Axum extractor/middleware layer (rejecting requests with no valid session on routes that require one), and both values are injected into the active OpenTelemetry context as Baggage (`tenant.id`, `user.id`), per CLAUDE.md's multi-tenancy rule.
- **AC-7:** Every request to `/signup`, `/login`, and `/logout` emits a trace span visible in the local Jaeger UI; no plaintext password, password hash, or raw session-cookie value ever appears as a span attribute, log field, or `Debug` output, at any log level.
- **AC-8:** No `.unwrap()`, `.expect()`, or `panic!()` in the new persistence/hashing/session code; database errors, hashing errors, and unique-constraint violations all surface as typed `Result`/`thiserror` errors mapped to proper HTTP responses, never a panic or a raw 500 stack trace.
