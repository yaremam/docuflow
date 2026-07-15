# User Story: Nightly Image Shipping (GHCR pipeline + self-host deploy)

## 1. User Value Statement
As a **DocuFlow user who wants to run the app on my own hardware (first
target: a Synology DS920+ NAS)**,
I want to **pull a prebuilt, tested nightly Docker image from a public
registry and start the whole stack with one compose file**,
So that **I can actually use DocuFlow day-to-day without cloning the repo
or installing a Rust toolchain, and always know exactly which build I'm
running.**

## 2. Strict Acceptance Criteria
- **AC-1:** A GitHub Actions workflow builds and pushes
  `ghcr.io/yaremam/docuflow` nightly (03:00 UTC cron) and on manual
  dispatch, tagged `nightly` (moving), `nightly-YYYY-MM-DD` (pinned, for
  rollback), and `sha-<short>` — `latest` stays unused, reserved for
  future real releases.
- **AC-2:** The workflow publishes nothing unless `cargo check` and the
  full `cargo test` suite pass on the same commit (CLAUDE.md's CI rule),
  with Postgres + MinIO provided as CI services and the tesseract
  language packs + poppler installed on the runner. Tests use
  `doc_manager_db_test` / `docuflow-uploads-test` exactly as locally.
- **AC-3:** A nightly run exits early (skipped, not failed) when `main`
  has no new commits since the last published `nightly` image — compared
  via the image's `org.opencontainers.image.revision` label. The very
  first run (no image published yet) builds instead of erroring.
- **AC-4:** Images are `linux/amd64` only — sufficient for the DS920+'s
  Celeron J4125. The build must never pass `-C target-cpu=native` (CI
  chips have AVX; the J4125 does not).
- **AC-5:** With `OTLP_ENDPOINT` unset, the app runs with **no OTel
  exporter at all** — no background export attempts to a nonexistent
  collector — while still logging to stdout so `docker logs` is useful.
  With `OTLP_ENDPOINT` set, spans export exactly as today. Local dev
  keeps exporting to Jaeger by default (dev env files set the endpoint
  explicitly; the silent `localhost:4317` fallback in `main.rs` is
  removed).
- **AC-6:** Structured stdout logging is always on, regardless of
  telemetry configuration — today the only `tracing` layer is the OTel
  exporter, so a container without Jaeger logs nothing at all.
- **AC-7:** `GET /health` reports the git revision the image was built
  from (new `revision` field; `"dev"` when not built via the pipeline),
  so a user can say exactly which nightly is misbehaving. The Dockerfile
  bakes the sha in via build arg and also stamps
  `org.opencontainers.image.revision` / `.source` labels (which AC-3's
  skip-check reads back).
- **AC-8:** A `deploy/docker-compose.yml` exists whose `app` service
  pulls the GHCR image (no `build:`), alongside Postgres, MinIO, and
  Mailpit with named volumes. Host ports and data paths are overridable,
  and `APP_BASE_URL` / `BLOB_PUBLIC_ENDPOINT_URL` are surfaced
  prominently with comments explaining the LAN-reachability requirement
  (QR scan flow + presigned blob URLs), since on a NAS these must be the
  NAS's LAN address, not `localhost`.
- **AC-9:** No `.unwrap()`/`.expect()`/`panic!()` introduced in runtime
  code, per CLAUDE.md's zero-panic rule; telemetry/health changes are
  test-covered first (TDD) in `tests/telemetry_bootstrap.rs` and
  `tests/health_check.rs`.

## 3. Explicitly out of scope this round
- **arm64 / multi-arch images.** The only known deploy target is amd64;
  QEMU-emulated Rust builds are painfully slow. Revisit if a real arm64
  user appears.
- **Auto-updating the NAS deployment** (Watchtower or DSM re-pull
  automation). The deploy compose pulls `nightly` on `docker compose
  pull`; whether updates happen automatically is the operator's call.
- **A `latest` tag / versioned releases.** Nightly-only until the
  project wants release discipline.
- **Real-provider SMTP setup.** The deploy compose ships Mailpit as a
  placeholder mail-catcher (reset emails viewable in its UI, not
  delivered); pointing `SMTP_*` at a real provider is documented but not
  automated.
- **Serving HTTPS / reverse-proxy config.** DSM's built-in reverse proxy
  (or any other) can front the app; not part of this feature.
