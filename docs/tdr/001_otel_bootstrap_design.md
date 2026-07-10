# TDR 001: OpenTelemetry Tracing Bootstrap Strategy

## 1. Context & Architectural Requirements
Every architectural rule in CLAUDE.md — tenant/user baggage propagation, DB round-trip spans, OCR worker instrumentation, PII-scrubbed telemetry — assumes a tracer is already running. Right now `src/main.rs` is a bare `println!` with no subscriber configured, so none of that instrumentation has anywhere to send spans. We need the bootstrap wired before the first Axum handler is written, so every endpoint added afterward is observable by default rather than retrofitted.

## 2. Alternatives Evaluated

### Alternative A: `tracing_subscriber::fmt` only (stdout logging, no OTel)
- **Pros:** Zero external dependencies at runtime, works before Jaeger is running, simplest possible setup.
- **Cons:** Produces plain log lines with no span hierarchy, no trace IDs correlating requests across services, and no visibility in the Jaeger UI — fails AC-3 outright and contradicts the CLAUDE.md telemetry platform choice.

### Alternative B: `tracing-opentelemetry` layer exporting via OTLP/gRPC to the local Jaeger collector
- **Pros:** Matches the stack already declared in `Cargo.toml` (`tracing-opentelemetry`, `opentelemetry-otlp` with the `grpc-tonic` feature) and the collector already running in `docker-compose.yml` on port `4317`. Gives full span trees, trace IDs, and baggage propagation needed for the multi-tenancy rule in CLAUDE.md section 2.
- **Cons:** Requires the Jaeger container to be up for spans to export successfully in local dev; adds async shutdown/flush handling to avoid dropping spans on exit.

## 3. Structural Decision
We choose **Alternative B**. It's the only option that satisfies the acceptance criteria and matches infrastructure and dependencies already committed to this repo. The subscriber will be built as a `Registry` with an `EnvFilter` layer (for local log-level control) and an `OpenTelemetryLayer` wrapping a `TracerProvider` configured with the OTLP gRPC exporter pointed at `http://localhost:4317`.

## 4. OpenTelemetry Implications
This TDR *is* the OpenTelemetry implementation, so it directly enables every downstream telemetry rule in CLAUDE.md: tenant/user OTel Baggage injection (section 2), `#[tracing::instrument(skip(file_bytes))]` on OCR workers (section 2), and structural DB round-trip spans with PII sanitized out of span attributes (section 3). The bootstrap itself emits one startup span/log line with no PII, and registers a shutdown hook that flushes the `TracerProvider` before process exit so in-flight spans reach Jaeger.
