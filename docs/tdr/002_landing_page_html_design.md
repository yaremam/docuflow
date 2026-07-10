# TDR 002: HTML Landing Page Rendering & Stubbed Auth UI

## 1. Context & Architectural Requirements
This is the first feature to serve real HTML and the first to stand up an actual Axum `Router` bound to a TCP listener — `src/main.rs` currently only bootstraps telemetry and exits. It also collides with backlog item 000, which reserved `GET /` for a JSON health/status payload; that endpoint is relocated to `GET /health` (000 amended) so this feature can own the root path for a human-facing page. Signup and login forms are required, fully responsive, but must not persist anything — no password hashing, no user table, no sessions — that is explicitly deferred to a follow-up auth-persistence feature.

## 2. Alternatives Evaluated

### Alternative A: Static HTML/CSS/JS served via `tower-http`'s `ServeDir`/`fs` layer
- **Pros:** Zero templating dependency, trivially cacheable, simplest possible serving model.
- **Cons:** No server-side dynamic content (can't inject compile-time version info, can't vary content per request), and every future dynamic need (flash messages, per-tenant branding, form validation echoing) would require bolting on a templating engine anyway or hand-writing string interpolation — the exact `&'static str` rigidity problem TDR 000 already rejected for the JSON endpoint.

### Alternative B: Separate SPA frontend (e.g., a JS/TS build served independently or via a reverse proxy)
- **Pros:** Rich client-side interactivity, clean separation of frontend/backend concerns, easier to hand off to a dedicated frontend stack later.
- **Cons:** Introduces an entire second toolchain (bundler, package manager, build step) the project has zero infrastructure for today; massively over-scoped for a landing page plus two stub forms; contradicts the explicit no-JS-framework decision for this pass.

### Alternative C: Server-rendered HTML via Askama compile-time-checked templates + plain CSS
- **Pros:** Templates are checked against their data structs at compile time (a malformed `{% block %}` or missing field is a build error, not a runtime 500) — this is a strong fit for the Zero Panic Safety philosophy already established for the OTel bootstrap. No JS build pipeline. Template inheritance (`base.html` + child blocks) keeps nav/CSS/head markup DRY across landing/signup/login. Handlers stay Rust-native, no separate process/deployment artifact.
- **Cons:** Presentation logic lives in `.html` template files rather than pure Rust, and any future rich client-side interactivity would need a different mechanism (htmx, a JS sprinkle, or a later SPA migration) layered on top.

## 3. Structural Decision
We choose **Alternative C**. Askama's compile-time template checking aligns with the project's zero-panic ethos better than either static files (no dynamism) or a full SPA (unjustified scope for a landing page). Static file serving (Alternative A's mechanism) is still used, narrowly, for the one shared CSS asset via `tower-http`'s `fs` feature — this is a targeted use of Alternative A's technique for a non-dynamic asset, not a rejection of it. The integration crate is `askama_web` (feature `axum-0.7`, matching the pinned `axum = "0.7"` / `axum-core 0.4.5`), which derives `IntoResponse` for template structs directly, avoiding hand-written `Html::from(tpl.render()?)` boilerplate in every handler.

Signup/login forms are wired to real `POST` routes now (not dead links) but their handlers are intentionally stubbed: they parse and validate structurally (via `EmailAddress`/`Password` newtypes, satisfying CLAUDE.md's type-driven-constraints rule from day one) but perform no hashing, no DB write, no session issuance, and respond `501 Not Implemented` with a friendly rendered confirmation. This is a deliberate placeholder, tracked as the seam for a dedicated follow-up "auth persistence" feature (argon2 hashing, a `users` table, `tower-sessions` or `axum-login`), none of which are in this feature's scope or its Cargo.toml.

## 4. OpenTelemetry Implications
Every route (`/`, `/health`, `/signup`, `/login` GET+POST) gets `#[tracing::instrument]`; the two POST handlers use `#[tracing::instrument(skip(form))]` so the `EmailAddress`/`Password` field values are never captured as span attributes — this is a direct application of CLAUDE.md's PII-sanitization rule, applied preemptively even though no real user data is persisted yet. The `TraceLayer` from `tower-http` is added to the `Router` so every request (including static asset hits) gets an HTTP-level span for free, visible in Jaeger exactly as the 001 bootstrap intends. No `TenantId`/`UserId` baggage extraction applies yet — there is no authenticated session in this pass — but the router structure (a single `app()` builder function) is deliberately shaped so a tenant/user-extraction `Router::layer` can be added later without restructuring the route table.
