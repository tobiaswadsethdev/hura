-- Connecting a tracker through its own sign-in (a GitHub App, Atlassian's
-- OAuth) rather than a pasted token.

-- The service's own apps there: client IDs and secrets, and a GitHub App's
-- webhook secret and private key. Encrypted with the master key.
create table integrations (
    kind text primary key check (kind in ('github', 'jira')),
    config bytea not null,
    created_at timestamptz not null default now()
);

-- A tracker connected that way: who as, its cloud ID on Atlassian, and when
-- its access token runs out (none of which is secret); and its refresh token,
-- encrypted with the account's key.
alter table trackers add column connection jsonb;
alter table trackers add column refresh_token bytea;

create index trackers_github_login on trackers (lower(connection->>'login'))
    where connection->>'provider' = 'github';

-- Connecting is a round trip through a browser, and the state that comes back
-- says whose it is and where to send them after. Kept as a hash, used once.
create table oauth_states (
    state_hash bytea primary key,
    account_id uuid not null references accounts (id) on delete cascade,
    provider text not null,
    return_to text not null,
    expires_at timestamptz not null
);
