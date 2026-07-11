create table tenants (
    id uuid primary key,
    created_at timestamptz not null default now()
);
