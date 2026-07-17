# User Story: Expiry Dates & Renewal Reminders

## 1. User Value Statement
As a **logged-in DocuFlow user with insurance policies, contracts,
subscriptions, or ID documents on file**,
I want to **see at a glance when one of them is expiring or has already
expired**,
So that **I don't miss a renewal or let an ID lapse because the paperwork
was filed away and forgotten.**

## 2. Strict Acceptance Criteria
- **AC-1:** A `date_expires` field is available on the metadata form,
  editable the same way `date_issued` already is — but only when the
  document's **confirmed** `doc_type` is `insurance`, `contract`,
  `bill`, or `id`. An unconfirmed `ocr_suggested_doc_type` of one of
  those values does not surface the field.
- **AC-2:** An OCR-based suggestion (`ocr_suggested_date_expires`) is
  computed the same "suggest, then explicit accept" way as
  `ocr_suggested_date_issued` (feature 012) — a wrong guess never lands
  in `date_expires` silently.
- **AC-3:** Unlike `date_issued`'s extraction (which accepts any
  date-shaped text with no keyword requirement), the expiry suggestion
  requires a nearby trigger phrase ("expires"/"expiry date"/"expiration
  date"/"valid until" in English, and their German/Dutch/Ukrainian
  equivalents) before accepting a candidate date — otherwise it would
  just re-find whatever date `date_issued`'s extraction already found.
- **AC-4:** The `/documents` dashboard shows a strip listing documents
  that are expired or expiring within **14 days**, already-expired ones
  included (not dropped once the date passes) — computed live on every
  page render, no background job, no email. Absent entirely when
  nothing qualifies.
- **AC-5:** A new "Expiry status" smart filter offers four OR-combined
  checkboxes: **Expired**, **Expiring soon** (within 14 days), **Later**,
  and **No expiry set** — the last scoped only to eligible-doc_type
  documents missing `date_expires`, not every document in the tenant.
- **AC-6:** No `.unwrap()`, `.expect()`, or `panic!()` introduced.
- **AC-7:** No new PII in spans/logs — `date_expires`/the OCR text it's
  matched against follow the same rule already applied to `ocr_text`/
  `date_issued`.

## 3. Explicitly out of scope this round
- **Email (or any other channel) reminders.** The strip is the entire
  notification mechanism this round — designed so the same "what's
  expiring" query can be reused if email/push notifications get built
  later, but no scheduler, no `Mailer` changes, no new dependency this
  round (see TDR 031 §2 Alternative B).
- **A full year/month calendar breakdown facet for `date_expires`**
  (mirroring `date_issued`'s) — the status-bucket shape (Expired/
  Expiring soon/Later/No expiry set) was chosen instead; see TDR 031 §2.
- **A configurable "expiring soon" threshold** — fixed at 14 days this
  round, not a per-user setting.
- **Extending the trigger-phrase/keyword-anchoring approach to
  `date_issued`** — that extraction keeps its existing no-keyword
  behavior unchanged; only the new expiry extraction is keyword-anchored.
