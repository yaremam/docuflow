create table password_reset_tokens (
    id uuid primary key,
    user_id uuid not null references users(id),
    token_hash text not null unique,
    expires_at timestamptz not null,
    used_at timestamptz,
    created_at timestamptz not null default now()
);

create index password_reset_tokens_user_id_idx on password_reset_tokens (user_id);
