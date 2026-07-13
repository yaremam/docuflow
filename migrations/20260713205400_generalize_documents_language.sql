-- Feature 020: replace feature 014's closed en/cyr script-bucket vocabulary with real
-- ISO 639-1 language codes. Validation against the full ISO 639-1 table lives in
-- application code (src/languages.rs, via the isolang crate) rather than as an
-- enumerated CHECK list here — matching TDR 014's original migration-avoidance
-- rationale, more so now that ~180 codes are valid instead of 2. The CHECK below is
-- just a defense-in-depth shape guard (two lowercase letters), not the source of truth.
alter table documents drop constraint documents_language_check;

-- Existing 'cyr' rows predate this migration and don't map to one specific language —
-- clearing them rather than guessing Russian vs. Ukrainian. Feature 013's reprocess-OCR,
-- or a manual re-tag, repopulates a real code.
update documents set language = null where language = 'cyr';

alter table documents add constraint documents_language_check
    check (language is null or language ~ '^[a-z]{2}$');
