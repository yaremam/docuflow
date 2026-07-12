create table documents (
    id uuid primary key,
    tenant_id uuid not null references tenants(id),
    user_id uuid not null references users(id),
    original_filename text not null,
    title text,
    content_type text not null,
    file_size_bytes bigint not null,
    blob_key text not null,
    tags text[] not null default '{}',
    date_issued date,
    ocr_status text not null default 'pending'
        check (ocr_status in ('pending', 'processing', 'done', 'failed', 'skipped')),
    ocr_text text,
    ocr_error text,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index documents_tenant_id_idx on documents (tenant_id);
create index documents_tags_idx on documents using gin (tags);
