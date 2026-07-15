-- Feature 022 (multi-page phone scan): one scan session now accumulates
-- N captured pages before being finalized into a single PDF document.

-- 'capturing' = at least one page uploaded, not yet finished. 'pending'
-- and 'captured' keep their feature-009 meanings (no pages yet / finalized
-- with document_id set).
alter table scan_sessions drop constraint scan_sessions_status_check;
alter table scan_sessions add constraint scan_sessions_status_check
    check (status in ('pending', 'capturing', 'captured'));

create table scan_pages (
    id uuid primary key,
    scan_session_id uuid not null references scan_sessions(id) on delete cascade,
    page_number int not null,
    blob_key text not null,
    content_type text not null,
    file_size_bytes bigint not null,
    created_at timestamptz not null default now(),
    unique (scan_session_id, page_number)
);
