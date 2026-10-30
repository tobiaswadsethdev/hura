# The hura relay

How `hurad` reaches a phone: a Cloudflare Worker with a D1 database. It holds
the APNs key, accepts notification events from `hurad`, and is the only thing
that talks to Apple. The API is in [docs/relay-api.md](../docs/relay-api.md),
and what `hurad` sends, and when, is in
[docs/notifications.md](../docs/notifications.md).

## Develop

```sh
npm ci            # if npm 10.9 trips over this tree, use: npx npm@11 ci
npm test          # vitest inside workerd, with D1 and a stubbed APNs
npm run typecheck
```

The tests need no Cloudflare account and never reach Apple.

## Deploy

You need a Cloudflare account and, from the Apple Developer portal, an APNs
key (`AuthKey_XXXXXXXXXX.p8`) for the team that signs the hura iOS app. One key
works for every app in a team, so a key already used for another app's push
will do.

`wrangler.jsonc` already names the key ID, the team ID and the topic
(`dev.tobiaswadseth.hura.ios`, the app's bundle ID). Change them there if yours
differ.

```sh
npx wrangler login
npx wrangler d1 create hura-relay                 # copy database_id into wrangler.jsonc
npx wrangler secret put APNS_KEY_P8 < AuthKey_XXXXXXXXXX.p8
npm run deploy                                    # applies migrations, then deploys
```

The relay is then at `https://hura-relay.<your-subdomain>.workers.dev`. The
app is built with that host (see `apps/ios`), so this is the address phones
will register with.

## Check that the relay can reach APNs

This needs no app. Register a device with a made-up token and ask the relay for
a test push:

```sh
RELAY=https://hura-relay.<your-subdomain>.workers.dev
curl -s $RELAY/v1/devices -H 'content-type: application/json' \
  -d "{\"apns_token\":\"$(printf '0%.0s' {1..64})\",\"environment\":\"sandbox\"}"
# → {"device_id":"d_…","device_secret":"hud_…"}
curl -s -X POST $RELAY/v1/devices/<device_id>/test -H "authorization: Bearer <device_secret>"
```

| Answer | Meaning |
|---|---|
| `"apns_status":400,"apns_reason":"BadDeviceToken"` | The relay reached APNs over HTTP/2 and the key works. Only the fake token was refused |
| `"apns_status":403,"apns_reason":"InvalidProviderToken"` | The relay reached APNs, but the key ID, the team ID or the `.p8` is wrong |
| `"apns_status":0` | The relay never got an answer: Workers couldn't reach APNs |

Delete the test device afterwards:

```sh
curl -X DELETE $RELAY/v1/devices/<device_id> -H "authorization: Bearer <device_secret>"
```

## Try it with `hurad`, without the app

You can play the app's part with `curl` and point a running `hurad` at the
relay. The `push-configure` request has to come over a paired connection,
since a route belongs to the token it was set up with:

```sh
# register a device (a real token from a development build, or a fake one)
curl -s $RELAY/v1/devices -H 'content-type: application/json' \
  -d '{"apns_token":"<hex>","environment":"sandbox"}'
# link a server; the server ID is any h_ and 16 base32 characters you choose
curl -s $RELAY/v1/servers -H "authorization: Bearer <device_secret>" \
  -H 'content-type: application/json' -d '{"server_id":"h_aaaabbbbccccdddd","label":"studio"}'
# hand the push token to hurad, with a token from `hurad pair`, then send a test
curl -sk https://<hurad>:17671/rpc -H "authorization: Bearer <pairing token>" \
  -d '{"op":"push-configure","relay_url":"'$RELAY'","push_token":"hup_…","server_id":"h_aaaabbbbccccdddd"}'
curl -sk https://<hurad>:17671/rpc -H "authorization: Bearer <pairing token>" -d '{"op":"push-test"}'
```

`-k` is there because `curl` has no way to pin the certificate's fingerprint
the way hura's clients do. Use it only against a server you know.
