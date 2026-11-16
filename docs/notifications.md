# Notifications on a phone

The desktop application tells you when a session starts waiting on you, but
only while it is open. To tell you on a phone, `hurad` watches the sessions
itself and sends notifications to the **hura iOS app** through the **hura
relay**. The relay is a small Cloudflare Worker in [`services/relay/`](../services/relay) that
holds the APNs key. Its API is in [relay-api.md](relay-api.md).

```
   iPhone (hura app)                                the hurad machine
   +---------------+  push-configure (paired TLS)   +-----------------+
   |  registers    | -----------------------------> |  hurad          |
   |  with the     |                                |   watches every |
   |  relay, links |   <---- APNs <---- relay <---- |   session       |
   |  the server   |                                +-----------------+
   +---------------+
```

**The relay holds the APNs key, and `hurad` doesn't.** What `hurad` holds is
one push token per paired client. The token can make that client's phone show
a notification, and that is all it can do. The relay stores only a hash of it.

## What notifies

| What happened | Kind | Body |
| --- | --- | --- |
| A chat session asks for permission | `claude_permission`, with Allow and Deny | what it asks, e.g. `Bash: npm test` |
| Any other wait: a terminal session's prompt, or a question or plan in a chat session | `claude_needs_input` | a question in its own words (`Which database should we use?`), `Claude has a plan for you to review`, `Claude is asking to use Bash`, or Claude Code's own sentence |
| A turn finishes: running or waiting, then idle | `claude_turn_finished` | `Claude finished and is waiting for you` |
| A session fails | `claude_failed` | `The session failed` |

The title is the session's name, and the subtitle is its project -- or, for a
session filed under none, this machine's hostname. The phone puts its own name
for the server in front, as `Mac · scope-creep`, because the server does not
know what the phone calls it.
`claude_permission` follows the `claude_needs_input` preference, because to a
person they are the same thing.

A tool's name is for a program, so it is not what a notification says when
there is anything better: `AskUserQuestion` is the question itself (the first,
with how many more there are), or `Claude has a question for you` from an
image too old to say it; `ExitPlanMode` is `Claude has a plan for you to
review`; and an MCP server's tool is said as `create_issue (github)` rather
than `mcp__github__create_issue`.

Notifications are sent **on the transition, not on the state**, as in the
desktop application. A session that has been waiting for an hour doesn't
notify every few seconds. Nor does starting `hurad`, or turning push on, while
sessions are already waiting; but a session created a moment ago that is
already waiting does notify.

The watcher looks every five seconds, with the same per-session poll the
status channel uses. It runs only while at least one route is configured, so
a server with no push set up pays nothing for it.

## Answering from the notification

A chat session's host knows which request it is waiting on, and writes it into
the status file, together with a line saying what the request does. A
`claude_permission` notification carries that request's id as `ask_id`, and
the app sends it back with the answer:

```json
→ {"op": "answer", "name": "fix-tests", "id": "toolu_01…",
   "decision": {"allow": true}}
← {"ok": {"reply": "told", "message": "allowed"}}
```

The answer goes to that request and no other. Once the request has been
answered, from the window or from another phone, a second answer is refused.
So an Allow tapped late can't approve whatever came next. Questions
(`AskUserQuestion`) and plans (`ExitPlanMode`) need a choice that a button
can't carry, so they are sent as `claude_needs_input` without an id. So is a
terminal session's prompt, which is on its screen and has no id.

When one request is answered and the next arrives straight after, the next
gets a notification of its own, even though the session never stopped
waiting.

## Live Activities

A session also shows on the phone's Lock Screen and in the Dynamic Island as
a Live Activity, through the same route. The watcher sends a
`session_activity` event whenever what a session is doing changes:

| Phase | When | Body |
| --- | --- | --- |
| `working` | a turn starts, or carries on after an answer | `Claude is working` |
| `permission` | it waits on a permission request, which Allow and Deny on the activity answer | what it asks, as for `claude_permission` |
| `input` | it waits on anything else | as for `claude_needs_input` |
| `done` | the turn finishes | `Claude finished and is waiting for you` |
| `failed` | the session fails | `The session failed` |
| `ended` | the session is deleted or stopped mid-turn | `The session ended` |

As with notifications, these are sent on the change, not on the state, and
not on the first look; a `done` or an `ended` is only sent after a turn the
activity showed. The text is encrypted like a notification's, and each event
carries `at`, when the change was seen, so the phone ignores a retry that
arrives after something newer. A newer event for a session also replaces one
still queued for it. The relay decides whether that starts an activity,
updates it or ends it: see [relay-api.md](relay-api.md#live-activities).

The `session_activity` preference switches them off for a phone without
touching its notifications.

Allow and Deny on an activity are the notification's: the app sends the same
`answer` with the `ask_id` the activity carries, Allow only once the phone is
unlocked. The activity shows the session working again straight away, and the
next look says what came after.

## Setting it up

You set it up from the hura app, once this server is paired with it. The app:

1. registers the phone with the relay (`POST /v1/devices`);
2. links this server to the phone (`POST /v1/servers`), and gets a push token
   back;
3. hands the push token to `hurad` over the paired connection.

Step 3 is one request on `/rpc`:

```json
→ {"op": "push-configure",
   "relay_url": "https://hura-relay.example.workers.dev",
   "push_token": "hup_…",
   "push_key": "<32 bytes, base64url without padding>",
   "server_id": "h_aaaabbbbccccdddd",
   "prefs": {"claude_turn_finished": false}}
← {"ok": {"reply": "push", "configured": true, "label": "studio",
          "encrypted": true, "prefs": {…}, "queued": 0, …}}
```

**`server_id` is the `h_…` id the app linked this server under.** An `hurad`
has no id of its own, so the app chooses one when it pairs. The id is part of
what an encrypted notification is bound to, so it must be the one the relay
puts in the payload.

`hurad` checks the token with the relay (`GET /v1/link`) before keeping it. A
rejected token is a failure, and nothing is stored.

| Request | Answer |
| --- | --- |
| `push-configure` | the route, as below. Configuring again replaces it and drops anything queued for the old one |
| `push-status` | `configured: false` when this connection has no route |
| `push-remove` | the route is gone |
| `push-test` | a test notification, queued like a real one |

`prefs` takes `claude_needs_input`, `claude_turn_finished`, `claude_failed`
and `session_activity`, each on unless it says `false`.

Every one answers with `PushView`: `configured`, `relay_url`, `label`,
`prefs`, `encrypted`, `broken`, `last_success_at`, `last_error` and `queued`.
There's never a token in it.

## One route per paired client

A route belongs to the token the request came with, not to the server. Each
paired client configures its own. **`hurad revoke` removes the route too,
within one look of the watcher (five seconds).** A phone that may no longer
drive this server is no longer told what its sessions are doing either.
Pairing again under the same name mints a new token, which has no route until
it configures one.

## Encrypted text

With a `push_key`, the title, subtitle and body leave `hurad` only as
ciphertext. The relay and Apple see the kind and show generic text, such as
`Claude needs your permission`. The app's notification service extension
decrypts the real text on the phone. The ciphertext is bound to the server,
the kind, the session and the request, so the relay can't move one
notification's text onto another's. For a permission request, that means it
can't point an Allow at a different request from the one the phone showed.
[relay-api.md](relay-api.md#encrypted-text) has the exact scheme, and
`hurad/daemon/src/push/push-crypto.json` is a test vector that both `hurad`
and the app are checked against.

## Delivery

Notifications are queued on disk and retried, so an outage of the relay or
the network delays them rather than losing them. They are kept for up to six
hours, the same expiry the relay gives APNs. Every retry carries the same
`event_id`, which the relay treats as a duplicate, so retrying after a lost
answer never buzzes the phone twice. The relay's answers are handled as its
API says:

| The relay answers | `hurad` |
| --- | --- |
| `200` | done: sent, a duplicate, or a phone that is gone |
| `400` | drops it, and says why in `last_error` |
| `401` | marks the route `broken` and stops sending on it until the app configures it again |
| `429` | waits as long as `Retry-After` says |
| `5xx`, no answer | backs off, from two seconds up to five minutes |

## Files

Both are in `~/.local/state/hura/`, readable only by their owner:

- `push.json`: the routes, including each push token and key;
- `push-queue.json`: what is still to be delivered.
