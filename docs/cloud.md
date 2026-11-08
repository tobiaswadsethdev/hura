# hura cloud

The service behind signing in: accounts, the devices signed in to them, and
(being built) the trackers they keep, read on a timer so a phone hears about a
ticket on time. Why it exists, and what comes next, is in
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
| `HURA_CLOUD_MAIL=log` | prints codes instead of sending them, for a laptop. Without this or a Resend key the service will not start |

On a laptop, with Docker for the database:

```sh
docker run -d --name hura-cloud-pg -e POSTGRES_PASSWORD=hura -p 55432:5432 postgres:17-alpine
DATABASE_URL=postgres://postgres:hura@localhost:55432/postgres HURA_CLOUD_MAIL=log \
  cargo run -p hura-cloud
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
3. **Its environment**: `DATABASE_URL` and `RESEND_API_KEY` as secrets.
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
| `POST /v1/token` `{refresh_token}` | `{access_token, expires_in}` |
| `GET /v1/me` | `{account, plan, sign_ins: [{provider, email}]}` |
| `GET /v1/devices` | `[{id, name, platform, created_at, last_seen_at, current}]`, newest first; times in Unix seconds |
| `DELETE /v1/devices/{id}` | signs that device out; its tokens stop working at once |
| `POST /v1/sign-out` | signs out the device asking |
| `GET /healthz` | `ok`, once the database answers |
