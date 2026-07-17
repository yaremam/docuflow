# User Story: Duplicate Detection

## 1. User Value Statement
As a **logged-in DocuFlow user uploading a document**,
I want to **be told if I've already uploaded this exact file before**,
So that **I don't end up with silent, wasted duplicate copies cluttering
my dashboard without realizing it.**

## 2. Strict Acceptance Criteria
- **AC-1:** Uploading a file whose exact byte content matches an
  already-uploaded document (same tenant) still creates the new
  document — nothing is ever blocked or rejected because of a match.
- **AC-2:** The new document's page shows a one-time notice — "You
  already uploaded a file with this exact content on {date} — see
  {link}" — the first time it's viewed after upload, and never again on
  a later visit.
- **AC-3:** The notice links to the **oldest** matching document, when
  more than one earlier match exists.
- **AC-4:** A file whose content doesn't match anything else in the same
  tenant uploads with no notice at all — behavior is unchanged from
  today.
- **AC-5:** Matching is scoped to the uploading user's own tenant —
  never compares against, or reveals the existence of, another tenant's
  documents (CLAUDE.md's tenancy-isolation rule).
- **AC-6:** Both ingestion paths — the desktop upload form and the
  phone-camera scan handoff (feature 009/022) — get the same detection,
  since both funnel through the same shared insert path.
- **AC-7:** A document uploaded before this feature shipped has no
  recorded content hash and can't be matched against until it's
  reprocessed (feature 013's existing "Reprocess OCR" button) — no bulk
  backfill sweep.
- **AC-8:** No `.unwrap()`, `.expect()`, or `panic!()` introduced.
- **AC-9:** No new PII in spans/logs — a document's content hash is a
  content fingerprint, not the content itself, but it's still kept out
  of tracing the same way `ocr_text` already is.

## 3. Explicitly out of scope this round
- **Near-duplicate / perceptual matching** (e.g. two photos of the same
  physical page taken moments apart) — this is exact-byte-content
  matching only, per the backlog's original phrasing ("hash file
  bytes").
- **Blocking or rejecting a duplicate upload** — considered and
  rejected; see TDR 029 §2.
- **A persistent duplicate marker** visible on a later visit or in the
  dashboard list — the notice is one-shot, shown only immediately after
  upload.
- **Bulk backfilling `content_hash`** for every pre-existing document —
  only reprocessing a document computes/refreshes its hash.
