-- One Live Activity for all of a device's sessions, T3 Code's (docs/relay-api.md,
-- "Live Activities"), for an app that asks for it; an older app keeps one per
-- session.

-- 'agents' when the device shows one activity for every session; NULL for
-- one per session, which is what every device before it did.
ALTER TABLE devices ADD COLUMN activity_style TEXT;

-- What each session shows in it: the latest event, as the JSON of its content,
-- with the server it came from. A finished turn's row stays a while, as a
-- session's own activity did, and an ended session's goes.
CREATE TABLE agent_rows (
  device_id   TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  link_id     TEXT NOT NULL REFERENCES links(id) ON DELETE CASCADE,
  session     TEXT NOT NULL,
  state       TEXT NOT NULL,
  at          INTEGER NOT NULL,                -- unix seconds, the event's
  done        INTEGER NOT NULL DEFAULT 0,      -- 1 once the turn finished or failed
  updated_at  INTEGER NOT NULL,                -- unix ms
  PRIMARY KEY (device_id, link_id, session)
);

-- The update token of the device's one activity, or '' while a push has
-- started it and the phone has not yet said where its updates go.
CREATE TABLE agent_activities (
  device_id   TEXT PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
  token       TEXT NOT NULL,
  updated_at  INTEGER NOT NULL,                -- unix ms
  started_at  INTEGER NOT NULL                 -- unix ms
);
