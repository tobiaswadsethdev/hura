# Plan: hura cloud

A central service with accounts, which keeps your trackers and filters in one
place and watches them all day, so that a phone is told about a ticket within a
minute or two rather than whenever iOS lets the app wake up.

This is a plan, worked on in increments. Each phase below is meant to be a pull
request or a few, and to leave everything working when it lands. Tick a phase
off here when it does.

## Why

Two things the trackers cannot do as long as they live only on the devices:

- **Notify a phone on time.** The desktop reads its trackers every three
  minutes while it is open. The phone can do the same only while the app is in
  front; in the background it gets background app refresh, which iOS runs at
  most every fifteen minutes and often hours apart. Something has to be awake
  all the time and push.
- **Be set up once.** A tracker, its token and its filters are added on the
  desktop and again on the phone, and each keeps its own idea of what you have
  seen, so the same change is announced twice.

Three ways of getting the first were weighed (see the discussion that led
here): the desktop forwarding its notifications through `hurad` and the relay,
which only works while the desktop is open; `hurad` reading the trackers,
which puts a tracker token on the machine that hosts sandboxes; and Jira's own
webhooks, which need an OAuth app, cannot express `currentUser()` or a saved
filter, and are limited to five per user. A service of our own, with accounts,
does both things and can use webhooks later where they help.

**This reverses a principle.** [tickets.md](../tickets.md) says the server holds
no tracker, no token and no ticket, and the tracker module says the same. That
stays true of `hurad`. It stops being true of hura as a whole for anyone who
signs in, which is why local-only stays (below).

## Decisions

| | |
|---|---|
| Who it is for | You, for now. Others later, possibly as a paid subscription, so it is built for many accounts from the first table |
| Where it runs | The dedicated server, under Coolify |
| Local-only mode | Kept. Without signing in, trackers and tokens stay on the device and are read from there, as today |
| Signing in | Sign in with Apple, and a sign-in that needs no Apple device (an emailed code, passkeys later) |
| Domain | `gethura.app` |
| Mail | Resend, with `gethura.app` already verified there as a sending domain |

## Shape

```
                 ┌──────────── hura cloud ──────────────┐
 desktop ──┐     │ accounts · trackers + filters         │
           ├────►│ tokens (encrypted) · what was seen     │──► Jira / GitHub / Azure DevOps
 phone ────┘     │ reads on a timer · what changed        │◄── webhooks (later)
     ▲           └────────────────┬───────────────────────┘
     │  live connection           │ events
     └── desktop notification     relay ──► APNs ──► phone
```

**`crates/hura-cloud`, in this repository**, a Rust service that runs
`hura-core`'s tracker code as it is. Rust rather than a Worker beside the relay
because that code is synchronous over a blocking transport
(`tracker::use_transport`), and a Worker would mean rewriting it async in
WebAssembly. One code path for the desktop, the phone and the service is the
point.

**Deployed as one image.** Built by a workflow into GHCR beside `image.yml`'s,
pulled by Coolify, which also terminates TLS on the domain and holds the
secrets. **Postgres**, from Coolify's own, rather than SQLite: little more work
now, and what many accounts want later. Coolify's scheduled backups to S3 cover
it.

**Self-hostable**, since it is the same image configured by environment. That
is the third choice for someone who will not hand a token over: local-only,
their own `hura-cloud`, or the hosted one.

**`hurad` is not involved.** Sessions and sandboxes stay on your machines and
pairing stays device to device; the service is about trackers.

## Accounts

**Signing in and connecting a tracker are separate.** You sign in with Apple or
an email; Jira and GitHub are connections added afterwards.

- **Sign in with Apple**: native on iOS, and the web flow on the desktop (which
  needs a Services ID).
- **An emailed one-time code**, so there is no password to store and no Apple
  device needed. Sent through Resend, from `gethura.app`.
- **Passkeys** later.
- One account, more than one way in: linked by a verified email, and by hand
  where Apple's "hide my email" gives an address that matches nothing.

**Each device has its own long-lived token**, listed and revocable from any
other, and swaps it for short-lived access tokens. The desktop signs in in the
browser and comes back on a `hura://` link, as pairing already does.

## Trackers on the service

**The same requests as now.** `crates/hura-mobile` already takes tracker
operations as JSON (`inbox`, `issue`, `transition`, `comment`, `pull`,
`pull_vote`…). The service takes the same at `POST /v1/tracker`, as the signed-in
account and with its tokens. A client therefore has two backends with one
request shape: hura-core in the process (local-only) or HTTPS (signed in). On
the desktop that is a second branch in the Tauri commands; on the phone, in
`TrackerCore.call`.

**Tokens are encrypted at rest**: a master key from Coolify's secrets wraps a
key per account, which encrypts that account's tokens. Moving the master key
into a KMS later changes one place.

**Moving over.** Signing in offers to upload this device's trackers, tokens
included, and then forgets them locally. Signing out goes back to local-only
with nothing configured.

## Notifications

**What changed is worked out once, on the service.** It reads each tracker
every one to two minutes, spread out so they do not all go at once, backing off
from one that fails or answers 429. What was seen is kept per account, so a
change is found once and announced to every device, and nothing is announced
twice.

**To the phone, through the relay.** The app links the service the way it links
a `hurad`: `POST /v1/servers` for a push token, handed to the service with the
phone's push key. The relay needs one new kind, `tracker_change`, and sees only
ciphertext, as it does for `hurad`'s.

**To the desktop, over a live connection** while the window is open, as an OS
notification. Signed in, the desktop stops reading on its own three-minute
timer.

## Paid, later

Not built until there is someone to charge, but room is left for it now: an
account has a plan, and a plan has limits (trackers, filters, how often they
are read). Stripe for the web and the desktop. On iOS, a subscription bought in
the app is Apple's in-app purchase, or it is sold on the web only; Apple's
rules on pointing to a web purchase vary by country and have been changing,
so they are checked when the time comes. RevenueCat can make both one
entitlement.

**Before anyone else signs up**, since this is run from the EU: the service
stays in the EU, there is a privacy policy, and an account can be exported and
deleted.

## Phases

- [x] **1. What changed, in `hura-core`.** `tracker::changes::record` takes
  a read and what was seen at the last one, and answers what is seen now and
  what changed. The desktop calls it through the `inbox_changes` command and
  the phone through hura-mobile's `changes` op; each keeps what was seen where
  it kept its snapshot before, in the same shape, so nothing is announced
  twice or missed across the update. The service will keep it per account.
- [ ] **2. The service, bare.** `crates/hura-cloud` ([cloud.md](../cloud.md)):
  - [x] The Postgres schema for accounts, sign-ins, devices and plans;
    emailed codes through Resend; refresh and access tokens; the device
    list; the image, its workflow and CI against a real Postgres.
  - [ ] Sign in with Apple.
  - [ ] The encrypted tracker store and `POST /v1/tracker`.
  - [ ] The Coolify deployment at `api.gethura.app`.
- [ ] **3. The apps sign in.** Sign-in screens on both, the local and HTTPS
  backends, uploading local trackers, and trackers and filters that are the
  same everywhere.
- [ ] **4. Notifications from the service.** The scheduled reads, the phone
  through the relay (`tracker_change`), the desktop over a live connection.
  This is the phase that makes them timely.
- [ ] **5. Better connections.** Jira's OAuth app and a GitHub App instead of
  pasted tokens, their webhooks to read a tracker the moment it changes, and
  "seen" shared between devices, clearing a notification on the others.
- [ ] **6. Ready for others.** The master key in a KMS, monitoring, rate
  limits, export and deletion, plans enforced, billing.

## Not decided yet

- Whether the relay ever moves into this service. For now it stays as it is:
  small, separate, and working.
