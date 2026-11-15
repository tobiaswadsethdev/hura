-- Telling a device that something changed on its account's trackers.

-- Where to: a link on the push relay, made by the phone as it links an
-- hurad, and the key the phone made for the text. Both are encrypted with
-- the account's key. One per device; signing the device out removes it.
create table push_routes (
    device_id uuid primary key references devices (id) on delete cascade,
    account_id uuid not null references accounts (id) on delete cascade,
    -- The phone's ID for this service on the relay, `h_` and 16 base32.
    server_id text not null,
    push_token bytea not null,
    push_key bytea not null,
    -- Why the relay stopped taking the token, once it has.
    broken text,
    created_at timestamptz not null default now()
);

create index push_routes_account on push_routes (account_id);

-- Reading an account's trackers on a timer: what was seen at the last read
-- (`hura_core::tracker::changes::Seen`), and when the next is due. The seen
-- is JSON kept as bytes, not `jsonb`: its keys hold a NUL between tracker and
-- ticket, which no Postgres text type will store.
create table watches (
    account_id uuid primary key references accounts (id) on delete cascade,
    seen bytea,
    next_read_at timestamptz not null default now(),
    failures integer not null default 0,
    last_read_at timestamptz
);
