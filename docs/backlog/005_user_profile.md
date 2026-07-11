# User Story: User Profile

## 1. User Value Statement
As a **logged-in DocuFlow user**,
I want to **view and edit my profile — name, address, phone number, and a profile picture**,
So that **my account carries the personal details relevant to my documents (bills, insurance, contracts often reference my name and address), and I can keep them current.**

## 2. Strict Acceptance Criteria
- **AC-1:** `GET /profile` requires an authenticated session; an unauthenticated request is redirected to `/login`, never rendering the page.
- **AC-2:** `GET /profile` renders the current values of first name, last name, street address, city, postcode, country, and phone number — blank for any field the user hasn't set yet.
- **AC-3:** `POST /profile` updates all seven fields in one request; a field submitted blank clears that field (stored as `NULL`, not an empty string). Values persist and are visible on the next `GET /profile`.
- **AC-4:** `POST /profile/picture` accepts a single image file upload (`multipart/form-data`), streams it to blob storage without buffering the whole file in memory, and updates the profile to reference it. Uploads with a non-image content type are rejected with `400`. Uploads over 8MB are rejected.
- **AC-5:** After a successful picture upload, `GET /profile` renders the picture (via a short-lived signed URL) rather than a placeholder.
- **AC-6:** Every request to `/profile` and `/profile/picture` emits a trace span; no profile field value (name, address, phone) or raw file bytes ever appear as a span attribute or log field.
- **AC-7:** No `.unwrap()`, `.expect()`, or `panic!()` in the new handler/blob-storage code; a storage or database failure surfaces as a `Result`/`thiserror` error mapped to a proper HTTP response, never a panic or a silently truncated upload.
