# The server

`hurad` drives the sandboxes on the machine it runs on, and serving is what lets
something else drive them: another machine on the network, a cloud box, or --
the case this was built for -- a Linux server inside WSL with the window out on
Windows. One binary does both, which is why `hurad` with no subcommand listens.
It runs [on Windows itself](install.md#running-the-server-on-windows) as well.

The sessions are the same sessions. One sandbox runtime, one cache, one set of
sandboxes, whether the thing asking is a terminal on that machine or a client
somewhere else.

```
   Windows                          WSL / a VM / a box on the LAN
   +-----------+                    +--------------------------------------+
   |  hura     |  TLS, one token    |  hurad  ->  Docker Sandboxes daemon  |
   |  (client) | -----------------> |             -> sandboxes (microVMs)  |
   +-----------+                    +--------------------------------------+
```

## Pairing

On the server:

```sh
hurad pair desktop            # one string, shown once
hurad serve                   # or run it under systemd; see below
```

`pair` prints a single line:

```
hura://box.lan:17671/8f3c...e21a#d8faa48b...e140
```

That is the address, a token, and the fingerprint of the certificate the server
will present. On a terminal it also prints the same line as a QR code above it,
for the [iOS app](../apps/ios/README.md) to scan. `--no-qr` leaves the code
out, and so does output that is not a terminal, so a script piping `pair`
gets the line alone. On the client:

```sh
hurad connect 'hura://box.lan:17671/8f3c...e21a#d8faa48b...e140' --name work
hura --server=work ls
```

`--server` alone is enough when only one server is paired. The name has to be
attached with `=`, because `hura --server work ls` cannot be told apart from a
server called `work`.

```sh
hurad remotes                  # what this machine is paired with
hurad remotes --forget work    # stop being
hurad tokens                  # what the server accepts
hurad revoke desktop          # stop accepting one, immediately
```

Both directions take effect at once: a token minted while the server is running
works on the next request, and a revoked one stops working on the next request.
Neither needs a restart.

## What works over a connection

Nearly everything, now. Listing and reconciling sessions; creating one; the
policy and the event feed; the working copy's files; git, including staging,
commit, push, pull and fetch; the diff and the review comments that go back to
the agent; and the agent's terminal and any shells beside it, streamed.

One thing is still the local machine's. **Attaching** with `hurad attach` hands
*this* terminal to the agent, which is a thing about the process you are
sitting in rather than a request; the desktop application's terminal is the
remote equivalent and it works over the connection.

Reading is `/rpc`, one request and one answer. The three things a client wants
*told* (the agent's screen, the sandbox's policy decisions as they are made, and
the terminal) are a websocket on `/ws`, multiplexed by channel, because polling
them would be a handshake per session per second to hear that nothing had
changed.

## Two machines

The server needs to listen somewhere the client can reach, and the pairing
string needs to name that address.

On the server:

```sh
# Listen on the network rather than on loopback. `--bind 0.0.0.0` is every
# interface; a single address is better where there is a choice.
hurad serve --bind 0.0.0.0

# In another shell: mint a token, and name the address the *client* should dial.
hurad pair laptop --host box.lan
```

On the client, paste the string it printed:

```sh
hurad connect 'hura://box.lan:17671/8f3c...#d8fa...' --name work
hura --server=work ls
```

**`--host` is the flag that matters, and leaving it out is the usual failure.**
The pairing string carries an address, and without `--host` that address is the
machine's own `/etc/hostname` (`hostname` on a Mac) -- which is frequently not a name the client can
resolve, and on a Debian-family box resolves *on the server itself* to
`127.0.1.1` while `hurad serve` is bound to `127.0.0.1`. The result is
`Connection refused` from a server that is running perfectly well, on the same
machine. Pass the address you want the client to dial:

```sh
hurad pair laptop --host 127.0.0.1     # same machine
hurad pair laptop --host box.lan       # over the network by name
hurad pair laptop --host 10.0.0.7      # ... or by address
```

**The certificate's names do not matter to the client.** The client judges a server
by the fingerprint in the pairing string and by nothing else -- `verify_server_cert`
ignores the name it was given -- so there is no "certificate is not valid for
this host" to run into, whatever address you dial. That is the point of pinning:
a self-signed certificate has no authority to appeal to, and a name check
against one proves nothing anyway.

`--san` is therefore for *other* clients: `curl`, a browser, anything that does
check names against a certificate. It is repeatable:

```sh
hurad serve --bind 0.0.0.0 --san hura.internal
```

Two things to know if you do use it. The certificate is generated once and then
reused whatever the SANs say, so a `--san` added after the first start does not
appear in it. And `hurad pair` generates the certificate too, with the defaults
only -- so pairing before the first `serve --san` locks the extra name out.
Delete `~/.local/state/hura/cert.pem` and `key.pem` and start again, which means
pairing again: the fingerprint every client holds has changed.

Two more things that are not hura's to fix but look exactly like it:

* **The port has to be open.** 17671/tcp on the server's firewall.
* **Both ends need the same protocol version.** `GET /version` answers without a
  token, so a client says "this server speaks 2, I speak 1" rather than failing
  in the middle of a request. `hurad doctor` on the client checks every paired
  server, which is where a moved address, a revoked token or a version skew
  shows up.

The desktop application reads the same paired servers as the CLI: pair once with
`hurad connect` and the window lists that server without being told again. It can
also *be* the thing that pairs -- paste the string into its servers screen,
which runs the same checks and writes the same file. That is what a Windows
client does, having no `hurad` to run:
[desktop.md](desktop.md#connecting-it-to-a-server).

## The WSL case

A server in WSL and a client on Windows is two networking arrangements that look
identical from inside the Linux side, and getting it wrong looks exactly like a
firewall problem:

* **mirrored** -- WSL shares the Windows network stack, and the client uses
  `localhost:17671`.
* **NAT**, the default -- the client uses the address of the WSL VM, *which
  changes every time WSL restarts*. Every restart then needs pairing again.

Only the window goes on the Windows side -- there is no `hurad` there, and the
pairing is done from the window itself. [install.md](install.md#windows) is the
installer.

`hurad doctor`, on the Linux side, says which one is in force and what to dial:

```
[  ok  ] wsl          mirrored networking: a client on Windows uses localhost:17671
```

Mirrored is worth turning on for this, in `%USERPROFILE%\.wslconfig`:

```ini
[wsl2]
networkingMode=mirrored
```

`hurad doctor` also checks every paired server, which is where an address that has
moved or a token that was revoked shows up:

```
[ FAIL ] servers      work: could not reach the server: box.lan:17671: Connection refused
         fix: check it is running, and that `box.lan:17671` is the address this
              machine should dial. `hurad remotes --forget work` drops it
```

## Shared previews

A client's preview of a port in a sandbox is normally its own: the desktop
and the phone each listen on their own loopback and tunnel what arrives there
over the paired connection ([desktop.md](desktop.md#previews)). That needs the
client running, and iOS stops an app's listeners soon after it leaves the
screen, so a page the phone handed to Safari stopped answering.

So the phone asks for a **shared** preview instead (`SharePreview` on `/rpc`).
The server gives the port a listener of its own, on the address `hurad serve`
listens on and at a port it picks, and answers with that port and a token.
The phone opens `https://<server>:<port>/?hura_preview=<token>` in Safari:

- **The token becomes a cookie.** The first request swaps it for a `Secure`,
  `HttpOnly` cookie and sends the browser on without it; a request with
  neither is refused. Tokens are 32 random bytes, made for the share.
- **It is this server's certificate,** the self-signed one the clients pin.
  Safari can't pin, so it warns once per preview and has to be told to go
  on. That was chosen over plain HTTP, which would put the page and the token
  on the network in the clear.
- **The page sees localhost.** Requests reach the service with `Host` (and
  `Origin`) saying `localhost:<port>`, which dev servers such as Vite insist
  on, and a redirect back to `http://localhost:<port>/...` arrives as a path.
  Websockets pass through, so a dev server's reload works.
- **It ends when unused.** After 30 minutes with no request it is stopped;
  asking again makes a new one, with a new token. Stopping the port's forward
  or the process behind it stops its shares too.

A share is reachable wherever the server's address is, so `--bind` decides who
can reach the listener, as it does for `/rpc`; the token decides who gets in.
An older server doesn't know the request, and the phone falls back to its own
preview in the in-app browser.

## Queued and scheduled sessions

A session can start later instead of now: at a time, or once fewer than
`queue_limit` sessions (2 unless the config file says otherwise) are creating,
seeding or running. Starting a worktree from an app offers **Now**, **When one
is free** and **At a time**; from the terminal it is a flag:

```sh
hurad new --task "Upgrade the dependencies" --at 02:00   # the next 02:00 here
hurad new --task "Fix the flaky test" --queue             # when there is room
hurad queue                  # what is waiting
hurad queue start 3          # start #3 now, whatever the limit says
hurad queue rm 3             # take it off
```

`--at` takes `02:00`, `2026-11-20 14:30`, or `+90m`, `+2h`, `+1d`. A session
scheduled for a time starts then whether or not the queue has room; one that is
waiting on someone (a permission request, a question) does not count as
running.

Messages can wait too. `hurad queue say <session> --at 08:00 "Rebase on main and
rerun the tests"` -- or the clock beside a chat's send button -- is said to the
session's agent at that time, or when its turn ends if it is busy then. Without a
time it is said as soon as the agent has finished what it is doing. Several
for one session are said one at a time, in the order they were written.

All of it is kept in `~/.config/hura/queue.json` and acted on by `hurad serve`,
so it happens with nothing connected, and carries on after the server or the
machine restarts: something that was being started when the server stopped is
started again unless its session had already been made. Both apps list the
queue in the project tree, and a chat shows what is waiting to be said to it.

## What this costs

**An authenticated client can create sandboxes, which are microVMs, on the
server's host, which makes a token equivalent to a login to that machine.**
That is not a qualification, it is the shape of the thing: the point of the
server is to start sandboxes, and starting sandboxes is a privileged act. Treat
a pairing string the way you would treat an SSH private key.

It goes one step further than sandboxes, in two ways worth naming. A client
may start, restart and stop the **managed MCP containers** this server runs, and store the secrets
they are given -- names and values in, names only ever out; see
[mcp.md](mcp.md). And a client can read your **tickets**, which means this
server holds a credential that can read them and comment on them; see
[tickets.md](tickets.md). None of them is a hole in the token: all of them are
things the token authorises, which is why a pairing string is a login.

`hurad` therefore listens on `127.0.0.1` unless told otherwise, and says so when
told otherwise:

```
listening on https://0.0.0.0:17671 -- reachable from the network. An
authenticated client can create containers on this host, so treat a token as a
login to it
```

Two things follow from that being the deal:

* **Tokens are named and revocable.** One per client, so losing a laptop costs
  that laptop's access and nothing else. They are 32 bytes from the OS, kept as
  a SHA-256 hash, and compared in constant time -- the file on the server holds
  nothing that can be presented as a credential.
* **The certificate is pinned, not trusted.** There is no authority to appeal
  to, so the client checks the server's certificate against the fingerprint in
  the pairing string and accepts nothing else. That covers the *first*
  connection as well as later ones, which ordinary trust-on-first-use does not.
  If the server is rebuilt and generates a new certificate, the client refuses
  and says both fingerprints; pair again.

Keys, tokens and saved connections live in `$XDG_STATE_HOME/hura` (usually
`~/.local/state/hura`), owner-readable only -- not in `~/.config/hura` beside the
session cache, because config directories are the ones people sync between
machines.

## Running it under systemd

A user service, ordered after the sandbox runtime's daemon, which has a unit
of its own ([install.md](install.md#installing-the-pieces) is that one):

```ini
# ~/.config/systemd/user/hurad.service
[Unit]
Description=hura server
After=sbx-daemon.service
Wants=sbx-daemon.service

[Service]
ExecStart=%h/.local/bin/hurad serve
Restart=on-failure

[Install]
WantedBy=default.target
```

```sh
systemctl --user enable --now hurad
loginctl enable-linger "$USER"    # so it survives logging out
```

The daemon has to be that unit rather than something `hurad` starts. The
runtime starts its daemon on the first `sbx` call, in the caller's cgroup, so a
daemon `hurad` started would be part of `hurad`'s service, and `systemctl --user
restart hurad` (which `hurad update` asks for) would stop it and every sandbox
with it. `hurad doctor` warns when `sbx-daemon.service` is not active.

`hurad` does not fail to start when the sandbox runtime is down: a server you
cannot reach is a server that cannot tell you why. It says so (`the sandbox
runtime did not answer at startup`) and carries on.

## Running it on a Mac, under launchd

A Mac runs `hurad` for [worktree sessions](worktrees.md) beside its sandboxes.
A LaunchAgent starts it at login, starts it again if it stops, and is what a
restart is asked of:

```xml
<!-- ~/Library/LaunchAgents/dev.hura.hurad.plist -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.hura.hurad</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/YOU/.local/bin/hurad</string>
    <string>serve</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/Users/YOU/Library/Logs/hurad.log</string>
  <key>StandardErrorPath</key><string>/Users/YOU/Library/Logs/hurad.log</string>
</dict>
</plist>
```

```sh
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.hura.hurad.plist
launchctl kickstart -k gui/$(id -u)/dev.hura.hurad    # restart it
launchctl bootout gui/$(id -u)/dev.hura.hurad         # stop it for good
```

A release `hurad serve` downloads is applied at its next start, so a
`kickstart -k` is also how an update takes effect. Releases are built for
Apple silicon; `install.sh` and `hurad update` pick that build on one.

## Running it on Windows, under Task Scheduler

A task that starts `hurad serve` when you log on, as you, and starts it again
if it stops. As you and in your logon session, rather than "whether the user is
logged on or not", because that mode has no password to open what Windows
keeps for you: the sandbox runtime's sign-in and Git's credentials among it.

```powershell
$action = New-ScheduledTaskAction -Execute "$HOME\.local\bin\hurad.exe" -Argument serve
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit 0 -AllowStartIfOnBatteries `
  -DontStopIfGoingOnBatteries -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1)
Register-ScheduledTask -TaskName hurad -Action $action -Trigger $trigger -Settings $settings
Start-ScheduledTask hurad
```

```powershell
Stop-ScheduledTask hurad; Start-ScheduledTask hurad    # restart it
Unregister-ScheduledTask hurad                          # stop it for good
```

The task runs `hurad.exe` itself, with nothing in between. Task Scheduler
gives a console program a console window, and the usual ways of hiding one
(`conhost --headless`, a hidden `cmd`, a script) put a process between the
task and the server; stopping the task then ends only that process, and the
server went on holding its port. Instead `hurad serve` notices that the
console it was given has nobody else on it, lets go of it, and writes what it
would have printed to `%LOCALAPPDATA%\hura\hurad.log`. A window shows for the
moment that takes. Run from a terminal, it prints there as usual.

A release `hurad serve` downloads is applied at its next start, so a restart
is also how an update takes effect.

---

[← Documentation](README.md) · [README](../README.md)
