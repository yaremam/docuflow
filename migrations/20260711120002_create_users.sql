create table users (
    id uuid primary key,
    tenant_id uuid not null references tenants(id),
    email text not null unique,
    password_hash text not null,
    created_at timestamptz not null default now()
);
