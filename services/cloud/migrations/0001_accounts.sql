-- Accounts, the ways into them, and the devices signed in to them.
--
-- Built for many accounts from the first table, though there is one: an
-- account has a plan, and every row below an account says which.

-- What an account may do. Nothing enforces these yet; they are here so that
-- an account always has a plan to be checked against when something does.
create table plans (
    id text primary key,
    -- Null is no limit.
    max_trackers integer,
    max_filters integer,
    -- How often one of the account's trackers may be read, at most.
    min_read_seconds integer not null
);

insert into plans (id, max_trackers, max_filters, min_read_seconds)
values ('free', null, null, 60);

create table accounts (
    id uuid primary key default gen_random_uuid(),
    plan text not null default 'free' references plans (id),
    created_at timestamptz not null default now()
);

-- One way into an account: an email address that answered a code, or an
-- Apple ID. One account may have several, joined by a verified email.
create table sign_ins (
    id uuid primary key default gen_random_uuid(),
    account_id uuid not null references accounts (id) on delete cascade,
    provider text not null check (provider in ('email', 'apple')),
    -- The address, for `email`; Apple's own identifier, for `apple`.
    subject text not null,
    -- A verified address, lowercased, or null where there is none.
    email text,
    created_at timestamptz not null default now(),
    unique (provider, subject)
);

create index sign_ins_email on sign_ins (email);

-- Codes sent to an address, hashed. One that is used, or tried too often, is
-- spent; the rows are what the sending limits count.
create table email_codes (
    id uuid primary key default gen_random_uuid(),
    email text not null,
    code_hash bytea not null,
    attempts integer not null default 0,
    created_at timestamptz not null default now(),
    expires_at timestamptz not null,
    used_at timestamptz
);

create index email_codes_email on email_codes (email, created_at desc);

-- A device signed in to an account, holding a long-lived refresh token that
-- it swaps for access tokens. Signing a device out deletes its row, and its
-- access tokens with it.
create table devices (
    id uuid primary key default gen_random_uuid(),
    account_id uuid not null references accounts (id) on delete cascade,
    name text not null,
    platform text not null,
    refresh_hash bytea not null unique,
    created_at timestamptz not null default now(),
    last_seen_at timestamptz not null default now()
);

create index devices_account on devices (account_id);

create table access_tokens (
    token_hash bytea primary key,
    device_id uuid not null references devices (id) on delete cascade,
    expires_at timestamptz not null
);

create index access_tokens_expiry on access_tokens (expires_at);
