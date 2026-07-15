# TDR 021: Nightly Image Shipping (GHCR pipeline + self-host deploy)

## 1. Context & Architectural Requirements
Through feature 020, "running DocuFlow" has meant cloning the repo and
`docker compose up -d --build` — fine for the dev loop, but there was no
way to *use* the app on real hardware without the source tree. The user's
explicit direction (2026-07-15): ship a **nightly image on a Docker
registry**, with the first (and so far only) deploy target being a
Synology DS920+ NAS — Intel Celeron J4125, i.e. `linux/amd64` with **no
AVX instructions**, 4 GB RAM, DSM Container Manager running compose
projects.

Two latent assumptions in the codebase blocked a pulled image from being
usable on a host without the dev stack around it:
- `main.rs` silently defaulted `OTLP_ENDPOINT` to `localhost:4317` — a
  NAS deployment has no collector there, so the batch exporter would
  retry against a dead endpoint forever.
- The OTel layer was the **only** `tracing` layer, so a container without
  Jaeger produced no logs at all — `docker logs` on the NAS would have
  been empty, which is untenable for the only diagnostic channel a pulled
  image has.

Per CLAUDE.md: publishing is gated on `cargo check` + `cargo test`
(the CI rule, until now enforced only by local discipline), zero-panic
runtime code, and no PII in whatever the new stdout log layer emits (it
sees the same already-sanitized events/spans the OTel layer does).

## 2. Alternatives Evaluated

### Alternative A: Docker Hub instead of GHCR
- **Pros:** Better unauthenticated discoverability; the default registry
  when users write bare image names.
- **Cons:** Needs a separately managed access token as a repo secret
  (GHCR uses the workflow's own `GITHUB_TOKEN`), anonymous pulls are
  rate-limited, and the package page lives away from the repo. For a
  personal project whose repo is already public on GitHub, every
  operational property favors GHCR.

### Alternative B: No registry — build on the NAS itself (git pull + compose build)
- **Pros:** No CI to maintain; nothing published.
- **Cons:** A release-mode Rust compile on a 4 GB J4125 takes ~an hour if
  it doesn't OOM outright; every "update" is a source checkout; nothing
  gates a broken commit from becoming the running build. Rejected — this
  is exactly the "clone the repo to use the app" status quo with extra
  steps.

### Alternative C (chosen): GHCR nightly, gated on the full test suite, with a label-based skip check
- **Pros:** One workflow owns test + publish, so an image tag existing
  *implies* its commit passed `cargo check`/`cargo test` (CLAUDE.md's CI
  rule, now enforced by machinery instead of discipline). The
  already-published image itself records what it was built from
  (`org.opencontainers.image.revision` label = `GIT_SHA` build arg =
  `/health`'s new `revision` field), so "did main move since last
  night?" needs no extra state — the check reads the label back off the
  registry and skips the run if it matches `HEAD`, treating any inspect
  failure (including the not-yet-published first run) as "build".
- **Cons:** Nightly cadence means a bad merge is live for up to a day —
  accepted: pinned `nightly-YYYY-MM-DD` tags exist precisely so the NAS
  can roll back one tag while the fix lands.

## 3. Structural Decision
We choose **Alternative C**, with these sub-decisions:

**Telemetry becomes opt-in, stdout logging unconditional**
(`src/telemetry.rs`, `src/main.rs`): `init_telemetry` now takes
`Option<&str>`; `None` (unset or empty `OTLP_ENDPOINT`) installs only the
`fmt` stdout layer, `Some` adds the OTLP layer exactly as before. The
silent `localhost:4317` fallback is removed from `main.rs` — dev setups
keep their Jaeger export because `.cargo/config.toml`, `.env`, and
`docker-compose.yml` all set `OTLP_ENDPOINT` explicitly now. The
alternative (keeping the fallback plus a disable flag) was rejected:
"absence of config" is the state a fresh pull runs in, so absence has to
be the safe mode, not the misconfigured one.

**Revision as a runtime env var, not a compile-time constant**
(`Dockerfile`, `src/web/handlers/health.rs`): the pipeline passes
`GIT_SHA` as a build arg that becomes a runtime `ENV` and two OCI labels,
declared *after* the compile/apt layers so the every-night-different value
never busts them. `/health` reads it per-request with a `"dev"` fallback.
Compile-time embedding (`option_env!`) was rejected: it invalidates the
final crate's build cache for zero benefit, and `"dev"` cleanly marks any
binary that didn't come from the pipeline.

**Dependency-layer caching via `cargo-chef`** (`Dockerfile`): planner/
builder stages split dependency compilation from app compilation, backed
by the GHA buildx cache (`type=gha,mode=max`), so a nightly rebuild
recompiles the app crate only. Installed from crates.io in our own stage
rather than trusting the third-party `lukemathwalker/cargo-chef` image.

**CI services mirror the dev stack** (`.github/workflows/nightly.yml`):
Postgres and Mailpit as service containers; MinIO via a `docker run` step
(its official image needs the `server /data` command, which `services:`
can't express — same image as dev rather than a differently-behaving
substitute). The committed `.cargo/config.toml` supplies the same env the
tests use locally; `SQLX_OFFLINE=true` keeps the sqlx macros on the
checked-in cache. `linux/amd64` only, and never `-C target-cpu=native`
(CI chips have AVX; the J4125 doesn't).

**The user-facing artifact is `deploy/docker-compose.yml`**: pulls
`:nightly`, no `build:`, Postgres/MinIO/Mailpit alongside, named volumes
(DSM bind-mount variants shown in comments), every host port and
credential overridable via `deploy/.env`. `APP_BASE_URL` and
`BLOB_PUBLIC_ENDPOINT_URL` are the loudest thing in the file — same
LAN-reachability constraint the dev `docker-compose.override.yml`
documents (QR scan + presigned URLs are opened by *other* devices).
Mailpit ships as a placeholder mail-catcher; real SMTP is documented,
not automated (backlog 021 §3).

## 4. OpenTelemetry Implications
The `fmt` layer subscribes to the identical, already-PII-sanitized event
stream the OTel layer exports — `#[instrument(skip(...))]` decisions and
redacted `Debug` impls apply to both, so stdout adds no new PII surface.
With `OTLP_ENDPOINT` unset, no exporter, batch processor, or gRPC channel
is ever constructed (not merely pointed at a dead endpoint), so the NAS
runs with zero export overhead. Dev behavior is unchanged except that
`cargo run` now *also* prints events to the terminal, which every prior
feature's "check Jaeger" verification treated as a missing nicety.

## 5. Test Strategy
- `tests/telemetry_disabled.rs` — deliberately its **own test binary**:
  the global subscriber can only install once per process, so the
  `None`-endpoint init can't share a binary with
  `tests/telemetry_bootstrap.rs`'s `Some`-endpoint init.
- `tests/health_check.rs` — asserts the new `revision` field reports
  `"dev"` outside the image build.
- The workflow itself is validated by `actionlint` and by its first real
  runs (verified 2026-07-15: local `docker build` of the reworked
  Dockerfile, plus booting the built image with no `OTLP_ENDPOINT` — see
  feature 021's verification notes); it cannot be exercised by `cargo
  test`.
