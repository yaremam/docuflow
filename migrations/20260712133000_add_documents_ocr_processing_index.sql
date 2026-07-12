-- Keeps state::migrate's boot-time "reset stuck OCR jobs" sweep
-- (`update documents set ocr_status = 'pending' where ocr_status =
-- 'processing'`) a cheap index scan instead of a full-table scan on every
-- boot, regardless of how large `documents` grows.
create index documents_ocr_processing_idx on documents (id) where ocr_status = 'processing';
