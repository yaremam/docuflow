alter table documents
  add column ocr_search tsvector
  generated always as (to_tsvector('simple', coalesce(ocr_text, ''))) stored;

create index documents_ocr_search_idx on documents using gin (ocr_search);
