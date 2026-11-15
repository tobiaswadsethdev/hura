-- Signing in with Apple through a browser, for the desktop: a round trip that
-- starts in the app, goes through Apple, and comes back to the app on this
-- computer's loopback. Kept for the ten minutes it may take, then gone.
create table apple_web_sign_ins (
    -- What Apple is sent and sends back.
    state_hash bytea primary key,
    -- The app's own secret, which it shows again to finish.
    secret_hash bytea not null,
    -- Whose SHA-256 Apple puts in the identity token.
    nonce text not null,
    -- Where on 127.0.0.1 the app waits for the browser.
    port integer not null,
    device_name text not null,
    platform text not null,
    -- Once Apple has answered: who signed in, and the code the browser
    -- carries back to the app.
    subject text,
    email text,
    code_hash bytea unique,
    expires_at timestamptz not null
);
