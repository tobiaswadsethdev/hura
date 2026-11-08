# hura cloud

The service behind signing in: accounts, the devices signed in to them, and
the trackers they keep, read on a timer so a phone hears about a ticket on
time. Why it exists, and what comes next, is in
[plans/hura-cloud.md](plans/hura-cloud.md). Without signing in, nothing here
is used: trackers and their tokens stay on the device, as before.

It is `crates/hura-cloud`, one binary on Postgres, configured by its
environment. The hosted one is at `https://api.gethura.app`; the same image
runs anywhere else.

## Running it

| Variable | |
| --- | --- |
| `DATABASE_URL` | Postgres, required. The schema is brought up to date at every start |
| `LISTEN` | `0.0.0.0:8080` unless set. Plain HTTP: TLS is the proxy's in front of it |
| `RESEND_API_KEY` | sends sign-in codes through [Resend](https://resend.com) |
| `MAIL_FROM` | who they are from, `hura <sign-in@gethura.app>` unless set. Its domain has to be verified on the Resend account |
| `HURA_CLOUD_MASTER_KEY` | required: 32 bytes in base64, from `openssl rand -base64 32`. It wraps each account's key for its trackers' tokens, so **losing it loses every stored token**: keep a copy somewhere safe that is not the database's backups |
| `HURA_CLOUD_RELAY` | the push relay tracker notifications go through, `https://hura-relay.twadseth.workers.dev` unless set. The service sends only there, whatever a phone says |
| `HURA_CLOUD_APPLE_AUDIENCE` | the iOS app's bundle ID, which a Sign in with Apple token must be for: `dev.tobiaswadseth.hura.ios` unless set |
| `HURA_CLOUD_URL` | where the service is reached from outside, `https://api.gethura.app` unless set: the address in the callbacks and the webhook it gives GitHub and Atlassian |
| `HURA_CLOUD_MAIL=log` | prints codes instead of sending them, for a laptop. Without this or a Resend key the service will not start |

On a laptop, with Docker for the database:

```sh
docker run -d --name hura-cloud-pg -e POSTGRES_PASSWORD=hura -p 55432:5432 postgres:17-alpine
DATABASE_URL=postgres://postgres:hura@localhost:55432/postgres HURA_CLOUD_MAIL=log \
  HURA_CLOUD_MASTER_KEY=$(openssl rand -base64 32) cargo run -p hura-cloud
```

Its tests run against the same database, each on one of its own, and skip
themselves without it:

```sh
HURA_CLOUD_TEST_DATABASE_URL=postgres://postgres:hura@localhost:55432/postgres \
  cargo test -p hura-cloud
```

## On Coolify

The image is `ghcr.io/tobiaswadsethdev/hura-cloud`, built from main by
`.github/workflows/cloud.yml`: `latest`, and one tag per commit to go back to.

1. **A Postgres** resource in the project, with Coolify's scheduled backups to
   S3 turned on. Its internal URL is the `DATABASE_URL`.
2. **An application** from that Docker image, on the same network as the
   database: port 8080, domain `https://api.gethura.app`, health check path
   `/healthz`. The package is private unless made public on GitHub; a private
   one needs ghcr.io credentials (a token with `read:packages`) added to
   Coolify.
3. **Its environment**: `DATABASE_URL`, `RESEND_API_KEY` and
   `HURA_CLOUD_MASTER_KEY` as secrets.
4. **Redeploy** to take a new `latest`, by hand or with Coolify's webhook from
   the workflow.

## The API

JSON in and out. A failure is a status and `{"error": "..."}`, in words that
can be shown to a person. Every route under `/v1` but signing in and
`/v1/token` takes `Authorization: Bearer <access token>`.

**Tokens.** Signing in gives a device a refresh token (`hura_rt_…`), which it
keeps until it signs out, and an access token (`hura_at_…`), which lasts an
hour. `POST /v1/token` swaps the first for a new second. A 401 from it means
the device was signed out, from itself or another; the service keeps only
hashes of either.

| | |
| --- | --- |
| `POST /v1/sign-in/email` `{email}` | sends a six-digit code. 204 whether or not the address has an account; 429 if one was sent in the last minute (five an hour at most) |
| `POST /v1/sign-in/email/verify` `{email, code, device: {name, platform}}` | `{account, device, refresh_token, access_token, expires_in}`. Signs in to the address's account, made if it has none. A code lasts ten minutes, is used once, and is spent after five wrong tries |
| `POST /v1/sign-in/apple` `{identity_token, nonce, device: {name, platform}}` | the same answer, for Sign in with Apple in the iOS app. The token's signature is checked against Apple's published keys, and it must be for the app (`HURA_CLOUD_APPLE_AUDIENCE`), unexpired, and carry the SHA-256 of `nonce`. It signs in to the account this Apple ID used before, or one whose verified address it shares (never a relayed "hide my email" one), or a new one |
| `POST /v1/token` `{refresh_token}` | `{access_token, expires_in}` |
| `GET /v1/me` | `{account, plan, sign_ins: [{provider, email}]}` |
| `GET /v1/devices` | `[{id, name, platform, created_at, last_seen_at, current}]`, newest first; times in Unix seconds |
| `DELETE /v1/devices/{id}` | signs that device out; its tokens stop working at once |
| `POST /v1/sign-out` | signs out the device asking |
| `GET /healthz` | `ok`, once the database answers |

### Trackers

Kept as the desktop keeps them: the same checks on adding one, a name that is
never changed, and a token that is stored but never shown. Each change answers
with the whole list, `[{source, token_set}]`. A token is encrypted with a key
of the account's own, which the master key wraps.

| | |
| --- | --- |
| `GET /v1/trackers` | the list |
| `POST /v1/trackers` `{source, token?}` | adds one. 400 if it could not work (a Jira tracker with no site), 409 if the name is taken |
| `PUT /v1/trackers/{name}` `{source}` | changes one, keeping its token. Not its name |
| `PUT /v1/trackers/{name}/token` `{token}` | stores or replaces its token |
| `DELETE /v1/trackers/{name}` | takes it away, token and all |
| `POST /v1/tracker` `{op, ...}` | one of the requests the iOS app makes of hura-core (`hura_core::tracker::request`), on the account's trackers |

`POST /v1/tracker` takes the same requests as the phone's own copy of
hura-core, with two differences: where a request takes a `tracker`, it is the
name of one of the account's, and `inbox` reads all of them. It answers 200
with `{"ok": ...}` or `{"error": ...}`, as the phone's does, so a client reads
the same thing from either.

### Connecting a tracker through its own sign-in

Instead of pasting a token, a tracker can be connected through GitHub (as a
GitHub App, with user tokens that act as you, so "assigned to me" is still
you) or through Atlassian's OAuth for Jira. The service swaps the code for
tokens, keeps them encrypted on a tracker of the account's (one already there
for the same account or site is connected in place, keeping its filters), and
renews them before they run out.

| | |
| --- | --- |
| `GET /v1/connect` | `{github, jira}`: which the service is set up to connect |
| `POST /v1/connect/{provider}` `{return}` | `{url}`, where to send the browser. `return` is `app`, to come back on `hura://cloud/connected`, or `browser`, to end on a page saying to go back |
| `GET /v1/oauth/{github,jira}/callback` | where the providers send the browser back |
| `POST /v1/webhooks/github` | the GitHub App's webhook: signed with its secret, and every account whose GitHub login is in a delivery is read at the watcher's next pass |

A connected tracker's list entry carries `connected`, who it is connected as;
its token cannot be set by hand.

**Setting the apps up.** While either is missing, the service writes
`setup: <url>/setup?token=…` to its log at start; the token is fresh at each
start and is the only way in. The page makes the GitHub App with GitHub's
manifest flow (one button, and its credentials come straight back), and takes
the Jira OAuth app's client ID and secret, with the steps for making one in
Atlassian's console. Both are kept encrypted with the master key. The GitHub
App is private to whoever makes it until it is made public in its settings.

### Notifications

The phone links the service on the push relay as it links an `hurad`
([relay-api.md](relay-api.md)): a push token from the relay and a key it makes
for the text, handed over with `PUT /v1/push`. From then on the account's
trackers are read every minute or so, by the same comparison the apps make
for themselves (`hura_core::tracker::changes`), against what was seen kept per
account. Each change goes to every device with a route as a `tracker_change`
event whose text only the phone can read.

| | |
| --- | --- |
| `PUT /v1/push` `{server_id, push_token, push_key}` | where to tell this device. The token is tried with the relay first; the token and key are kept encrypted |
| `GET /v1/push` | `{configured, broken}`: `broken` says why the relay stopped taking the token |
| `DELETE /v1/push` | stop telling this device |

A desktop is told over a connection it keeps open while its window is:

| | |
| --- | --- |
| `GET /v1/events` | server-sent events: a `tracker_change` event for each read that found something, its data `{"changes": [...]}` as `hura_core::tracker::changes` gives them, and a comment every fifteen seconds to keep proxies from closing it |

An account with a stream open is read even with no phone linked. The
streams are the process's own, so for now the service runs as one instance.

What was seen moves on only once every device has been told, or could not be
for good. A relay that is away means the same changes are sent again at the
next read under the same event IDs, which the relay drops as duplicates. An
account is read only while it has a working route or an open stream, as often
as its plan allows plus up to half a minute, and less often while none of its
trackers answers.

### Seen

Looking at the inbox on one device counts on all of them: what was waiting
on you then (tickets assigned to you, pull requests asking for your review,
as `tracker/id`) is kept for the account, and the account's other phones are
woken quietly to clear their badge and the tracker notifications they showed
before then.

| | |
| --- | --- |
| `GET /v1/seen` | `{items, at}`; `at` in Unix seconds, or null before the inbox has been looked at anywhere |
| `PUT /v1/seen` `{items}` | the inbox was just looked at on this device. When that differs from what was seen, every other device with a push route gets an `inbox_seen` background push |

iOS spaces background pushes out as it sees fit, so a phone may catch up only
when it is next opened, which asks too.

The service only sends requests over HTTPS to public addresses. A Jira site
inside the server's own network (`https://10.0.0.5`, `localhost`) is refused,
since otherwise any account could make the server ask its neighbours.
