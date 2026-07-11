# TDR 003: Auth Persistence — Hashing, Postgres Users, and Sessions

## 1. Context & Architectural Requirements
Feature 002 wired real `/signup` and `/login` routes but left both POST handlers intentionally stubbed: they validate structurally via the `EmailAddress`/`Password` newtypes and then discard the submission, responding `501`. TDR 002 explicitly named this feature — "argon2 hashing, a `users` table, `tower-sessions` or `axum-login`" — as the seam it deferred to. This is also the first feature to introduce a live database connection: no `PgPool`, `AppState`, or migrations exist anywhere in the codebase today, even though `sqlx` has been a declared (unused) dependency since the initial scaffold. It is likewise the first feature to satisfy CLAUDE.md's blanket multi-tenancy rule — "every incoming HTTP request must extract a `TenantId` and `UserId` via an Axum extractor middleware layer" and inject them into OTel Baggage — which both TDR 001 and TDR 002 flagged as not-yet-implemented.

Scope boundary: no protected application routes exist yet beyond auth itself (no document upload, no OCR, no dashboard). This feature proves the tenancy-extraction path end-to-end on the one route that can meaningfully require it today (`/logout`), leaving it ready for future protected routes to reuse without re-wiring.

## 2. Alternatives Evaluated

### Alternative A: Hand-rolled signed cookie (no session table)
- **Pros:** Minimal dependency footprint — sign a cookie containing the user id directly (e.g. via `axum-extra`'s `PrivateCookieJar`), no server-side store to manage.
- **Cons:** We own all the security-sensitive signing, expiry, and revocation logic ourselves. Critically, AC-5 (server-side logout invalidation) is not achievable with a purely client-signed cookie — a stateless signed token remains valid until it expires, it can't be revoked early without a server-side denylist, which is just a session store by another name.

### Alternative B: `axum-login`
- **Pros:** Built on the same `tower-sessions-core` foundation; adds an opinionated `AuthUser`/`AuthnBackend` trait layer aimed at authorization (roles, permissions) on top of authentication.
- **Cons:** That trait-layer abstraction solves a problem — role/permission-based authorization — that this feature doesn't have yet (there is nothing to authorize access to besides "is there a valid session"). Adopting it now means carrying scaffolding this feature won't exercise. Since it sits on `tower-sessions-core`, it can be layered in later without disrupting the session-storage choice made here.

### Alternative C: `tower-sessions` + `tower-sessions-sqlx-store` (Postgres), used directly
- **Pros:** Solves exactly what's needed — server-side session state backed by Postgres, an `HttpOnly`/`SameSite` cookie carrying only an opaque session id, and a `Session::flush()` call that gives AC-5's real invalidation for free. `PostgresStore::migrate()` self-manages its own session table, so no additional migration authoring is required. Matches CLAUDE.md's Postgres-backed persistence expectation without inventing bespoke cookie-signing code.
- **Cons:** One more dependency pair versus Alternative A; carries none of Alternative B's authorization scaffolding, so a future roles/permissions feature will need to add that layer itself.

**Chosen: Alternative C.**

---

### Alternative D: Real multi-user tenants table (tenants ↔ users join, invite flow)
- **Pros:** Matches CLAUDE.md's "multi-user multi-tenancy" language most literally; no schema rework needed if multiple users per tenant is required soon.
- **Cons:** Unbounded scope for this feature — invite flows, membership roles, and an org-creation UX are not requested and have no acceptance criteria here.

### Alternative E: No `tenants` table at all — `users.id` doubles as tenant id
- **Pros:** Simplest possible schema; one fewer table and one fewer join for every query.
- **Cons:** Violates CLAUDE.md's type-driven-constraints rule (no distinct `TenantId` type to prevent a `UserId` being passed where a `TenantId` is expected) and would force an invasive rename/migration later if multi-user tenants are ever added, since nothing in the schema or types currently distinguishes the two concepts.

### Alternative F: 1:1 tenant-per-user, with a real (if trivial) `tenants` table
- **Pros:** Every signup creates one `tenants` row and one `users` row sharing a UUID; the FK and the `TenantId`/`UserId` newtype distinction are both real from day one, so a future many-users-per-tenant migration only has to relax an invariant (that `users.tenant_id` is always the signing-up user's own id), not reshape the schema or retrofit types across the codebase.
- **Cons:** A small amount of schema/type ceremony (a second table, two newtypes) that does nothing observably different from Alternative E today.

**Chosen: Alternative F.**

---

### Alternative G: Full RFC 5322 email grammar validation
- **Pros:** Rejects the widest range of malformed addresses at the edge.
- **Cons:** RFC 5322's full grammar is notoriously permissive and complex to implement correctly; most of its value (does this address actually receive mail) can't be verified without sending mail anyway, which is out of scope here.

### Alternative H: Adopt the `validator` crate's email check
- **Pros:** One dependency, regex-based, more thorough than a bare `contains('@')` check.
- **Cons:** Adds a validation-framework dependency the project has deliberately avoided so far (forms.rs's existing newtypes are hand-rolled `TryFrom` on purpose); its regex still doesn't guarantee deliverability.

### Alternative I: Keep the existing minimal check, add only a max-length bound
- **Pros:** The real backstops for email correctness are the database's `UNIQUE` constraint (catches duplicates) and eventual delivery failure (catches typos), not client-side grammar perfectionism. A length bound (~254 chars, RFC 5321's practical limit) closes the one concrete gap — an unbounded string reaching the database — without adding a dependency.
- **Cons:** Still permits some locally-invalid strings (e.g. `a@b`) through to the database layer.

**Chosen: Alternative I.** This is a deliberate deferral, not an oversight — recorded here so it isn't mistaken for one later.

## 3. Structural Decision
We adopt `argon2 = "0.5"` for password hashing, `tower-sessions = "0.15"` (feature `axum-core`) with `tower-sessions-sqlx-store = "0.15"` (feature `postgres`) for session storage, and `uuid = "1"` (features `v4`, `serde`) as a direct dependency — previously only pulled in transitively via `sqlx`'s `uuid` feature flag, now needed directly for the new `TenantId`/`UserId` domain newtypes. `sqlx`'s `macros` and `migrate` features are already active by default in the existing `Cargo.toml` entry (no `default-features = false` is set), so `sqlx::query!`/`sqlx::migrate!` require no dependency change — worth stating explicitly since it's easy to assume otherwise.

`AppState { pool: PgPool }` is introduced for the first time; `web::router::app()` becomes `web::router::app(state: AppState) -> Router`, applying `SessionManagerLayer` (wrapping the whole router, required before any handler can extract `tower_sessions::Session`) ahead of `.with_state(state)`. `main.rs` builds the pool from `DATABASE_URL`, runs our own migrations via `sqlx::migrate!`, and separately calls `PostgresStore::migrate()` for `tower-sessions`' self-managed session table — that table lives outside our `migrations/` directory entirely, since the store issues its own raw `sqlx::query()` calls (not the `query!` macro) and therefore needs no entry in the checked-in `.sqlx/` offline-query cache.

Tenancy is 1:1: signup mints one `Uuid::new_v4()` used as both the new user's id and their tenant's id, inserting both rows in a single transaction. `TenantId(Uuid)`/`UserId(Uuid)` remain distinct newtypes per CLAUDE.md's type-driven-constraints rule even though they carry the same value in every case this feature can produce.

Anti-enumeration (AC-2, AC-3) is implemented by routing both "wrong password" and "no such user" through the identical `AppWebError::InvalidCredentials` response, and by giving signup's duplicate-email path the same generic response shape as other signup validation failures, rather than a distinguishable "email taken" message.

## 4. OpenTelemetry Implications
`/signup`, `/login`, and the new `/logout` route all carry `#[tracing::instrument(skip(form))]` (or `skip(form, session)` where a `Session` is also a parameter) — matching the existing PII-redaction idiom from `src/web/handlers/auth.rs` and `Password`'s manually-redacted `Debug` impl. Any new sensitive types this feature introduces (the argon2 `PasswordHash`, the raw session-cookie value) get the same treatment: never captured as a span attribute, never printed via a derived `Debug`.

This is the first feature to populate `TenantId`/`UserId` OTel Baggage, closing the gap both TDR 001 and TDR 002 flagged as deferred. The new `web::tenancy::TenantContext` extractor sets `tenant.id`/`user.id` as Baggage on the active `Context` immediately after successfully reading the session, so every span created downstream of that extraction — for now, just the `/logout` handler's own span, since no other protected route exists yet — carries both values without each handler needing to set them manually. Future protected routes join the same nested router the extractor is scoped to, inheriting this instrumentation automatically rather than needing their own Baggage-setting code.
