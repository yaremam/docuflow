# User Story: Multi-Page Phone Scan

## 1. User Value Statement
As a **DocuFlow user scanning a document that's longer than one page
(a contract, a multi-page bill) with my phone**,
I want to **capture all its pages in one scan session and get one single
document out of it**,
So that **a five-page contract is one entry in my archive with all five
pages OCR'd, instead of five separate one-page documents I'd have to
create through five separate QR codes.**

## 2. Strict Acceptance Criteria
- **AC-1:** One scan session accepts multiple page captures. After each
  uploaded photo the phone shows a "page added" state with the running
  page count and two actions: **"Add another page"** (returns to the
  capture form) and **"Finish — create document"**. Feature 009's
  capture-once-and-done phone flow becomes capture-repeat-finish.
- **AC-2:** Finishing produces exactly **one** document: the session's
  pages combined, in capture order, into a single PDF that flows through
  the existing ingest path (blob storage → `documents` row → detached
  OCR spawn). Feature 010's PDF OCR already rasterizes per page, so the
  document's `ocr_text` covers every page with no OCR-pipeline changes.
- **AC-3:** A single-page scan still works end-to-end: capture one page,
  tap Finish. (One deliberate extra tap compared to feature 009's
  auto-finalize — the cost of making "more pages?" an explicit question
  instead of a guess.)
- **AC-4:** The desktop `GET /scan` page shows live capture progress
  ("N page(s) received so far…") while the phone works, using the
  existing `<meta http-equiv="refresh">` polling idiom — no JS — and
  still auto-redirects to `/documents/{id}?uploaded=true` once the phone
  finishes.
- **AC-5:** Capturing a page extends the session's `expires_at` by the
  full TTL (sliding expiry) — a slow multi-page scan can't die of the
  10-minute fuse lit at QR-mint time. An expired-mid-session token shows
  the phone's existing "code isn't valid anymore" state; already-captured
  pages of an expired session never become a document.
- **AC-6:** Session state distinguishes "no pages yet" (`pending`),
  "capturing" (≥1 page, not finished), and "captured" (finalized,
  `document_id` set — unchanged meaning from 009). Token
  invalid/expired handling on both phone and desktop is otherwise
  unchanged.
- **AC-7:** Concurrent/duplicate finish submits can't create two
  documents from one session (same guarded-update discipline as 009's
  `status = 'pending'` re-check).
- **AC-8:** No `.unwrap()`/`.expect()`/`panic!()` in runtime code; raw
  page bytes stay out of spans (`skip(...)`, matching `submit_scan`);
  phone-side tenancy still resolves from the `scan_sessions` row (TDR
  009 §3's documented exception). Mockup signed off before any
  template/handler code, per CLAUDE.md §5.

## 3. Explicitly out of scope this round
- **Reordering, retaking, or deleting individual pages on the phone.**
  Pages land in capture order, full stop; a botched page means
  finish-and-delete-the-document or abandon the session and rescan.
  Worth revisiting only if it actually stings in practice.
- **Mixing desktop-uploaded files into a scan session** — sessions
  remain phone-only, as in 009.
- **A background sweep for abandoned sessions' page blobs.** An
  abandoned session's already-uploaded page blobs are small, personal-
  scale orphans; cleanup stays opportunistic/deferred rather than a new
  scheduled-job subsystem. Revisit if blob storage growth ever shows up
  as a real cost.
- **Combining pages into anything other than PDF** (multi-image
  documents, TIFF stacks) — one PDF per session keeps the document
  model and OCR pipeline untouched.
