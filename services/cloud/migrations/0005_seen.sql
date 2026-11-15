-- What was waiting on you when you last looked at the inbox, on any of the
-- account's devices: the same "tracker/id" keys the apps count their badge
-- by, when, and from which device.
create table inbox_seen (
    account_id uuid primary key references accounts (id) on delete cascade,
    items text[] not null,
    seen_at timestamptz not null default now(),
    device_id uuid references devices (id) on delete set null
);
