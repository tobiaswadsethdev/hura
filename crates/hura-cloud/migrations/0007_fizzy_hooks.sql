-- Webhooks on a Fizzy tracker's boards, which make its account due for a
-- read the moment a card changes.

-- One per board the token runs. Its ID is the last part of the address
-- Fizzy delivers to, and its secret -- which Fizzy makes and shows once, as
-- the webhook is made -- is encrypted with the account's key. Fizzy's own
-- ID for it is what removing it again takes.
create table fizzy_hooks (
    id uuid primary key,
    account_id uuid not null,
    tracker text not null,
    -- The account's number, as in its address, and the board's ID.
    fizzy_account text not null,
    board_id text not null,
    webhook_id text not null,
    secret bytea not null,
    created_at timestamptz not null default now(),
    foreign key (account_id, tracker) references trackers (account_id, name) on delete cascade,
    unique (account_id, tracker, fizzy_account, board_id)
);

-- What the last look at a tracker's boards found, in words for its row on
-- the settings screen, and when that was: they are looked at again now and
-- then, for a board added since or a webhook Fizzy turned off.
alter table trackers add column webhooks text;
alter table trackers add column webhooks_checked_at timestamptz;
