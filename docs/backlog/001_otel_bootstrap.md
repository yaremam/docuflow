# User Story: OpenTelemetry Tracing Bootstrap

## 1. User Value Statement
As a **Platform Operator**,
I want to **have every server process initialize an OpenTelemetry tracer and export spans to the local Jaeger collector on startup**,
So that **all subsequent request handling, database calls, and background workers are observable from the very first endpoint, without retrofitting instrumentation later.**

## 2. Strict Acceptance Criteria
- **AC-1:** On process start, the application configures a `tracing_subscriber` registry combining an `EnvFilter` layer and an OpenTelemetry layer, before any request-handling code runs.
- **AC-2:** Spans are exported via OTLP over gRPC to `http://localhost:4317`, matching the Jaeger collector defined in `docker-compose.yml`.
- **AC-3:** A minimal `tracing::info!` emitted at startup (e.g., "server booting") is visible as a trace in the Jaeger UI at `http://localhost:16686` within a few seconds of startup.
- **AC-4:** The tracer provider is flushed and shut down cleanly on process exit so no spans are lost or hang the process.
- **AC-5:** No `.unwrap()`, `.expect()`, or `panic!()` is used in the bootstrap path; failures to initialize telemetry surface as a `Result` handled via `thiserror`, per CLAUDE.md's Zero Panic Safety rule.
