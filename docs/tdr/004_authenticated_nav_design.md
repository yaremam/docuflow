# TDR 004: Auth-Aware Navigation

## 1. Context & Architectural Requirements
Every page template extends `templates/base.html`, whose nav bar unconditionally
links to `/login`/`/signup`. The only existing auth extractor, `TenantContext`
(`src/web/tenancy.rs`, introduced in feature 003), **hard-rejects**
unauthenticated requests — it redirects to `/login` on failure, which is
correct for a protected route like `/logout` but unusable on a public page
like the landing page, which must render successfully either way. This
feature needs a second, non-rejecting way to answer "is this request
authenticated" purely to decide what the nav bar shows, without duplicating
`TenantContext`'s session-lookup logic.

## 2. Alternatives Evaluated

### Alternative A: `Option<TenantContext>` via a blanket impl
- **Pros:** Would let handlers just take `Option<TenantContext>` directly, no new type name to learn.
- **Cons:** Both `axum::extract::FromRequestParts` and `Option<T>` are foreign to this crate — Rust's orphan rule forbids implementing a foreign trait for a foreign generic type. Not actually implementable without a wrapper.

### Alternative B: Re-parse the session independently in a new extractor
- **Pros:** No changes to the existing `TenantContext` implementation.
- **Cons:** Duplicates the exact session-lookup sequence (`parts.extensions.get::<Session>()`, then `session.get(SESSION_USER_ID_KEY)`) in two places that must stay in sync; a future change to how the session is read (e.g. adding a second session key) would need to be applied twice.

### Alternative C: Factor the session lookup into a shared helper, build both extractors on it
- **Pros:** `TenantContext` and the new `MaybeTenantContext` share one `session_user_id(parts)` helper; `TenantContext` becomes `session_user_id(parts).await?.ok_or(Unauthenticated).map(from_user_id)`, `MaybeTenantContext` maps `None`/`Err` to "not authenticated" instead of rejecting. No duplicated session-parsing logic, and the span-tagging construction (`TenantContext::from_user_id`) is reused by both.
- **Cons:** Requires touching the existing `TenantContext` implementation (a small refactor, not just an addition).

## 3. Structural Decision
We choose **Alternative C**. `src/web/tenancy.rs` gains a private
`async fn session_user_id(parts: &Parts) -> Result<Option<Uuid>, AppWebError>`
and a `TenantContext::from_user_id(Uuid) -> Self` associated function (moving
the existing UUID-wrapping + span-tagging logic there); `TenantContext`'s own
`FromRequestParts` impl becomes a two-line composition of both.
`MaybeTenantContext(pub Option<TenantContext>)` is a new public newtype
wrapping the same lookup, with `Rejection = Infallible` — it cannot fail;
a session-store error while checking auth state degrades to "treat as logged
out" (via `tracing::warn!`, not a panic, not a 500), since a nav-bar
rendering decision must never be the reason a public page fails to load.

Every existing page-template struct (`LandingTemplate`, `SignupTemplate`,
`LoginTemplate`, `WelcomeTemplate`) gains a plain `authenticated: bool` field,
populated by running `MaybeTenantContext` in the corresponding handler
(`landing::show`, `landing::welcome`, `auth::signup_form`, `auth::login_form`).
`templates/base.html`'s nav and `templates/landing.html`'s hero CTA both
branch on this one field. Future protected pages (e.g. the profile page)
already know `authenticated: true` unconditionally, since they're only
reachable via the hard-rejecting `TenantContext` in the first place.

## 4. OpenTelemetry Implications
`MaybeTenantContext`'s extraction reuses `TenantContext::from_user_id`'s
existing span-attribute tagging (`tenant.id`/`user.id` on the active span) for
the authenticated case — no new instrumentation needed, no new PII exposed.
The soft-fail path on a session-store error logs via `tracing::warn!(%error,
...)`, deliberately not including the raw session value itself (already an
opaque session id, not sensitive, but kept minimal on principle). No change
to which routes carry a `TraceLayer` span — this feature adds no new routes.
