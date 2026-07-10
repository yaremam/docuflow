# TDR 000: Endpoint Response and Version Mapping Strategy

> **Revision Note (2026-07-10):** Originally specified `GET /`. The root path was
> reassigned to the HTML marketing landing page in feature 002, so this endpoint
> now lives at `GET /health`.

## 1. Context & Architectural Requirements
We need to determine the most lightweight and observable way to return a health/status response from our Axum server while serving app context metadata.

## 2. Alternatives Evaluated

### Alternative A: Return a Static Raw String Slice (`&'static str`)
- **Pros:** Maximum execution speed, zero memory allocation overhead.
- **Cons:** Rigid and inflexible. We cannot dynamically attach runtime metadata, environment targets, or compile-time cargo package version fields.

### Alternative B: Return a JSON Payload (`axum::Json`) Derived from Cargo Constants
- **Pros:** Returns structural data (`{ "name": "DocuFlow", "version": "0.1.0", "status": "healthy" }`). Allows us to inject compile-time metadata natively using the `env!("CARGO_PKG_VERSION")` macro.
- **Cons:** Marginally higher serialization cost, though completely negligible for our application volume.

## 3. Structural Decision
We choose **Alternative B (JSON response matching Cargo constants)**, now served at
`GET /health`, to guarantee our client integrations can programmatically inspect
the system state and version numbers.

## 4. OpenTelemetry Implications
The handler will be annotated with `#[tracing::instrument]`, mapping the request context directly into our Jaeger canvas.
