# DocuFlow: Architecture, Telemetry, and Development Blueprint

## 1. Core Technical Stack
- **Language:** Rust (Latest Stable Edition)
- **Web Runtime:** Axum + Tokio (Asynchronous Engine)
- **Database:** PostgreSQL + SQLx (Strict Compile-Time Verified Queries)
- **Blob Storage:** MinIO / AWS S3 (Streaming Chunked File Uploads)
- **Telemetry Platform:** OpenTelemetry Core + `tracing` Ecosystem
- **Local Exporter:** Jaeger UI running at `http://localhost:16686` (gRPC port `4317`)
- **Version Control:** Jujutsu (`jj`) with Colocated Git Backend (Never use `git add`)

## 2. Core Architecture & Component Rules
- **Multi-User Multi-Tenancy:** 
  - Every incoming HTTP request must extract a `TenantId` and `UserId` via an Axum extractor middleware layer.
  - Every database query and storage query must implicitly pass these IDs to enforce complete data isolation.
  - Inject `tenant.id` and `user.id` into the active OpenTelemetry context using OTel Baggage.
- **OCR Engine Layer:**
  - File parsing must run as decoupled asynchronous workers using Tokio background green threads.
  - Spans must track heavy data transformations. Use `#[tracing::instrument(skip(file_bytes))]` to keep raw byte arrays out of the logging targets.
- **Blob Storage Manager:**
  - Files (bills, insurance documents, contracts) must be securely chunked and pushed to the storage layers using streaming wrappers to prevent high memory spikes.

## 3. Engineering Styles & Coding Standards
- **Zero Panic Safety:** Strict prohibition of `.unwrap()`, `.expect()`, and `panic!()` macros inside production runtime endpoints. All fallible actions must bubble up via context-rich `Result` patterns handled by the `thiserror` crate.
- **Type-Driven Data Constraints:** Use robust specialized types instead of primitives where possible (e.g., `TenantId(Uuid)` instead of raw `String`).
- **Telemetry Boundaries:**
  - Database interactions must be wrapped in structural trace spans tracking round-trip latencies.
  - Sanitize all PII metadata (e.g., specific payment values on bills or name fields on contracts) out of logs and traces to safeguard user confidentiality.

## 4. Test-Driven Development (TDD) Process
- **Red-Green-Refactor Loop:** Every implementation must start with an isolation/integration test inside the `tests/` path defining the behavior.
- **Continuous Integration Flow:** Production changes are valid only when `cargo test` and `cargo check` compile successfully.
- **AI Agent Split:**
  - `claude` -> Deep architectural refactoring, async compiler bug remediation, lifetimes, and macro definitions.
  - `agy` (Google Antigravity, not yet adopted — reserved for future use) -> Heavy CRUD boilerplate generation, raw SQL schema data layouts, and package tracking.

## 5. UI Design Process
- **Mockup Before Implementation:** Any new UI screen or user-facing feature must start with a mockup (an Artifact) before any template/handler code is written. Minor tweaks to existing screens (copy edits, color/spacing fixes) don't require this.
- Mockups must reuse the approved "ledger and stamp" visual identity (tokens in `static/style.css`: navy ink, slate paper, forest-green stamp accent; Roboto Slab/Fira Sans/Fira Mono) rather than introducing a new design direction.
- Get explicit sign-off on the mockup before implementing.
