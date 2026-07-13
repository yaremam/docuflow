create table smart_collections (
    id uuid primary key,
    tenant_id uuid not null references tenants(id),
    name text not null,
    query text not null default '',
    created_at timestamptz not null default now()
);

create index smart_collections_tenant_id_idx on smart_collections (tenant_id);
