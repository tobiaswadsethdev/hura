-- The trackers an account keeps, and the key their tokens are encrypted with.

-- One key per account, wrapped by the service's master key
-- (`HURA_CLOUD_MASTER_KEY`). Moving the master key into a KMS later changes
-- how this column is unwrapped, and nothing else.
create table account_keys (
    account_id uuid primary key references accounts (id) on delete cascade,
    wrapped bytea not null,
    created_at timestamptz not null default now()
);

-- A tracker, as `hura_core::tracker::Stored` keeps one: its source, and its
-- token encrypted with the account's key. Named as on the desktop, where the
-- name is what a session records its ticket came from, so it is the key and
-- is never changed.
create table trackers (
    account_id uuid not null references accounts (id) on delete cascade,
    name text not null,
    source jsonb not null,
    token bytea,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    primary key (account_id, name)
);
