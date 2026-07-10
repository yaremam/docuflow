# User Story: JSON API Health/Status Endpoint

> **Revision Note (2026-07-10):** Originally specified `GET /`. The root path was
> reassigned to the HTML marketing landing page in feature 002, so this endpoint
> now lives at `GET /health`.

## 1. User Value Statement
As a **Public Visitor / Monitoring System**,
I want to **access a health/status endpoint of the API gateway (`GET /health`)**,
So that **I can confirm the application name, version, and running state.**

## 2. Strict Acceptance Criteria
- **AC-1:** The server responds to a native `GET /health` request with a standard JSON string or text landing confirmation.
- **AC-2:** The handler returns a clean `200 OK` HTTP status code.
- **AC-3:** Every request to this endpoint must automatically trigger an OpenTelemetry `tracing::info!` log or span recording the hit, which must be visible inside our local Jaeger UI dashboard.
