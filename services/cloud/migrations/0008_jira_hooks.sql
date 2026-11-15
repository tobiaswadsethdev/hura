-- The webhook on a connected Jira site, which makes its account due for a
-- read the moment an issue there changes that is the account's.

-- One per connected tracker, registered with its own token: Atlassian keeps
-- an OAuth app's webhooks per person and site, and sends every one of them
-- to the same address, naming the ones that matched. So a delivery is
-- matched to its accounts here, by Jira's ID for the webhook, and to the
-- issues that are theirs by who the token belongs to.
create table jira_hooks (
    account_id uuid not null,
    tracker text not null,
    cloud_id text not null,
    webhook_id bigint not null,
    -- The Atlassian account ID the token acts as: an issue assigned to or
    -- reported by it is the account's.
    jira_account text not null,
    -- Jira stops sending it thirty days after it was made or last renewed.
    expires_at timestamptz,
    created_at timestamptz not null default now(),
    primary key (account_id, tracker),
    foreign key (account_id, tracker) references trackers (account_id, name) on delete cascade
);

create index jira_hooks_webhook on jira_hooks (webhook_id);

-- Issues a delivery was about that were not yet known to be the account's:
-- whether one of the tracker's filters holds them is asked of Jira a few
-- seconds later, all of a tracker's in one search. Kept from the first
-- delivery, so an issue that keeps changing is still asked about.
create table jira_checks (
    account_id uuid not null,
    tracker text not null,
    issue_key text not null,
    queued_at timestamptz not null default now(),
    primary key (account_id, tracker, issue_key),
    foreign key (account_id, tracker) references trackers (account_id, name) on delete cascade
);
