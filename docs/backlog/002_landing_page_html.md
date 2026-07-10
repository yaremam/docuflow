# User Story: HTML Landing Page with Signup/Login (UI Only)

## 1. User Value Statement
As a **Public Visitor**,
I want to **land on a rendered HTML page at `GET /` that introduces DocuFlow and lets me reach signup and login pages**,
So that **I can understand what the product does and start the account-creation flow, on both desktop and mobile devices.**

## 2. Strict Acceptance Criteria
- **AC-1:** `GET /` returns `200 OK` with `Content-Type: text/html`, rendering a server-side Askama template (not a static file) containing product intro copy and CTA links to `/signup` and `/login`.
- **AC-2:** `GET /signup` and `GET /login` each return `200 OK` HTML pages containing a `<form>` with `email` and `password` fields, posting to `/signup` and `/login` respectively.
- **AC-3:** `POST /signup` and `POST /login` accept form-encoded submissions matching their respective forms, do NOT persist any data or perform password hashing, and return an explicit "not yet implemented" placeholder response (HTTP 501) rendering a friendly confirmation page — not a raw error, not a 404, not a silent no-op.
- **AC-4:** The layout is responsive: forms and navigation adapt correctly at both a mobile viewport (~375px) and a desktop viewport (~1280px), matching the approved visual mockup produced before implementation.
- **AC-5:** The pre-existing JSON status/health endpoint (previously specified at `GET /` in backlog 000) is relocated to `GET /health` to free up `GET /` for this page; 000's docs are amended accordingly.
- **AC-6:** Every request to `/`, `/signup`, `/login` (GET and POST) emits a trace span visible in the local Jaeger UI, per the OTel bootstrap already in place (001); no email/password values ever appear in span attributes or logs.
- **AC-7:** No `.unwrap()`, `.expect()`, or `panic!()` in the new handler/template/form code; malformed form submissions surface as a `400`-class response via `Result`/`thiserror`, never a panic.
