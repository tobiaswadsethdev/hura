# hura for iOS

The phone side of hura, working toward what the desktop window does. A
server's worktrees are grouped under their projects; you can make a project
from a repository on the server, start a worktree in it with the same form
the desktop uses, and delete one. A chat session's conversations are live:
answer a permission request, a question or a plan, switch the permission mode
or the model, and open more conversations beside the agent's. A terminal
session's agent, and any shell beside it, is a terminal ([SwiftTerm](https://github.com/migueldeicaza/SwiftTerm)),
with a key bar for esc, ctrl, tab and the arrows. A permission request can
also be answered from its notification.

Beside the tabs, a worktree's dock has what the desktop's has: its files,
read-only; its git, to stage, commit, push and pull, with each file's diff;
review remarks on a diff's lines, sent to the agent as one message; the
events, grouped by endpoint, to allow (the whole host or only some paths) or
block from; its network policy, to remove rules from; its ports and
processes; and what it has spent. A port can be previewed: it is opened on
the phone's own loopback, at the same number when that's free, tunnelled to
the sandbox over the paired connection, and shown in Safari's in-app browser.

The Inbox tab is what is yours on GitHub, Azure DevOps and Jira: tickets
assigned to you or created by you, and pull requests you opened or that wait
on your review. A Jira ticket opens in full, to move its status, edit its
fields and comment; a pull request opens with its checks, reviewers and
conversation, to reply, give a review, and read its files. Either starts a
worktree, on the branch the desktop would give it. Trackers are set up on the
phone and their tokens stay in its Keychain: they are read by hura-core's own
tracker code, built for iOS from [`crates/hura-mobile`](../../crates/hura-mobile),
so no server holds a tracker or a token.

A server's menu has its integrations (MCP servers to start, restart or stop,
the secrets they're given, and the skills new sessions get), its settings
(what new sessions start from, and whether it updates itself), and what
versions are running. It talks to `hurad` over the same paired, pinned TLS connection the
desktop application uses, and receives notifications through the
[relay](../../relay). [docs/notifications.md](../../docs/notifications.md)
covers the `hurad` side.

| Part | What it is |
|---|---|
| `Hura/` | the SwiftUI app |
| `HuraKit/` | a Swift package with no UI: pairing, certificate pinning, the Keychain, `/rpc`, the chat and terminal channels on the websocket, and the push relay's client and encryption |
| `HuraNotificationService/` | a notification service extension that decrypts end-to-end encrypted notification text on the phone |

## Build

You need Xcode 16 or newer and [XcodeGen](https://github.com/yonaskolb/XcodeGen)
(`brew install xcodegen`). The project is generated from `project.yml`, and the
`.xcodeproj` is not checked in.

The app links hura-core's tracker client, built from Rust. Build it once,
and again after changing the Rust:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
apps/ios/Tools/build-core.sh    # makes HuraKit/Build/HuraCore.xcframework
```

```sh
cd apps/ios
xcodegen
open Hura.xcodeproj           # or build and test from the command line:
xcodebuild -project Hura.xcodeproj -scheme Hura \
  -destination 'platform=iOS Simulator,name=iPhone 17' -skipPackagePluginValidation test
cd HuraKit && swift test      # HuraKit alone, on the Mac
```

`Config/Hura.xcconfig` sets the bundle ID (`dev.tobiaswadseth.hura.ios`) and
the team. To sign under another team, override them in
`Config/Signing.local.xcconfig`, which git ignores:

```
HURA_BUNDLE_ID = com.yourname.hura
HURA_TEAM_ID = ABCDE12345
```

The APNs topic is the bundle ID, so a different bundle ID also needs a relay
of its own whose `APNS_TOPIC` matches it.

## Pairing

On the machine running `hurad`, mint a token whose address the phone can
reach. Over Tailscale, that is the machine's tailnet address or MagicDNS name:

```sh
hurad pair phone --host box.tailnet-name.ts.net
```

It prints a QR code above the `hura://…` line. Scan it on the app's pairing
screen, or with the Camera app, which opens the link in hura. Pasting the line
works too. The app shows the address and the certificate's fingerprint,
connects only to a server presenting exactly that certificate, and sends the
token only after it has. It then checks that the server accepts the token
before it keeps anything. `hurad revoke phone` on the server cuts the phone
off.

## Notifications

Notifications go through the relay in [`relay/`](../../relay). Deploy it
first (its README covers that), then set its host in `Config/Hura.xcconfig`:

```
HURA_RELAY_HOST = hura-relay.example.workers.dev
```

With it empty, the app builds without the Notifications section.

Turn notifications on per server, on the server's screen. The app registers
the phone with the relay, links the server, and gives `hurad` the link's push
token together with a fresh encryption key. From then on the relay and Apple
see only the kind of each notification, and the notification service
extension decrypts the text on the phone. The keys live in a Keychain group
shared with the extension. Server records, and their tokens, are kept out of
that group.

A permission request from a chat session arrives with **Allow** and **Deny**.
Allow needs the phone unlocked, because it runs something on the server;
Deny doesn't. Either answers that one request: if it has already been
answered elsewhere, the app posts a notification saying so instead.

Builds run from Xcode register as `sandbox` with APNs, and TestFlight builds
as `production`. The relay needs to know which, and the app tells it.

## Trying it in the simulator

The simulator can't receive real pushes, but it can show one sent with
`simctl`:

```sh
cat > payload.json <<'EOF'
{"Simulator Target Bundle": "dev.tobiaswadseth.hura.ios",
 "aps": {"alert": {"title": "fix-tests", "body": "Bash: npm test"}, "category": "CLAUDE_PERMISSION"},
 "hura": {"kind": "claude_permission", "server_id": "h_…", "session": "fix-tests", "ask_id": "toolu_…"}}
EOF
xcrun simctl push booted dev.tobiaswadseth.hura.ios payload.json
```

`server_id` has to be one this app has paired, and the app has to have been
allowed to show notifications first.

`HuraKit`'s live tests run against a real `hurad`:

```sh
HURA_PAIRING="$(hurad pair kit --host 127.0.0.1 | grep hura://)" swift test --filter LiveTests
```

## The icon

`Hura/Assets.xcassets/AppIcon.appiconset/icon-1024.png` is made from the
desktop application's icon by `Tools/make-icon.swift`, which crops it to a
full-bleed square, as iOS wants.
