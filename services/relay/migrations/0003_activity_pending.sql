-- What a session's activity should show once the phone gives its token: the
-- latest event that came while it was still starting, as the JSON of its
-- content (docs/relay-api.md, "Live Activities").
ALTER TABLE activities ADD COLUMN pending TEXT;
