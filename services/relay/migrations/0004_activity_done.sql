-- A session keeps one activity from turn to turn (docs/relay-api.md, "Live
-- Activities"), so the relay has to know when it began, to start a new one
-- before ActivityKit's eight hours are up, and what it showed when the turn
-- finished, to end it once that has stayed long enough.

-- Unix ms: when the activity was started, or first heard of from the phone.
ALTER TABLE activities ADD COLUMN started_at INTEGER;
-- The JSON of the `done` or `failed` content it shows, from `updated_at` on;
-- NULL while the session is in a turn.
ALTER TABLE activities ADD COLUMN done TEXT;
UPDATE activities SET started_at = updated_at;
