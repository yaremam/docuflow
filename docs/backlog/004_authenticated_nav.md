# User Story: Auth-Aware Navigation

## 1. User Value Statement
As a **logged-in DocuFlow user**,
I want to **see a link to my profile instead of "Log in"/"Sign up" once I'm authenticated**,
So that **the site reflects that I'm already signed in, instead of inviting me to register or log in again.**

## 2. Strict Acceptance Criteria
- **AC-1:** On every page, when the request carries no valid session, the nav bar shows "Log in" and "Sign up" links exactly as today.
- **AC-2:** On every page, when the request carries a valid session, the nav bar shows a single "Profile" link (to `/profile`) instead of "Log in"/"Sign up".
- **AC-3:** The landing page's hero call-to-action buttons follow the same rule: "Sign up free"/"Log in" when logged out, a single "Go to your profile" action when logged in.
- **AC-4:** This check must never reject or error a public page — a session-store failure while determining auth state degrades to "render as logged out" (logged server-side), not a 500.
- **AC-5:** Every request to `/`, `/welcome`, `/signup`, `/login` continues to emit a trace span as before; the auth-state check introduces no new PII in logs/traces.
- **AC-6:** No `.unwrap()`, `.expect()`, or `panic!()` in the new extractor code; a malformed or missing session value surfaces as "not authenticated", never a panic.
