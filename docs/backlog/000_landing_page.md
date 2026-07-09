# User Story: Public Landing Page and API Gateway Root

## 1. User Value Statement
As a **Public Visitor**,
I want to **access the root URL of the API gateway (`GET /`)**,
So that **I can see a clean greeting index page confirming the application name, version, and running state.**

## 2. Strict Acceptance Criteria
- **AC-1:** The server responds to a native `GET /` request with a standard JSON string or text landing confirmation.
- **AC-2:** The handler returns a clean `200 OK` HTTP status code.
- **AC-3:** Every request to this endpoint must automatically trigger an OpenTelemetry `tracing::info!` log or span recording the hit, which must be visible inside our local Jaeger UI dashboard.
