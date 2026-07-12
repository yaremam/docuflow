create table scan_sessions (
    id uuid primary key,
    tenant_id uuid not null references tenants(id),
    user_id uuid not null references users(id),
    token_hash text not null unique,
    status text not null default 'pending'
        check (status in ('pending', 'captured')),
    document_id uuid references documents(id),
    expires_at timestamptz not null,
    created_at timestamptz not null default now()
);

create index scan_sessions_tenant_id_idx on scan_sessions (tenant_id);
