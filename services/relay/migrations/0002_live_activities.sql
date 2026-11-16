-- Live Activities (docs/relay-api.md, "Live Activities").

-- The device's push-to-start token, which starts an activity when none shows.
ALTER TABLE devices ADD COLUMN activity_token TEXT;

-- The update token of the activity showing one session from one link.
CREATE TABLE activities (
  link_id     TEXT NOT NULL REFERENCES links(id) ON DELETE CASCADE,
  session     TEXT NOT NULL,
  token       TEXT NOT NULL,
  updated_at  INTEGER NOT NULL,                -- unix ms
  PRIMARY KEY (link_id, session)
);
