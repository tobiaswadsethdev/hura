# hura for iOS

The phone side of hura, working toward what the desktop window does. A
server's worktrees are grouped under their projects; you can make a project
from a repository on the server, start a worktree in it with the same form
the desktop uses, and delete one. A chat session's conversations are live:
answer a permission request, a question or a plan, switch the permission mode
or the model, and open more conversations beside the agent's. A `/` lists the
session's slash commands with what each takes and does, and a message that
starts with `!` is a command run in the sandbox, whose output Claude reads with
the next message. A terminal
session's agent, and any shell beside it, is a terminal ([SwiftTerm](https://github.com/migueldeicaza/SwiftTerm)),
with a key bar for esc, ctrl, tab and the arrows. A permission request can
also be answered from its notification. Images in a chat are drawn, as on the
desktop: a screenshot a tool answered with, and a file of the working copy a
reply names. The **+** beside the message box sends photos from the library or
files from Files with the next message; a photo goes as a JPEG of at most
2048 pixels on its long side, which is as much as the model can use.

Beside the tabs, a worktree's dock has what the desktop's has: its files,
read-only, with images shown and zoomable; its git, to stage, commit, push and pull, with each file's diff;
review remarks on a diff's lines, sent to the agent as one message; the
events, grouped by endpoint, to allow (the whole host or only some paths) or
block from; its network policy, to remove rules from; its ports and
processes; and what it has spent. A port can be previewed: it is opened on
the phone's own loopback, at the same number when that's free, tunnelled to
the sandbox over the paired connection, and shown in Safari's in-app browser.

The Search tab searches every conversation the phone has seen, including
ended ones and deleted sessions: words in what was said, `file:` for the files
a tool call touched and `tool:` for the tool, as on the desktop. A result opens
its conversation at the message, or, when it exists nowhere else, what the
phone kept of it. The copy is the phone's own (`ChatHistory` in HuraKit),
kept in Application Support and out of backups, written as each entry arrives
and caught up from the host when a turn ends, before a session is deleted and
before a conversation is closed.

The Inbox tab is what is yours on GitHub, Azure DevOps and Jira: tickets
assigned to you or created by you, a section for each of a Jira tracker's JQL
filters, and pull requests you opened or that wait on your review. The
trackers are kept in a hura account ([docs/cloud.md](../../docs/cloud.md)),
which reads them all day with hura-core's own tracker code and tells the phone
through the relay when they change; signed out, the tab asks you to sign in. A
Jira ticket opens in full, to move its status, edit its fields and comment; a
pull request opens with its checks, reviewers and conversation, to reply, give
a review, and read its files. Either starts a worktree, on the branch the
desktop would give it.

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

## TestFlight

Builds go to TestFlight's internal group from a Mac, not from CI:

```sh
git switch main && git pull
apps/ios/Tools/testflight.sh
```

The script regenerates the project, archives a
Release build and uploads it with `Config/ExportOptions.plist`. It refuses a
branch other than `main` or a checkout with uncommitted changes
(`HURA_TESTFLIGHT_ANY_BRANCH=1` overrides both).

- **Signing** is automatic, with the team in `Config/Hura.xcconfig`. Xcode has
  to be signed in to an account on that team (Xcode > Settings > Accounts);
  the archive and the upload both use it.
- **The build number** is App Store Connect's to give
  (`manageAppVersionAndBuildNumber`), so `CURRENT_PROJECT_VERSION` in
  `project.yml` stays as it is. The version testers see is
  `MARKETING_VERSION`, bumped by hand when it should change. It is the app's
  own, separate from the server's release numbers.
- **After the upload** App Store Connect takes a few minutes to process the
  build and mails when it is done; internal testers get it then, with nothing
  more to do. A build that never appears was usually rejected in processing,
  which App Store Connect also says by mail.
- Builds from Xcode register with APNs as `sandbox` and TestFlight builds as
  `production` (see Notifications below), so notifications are worth checking
  on a TestFlight build rather than only a debug one.

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

Turn notifications on per server, in the server's Settings (its ⋯ menu), where its name on this phone can also be changed. The app registers
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

To look at the app with no server, `scripts/fake-hurad.py` stands in for one,
and for hura cloud: a few projects and sessions in every state, and a
conversation in each chat. It prints the launch command that pairs a Debug
build with it, with nothing to tap:

```sh
python3 scripts/fake-hurad.py
xcrun simctl launch booted dev.tobiaswadseth.hura.ios \
  -HuraPair 'hura://127.0.0.1:18555/fake-token#…' -HuraName devbox \
  -HuraCloud http://127.0.0.1:18556 -HuraNoLock YES -HuraOpen server
```

`-HuraOpen <session>` goes straight to a session's screen instead, and
`-HuraName` names the server, which is otherwise called by its address. These
arguments are read by Debug builds only. The screenshots on gethura.app are
taken this way; see `site/README.md`.

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

## Icons

hura's own icons are Lucide, the set the desktop draws in, so a branch or a
ticket is the same mark on both; standard iOS controls keep Apple's symbols.
`Hura/Views/Glyph.swift` names each one, and `Tools/lucide-symbols.sh` makes
them into custom symbols in `Hura/Assets.xcassets/Lucide` (it needs npm and
`brew install swiftdraw`). Add a case to `Glyph`, run the script, and check in
what it writes.

## The icon

`Hura/Assets.xcassets/AppIcon.appiconset/icon-1024.png` is made from the
desktop application's icon, `../desktop/src-tauri/icons/icon.svg`, by
`Tools/make-icon.swift`, which squares the tile's corners and crops it to a
full-bleed square, as iOS wants:
`swift Tools/make-icon.swift ../desktop/src-tauri/icons/icon.svg Hura/Assets.xcassets/AppIcon.appiconset/icon-1024.png`. The lock screen shows it too, from
`Logo.imageset`, which is the same image made small:
`sips -Z 336 Hura/Assets.xcassets/AppIcon.appiconset/icon-1024.png --out Hura/Assets.xcassets/Logo.imageset/logo@3x.png`.
