# User Story: Bulk Actions on the Dashboard

## 1. User Value Statement
As a **logged-in DocuFlow user with several documents to tag, delete, or
re-run OCR on**,
I want to **select multiple documents on the dashboard at once and apply
an action to all of them**,
So that **I don't have to open each document individually to tag it,
delete it, or reprocess its OCR.**

## 2. Strict Acceptance Criteria
- **AC-1:** Each dashboard row has a checkbox; checking one or more shows
  a bulk-action toolbar with the number of documents currently selected.
- **AC-2:** "Delete" renders a confirm page listing every selected
  document's title and filename before anything is deleted — same
  confirm-before-destroy precedent as deleting a single document.
- **AC-3:** Confirming bulk delete removes every selected document (and
  best-effort deletes its blob and generated thumbnail) and redirects
  back to the dashboard with whatever filters/sort were active before the
  action, not a reset unfiltered view.
- **AC-4:** "Add tag" applies the typed tag(s) to every selected document
  without duplicating a tag a document already has, and needs no
  confirmation step.
- **AC-5:** "Reprocess OCR" re-queues every selected document whose
  `ocr_status` is `done`/`failed`/`skipped`; a document already `pending`
  or `processing` is left untouched (not re-queued a second time) —
  closes the bulk-reprocess gap named in ARCHITECTURE.md §8.
- **AC-6:** Every bulk action is tenant-scoped: a document id that
  doesn't belong to the acting tenant is silently excluded from the
  action, never affecting another tenant's data and never raising an
  error for the request as a whole.
- **AC-7:** The bulk toolbar's buttons live inside the dashboard's single
  existing filters `<form>`, never a second nested `<form>` — regression-
  tested given this project's documented nested-`<form>` bug (found
  2026-07-13, `tests/documents_date_suggestion.rs`).
- **AC-8:** No `.unwrap()`, `.expect()`, or `panic!()` introduced.
- **AC-9:** No new PII in spans/logs.

## 3. Explicitly out of scope this round
- **Bulk untag / bulk edit of other metadata** (date issued, language,
  document type) — only tag-add, delete, and reprocess ship this round.
- **A "select all" control** — a user selects rows individually; no
  select-all-matching-filter shortcut.
- **Undo for bulk delete** — deletion is immediate and permanent once
  confirmed, same as single-document delete; no soft-delete/trash (a
  separate, already-parked backlog item).
