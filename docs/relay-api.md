# The hura relay, version 1

The relay is how `hurad` reaches a phone. It is a Cloudflare Worker in
[`relay/`](../relay) that holds the APNs key, and the only thing that talks to
Apple. The hura iOS app registers the phone with the relay and links each
server it has paired to the phone. Each `hurad` then sends events to the relay
with a push token that can do nothing else.

```
app ──register device, link server──► relay ◄──events (push token)── hurad
                                        │
                                        └──► APNs ──► iPhone (hura app)
```

[notifications.md](notifications.md) covers the `hurad` side: when it
notifies, and how a route is set up over the paired connection.

## Principles

- **`hurad` never holds the APNs key, and the relay never holds a pairing
  token.** A push token can make one phone show a notification and nothing
  more.
- **The relay decides how loud a notification is.** `hurad` sends a `kind`,
  and the relay maps it to an APNs category and interruption level. A leaked
  push token therefore can't send arbitrary payloads or break through Focus.
- **The relay stores only SHA-256 hashes of secrets**, and never logs them.
- **The server ID in a notification comes from the link**, not from the
  request, so one server can't pass itself off as another.
- **Notification text can be end-to-end encrypted**, so that the relay and
  Apple see only the kind (see [Encrypted text](#encrypted-text)).

## Conventions

- JSON bodies, `Content-Type: application/json`, at most 16 KiB.
- `Authorization: Bearer <secret>` where a route needs a credential.
- Errors are `{"error": {"code": "…", "message": "…"}}` with the HTTP status
  below. The codes are `bad_request`, `unauthorized`, `not_found`,
  `rate_limited`, `apns_error` and `internal`.
- Rate-limited responses carry `Retry-After`, in seconds.

| Credential | Format | Issued by | Used for |
|---|---|---|---|
| Device secret | `hud_` + 43 base64url chars | `POST /v1/devices` | the app managing its device and links |
| Push token | `hup_` + 43 base64url chars | `POST /v1/servers` | one `hurad` sending events for one link |

| ID | Format | Chosen by |
|---|---|---|
| Device | `d_` + 16 base32 characters | the relay |
| Link | `l_` + 16 base32 characters | the relay |
| Server | `h_` + 16 base32 characters | the app, when it pairs a server (an `hurad` has no ID of its own) |

Base32 here is lowercase RFC 4648: `a`–`z` and `2`–`7`.

## Device routes (used by the app)

### `POST /v1/devices`

Registers an iPhone. Needs no credential; limited to 10 registrations per
source address per hour.

```json
→ {"apns_token": "<hex device token>", "environment": "production"}
← 201 {"device_id": "d_…", "device_secret": "hud_…"}
```

`environment` is `sandbox` for builds run from Xcode, and `production` for
TestFlight and App Store builds. `apns_token` is 64–200 hex characters.

### `PUT /v1/devices/{device_id}`

The app calls this whenever iOS hands it a new device token. It also
re-enables a device that APNs had reported as unregistered.

```json
→ {"apns_token": "…", "environment": "production"}
← 200 {"device_id": "d_…"}
```

### `DELETE /v1/devices/{device_id}`

Removes the device and every link to it. `204`.

### `POST /v1/devices/{device_id}/test`

Sends a test notification straight to the device, and reports what APNs said:

```json
← 200 {"sent": true, "apns_status": 200, "apns_id": "…"}
← 200 {"sent": false, "apns_status": 400, "apns_reason": "BadDeviceToken"}
```

### `POST /v1/servers`

Links a server to the device. The app then hands the push token to that
`hurad` with `push-configure`. A device can have at most 50 links.

```json
→ {"server_id": "h_…", "label": "studio"}
← 201 {"link_id": "l_…", "push_token": "hup_…"}
```

### `GET /v1/servers`

```json
← 200 {"links": [{"link_id": "l_…", "server_id": "h_…", "label": "studio",
                  "created_at": "2026-10-30T21:00:00.000Z", "last_event_at": null}]}
```

### `DELETE /v1/servers/{link_id}`

Deletes the link; its push token stops working at once. `204`.

## Server routes (used by `hurad`)

### `GET /v1/link`

Checks a push token. `push-configure` calls this before it keeps a token.

```json
← 200 {"link_id": "l_…", "label": "studio", "device_enabled": true}
```

### `POST /v1/events`

Sends one notification to the linked device. Limited to 30 events per link
per minute.

```json
→ {
    "event_id": "h_4pgsopgu-42",
    "kind": "claude_permission",
    "title": "fix-tests",
    "subtitle": "studio",
    "body": "Bash: npm test",
    "session": "fix-tests",
    "ask_id": "toolu_01abc"
  }
← 200 {"sent": true}
← 200 {"sent": false, "reason": "duplicate"}
← 200 {"sent": false, "reason": "device_unregistered"}
```

| Field | Notes |
|---|---|
| `event_id` | unique per server, at most 100 of `A–Z a–z 0–9 _ . : -`. A repeat within 24 hours is acknowledged as `duplicate` and not sent again, so a server can retry safely |
| `kind` | from the table below; anything else is `400` |
| `title` | at most 100 characters; longer is cut |
| `subtitle` | optional, at most 100 characters |
| `body` | at most 500 characters |
| `session` | optional: the session's name, which the app opens when the notification is tapped. At most 64 of `a–z 0–9 -` |
| `ask_id` | `claude_permission` only, and required for it: the request the notification answers, at most 100 of `A–Z a–z 0–9 _ -`. Refused on any other kind |
| `encrypted` | sent instead of `title`, `subtitle` and `body`: the text, encrypted end to end, as base64url, at most 3000 characters. Sending it together with any of those three is `400` |

| `kind` | Category | Interruption level | Generic body |
|---|---|---|---|
| `claude_needs_input` | `CLAUDE_INPUT` | `active` | Claude needs your input |
| `claude_permission` | `CLAUDE_PERMISSION` | `active` | Claude needs your permission |
| `claude_turn_finished` | `CLAUDE_TURN` | `active` | Claude finished a turn |
| `claude_failed` | `CLAUDE_FAILED` | `time-sensitive` | A session failed |
| `test` | `TEST` | `active` | Test notification |

The generic body is what an encrypted notification shows when the phone can't
decrypt it, with the link's label as the title.

How `hurad` treats the answers:

| Response | Meaning | `hurad` |
|---|---|---|
| `200` | handled: sent, a duplicate, or the device is gone | done |
| `400` | malformed event | drops it |
| `401` | push token revoked or unknown | stops using the route until it is configured again |
| `429` | rate limited | retries after `Retry-After` |
| `502` | APNs is unavailable or overloaded | retries with backoff |
| other `5xx`, network errors | relay trouble | retries with backoff |

When APNs reports the device token as `Unregistered` or `BadDeviceToken`, the
relay disables the device until the app updates it with `PUT`. Until then it
answers events with `device_unregistered`.

## The APNs payload

```json
{
  "aps": {
    "alert": {"title": "fix-tests", "subtitle": "studio", "body": "Bash: npm test"},
    "sound": "default",
    "category": "CLAUDE_PERMISSION",
    "thread-id": "h_…:fix-tests",
    "interruption-level": "active"
  },
  "hura": {"kind": "claude_permission", "server_id": "h_…", "link_id": "l_…",
           "session": "fix-tests", "ask_id": "toolu_01abc"}
}
```

For an encrypted event, `alert` holds the generic text, `aps` adds
`"mutable-content": 1` so the app's notification service extension can
replace it, and `hura` adds `"enc": "<the encrypted field>"`.

Headers: `apns-push-type: alert` and `apns-priority: 10`. `apns-topic` is the
app's bundle ID, `dev.tobiaswadseth.hura.ios`, and `apns-expiration` is six
hours ahead. The JWT is ES256 with the key ID and team ID from the relay's
configuration, and is reused for 50 minutes.

## Encrypted text

With a `push_key` on its route, `hurad` sends the text only as ciphertext:

1. Clip `title` and `subtitle` to 100 code points and `body` to 500. A
   clipped string ends with `…`.
2. Serialize `{"title":…,"subtitle":…,"body":…}` as JSON, leaving out an
   empty `subtitle`.
3. Encrypt with AES-256-GCM, using the 32-byte key, a fresh random 12-byte
   nonce for every attempt, and this additional data (fields separated by
   `\n`, absent ones empty):

   ```
   hura-push-v1\n<server_id>\n<kind>\n<session>\n<ask_id>
   ```

4. Send `encrypted` = base64url without padding of nonce ‖ ciphertext ‖ tag.
   If that is longer than 3000 characters, halve the body's limit and encrypt
   again.

The additional data ties the ciphertext to its server, kind, session and
request. The relay therefore can't move one notification's text onto
another's, or point an Allow at a different request from the one whose text
the phone shows. The phone rebuilds the additional data from the `hura`
fields of the payload.

## `GET /v1/health`

`200 {"ok": true, "version": "…"}`. Needs no credential.
