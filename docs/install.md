# Installing hura

There are two things to install and they do not go in the same place. **`hurad`
runs where the sandboxes are**, which is Linux with KVM, because every session is
a microVM. **The desktop application runs where you are sitting**, which may be
the same machine or may be Windows. It makes requests of an `hurad` and needs no
sandbox runtime, no Docker and no tmux of its own.

`hura` was a second Linux binary until v0.4.0, carrying this CLI and a terminal
interface beside it. It folded into `hurad`: every command it had, `hurad` has,
and the window is the interactive surface now. Upgrading from v0.3.1 or earlier
means running `install.sh` once. `hura update` cannot cross the rename, because
there is no longer an `hura` for it to replace itself with.

Most of this page is the first half. [The desktop
application](#the-desktop-application) at the end is the second, and is all that
a Windows machine needs.

## Prerequisites

Linux with systemd, KVM and Docker. Verified on Arch on WSL2, where KVM is
available through nested virtualization; nothing here is portable to macOS.

| | |
| --- | --- |
| KVM | `/dev/kvm` usable by your user. On WSL2 it is there when nested virtualization is, which Windows 11 has on by default |
| [Docker Sandboxes](https://docs.docker.com/ai/sandboxes/) | `sbx` v0.47 or newer, signed in. Each session is one of its sandboxes |
| Docker | server 29.x, reachable by your user. It builds the sandbox image, which the sandbox runtime cannot |
| tmux | on the host, for `hurad attach` |
| Rust | 1.89 or newer, only to build `hurad` yourself (edition 2024, let-chains, `File::lock`) |

`hurad doctor` checks every one of them, plus the sandbox image, the runtime's own
health, its policy and whether systemd lingering is enabled, and says what to do
about whatever is missing:

```
[  ok  ] sbx          v0.47.0 (daemon v0.47.0)
[  ok  ] sbx checks   13 passed
[  ok  ] sbx policy   deny-all: only what a session's rules allow gets out
[  ok  ] sbx daemon   sbx-daemon.service
[  ok  ] docker       server 29.7.2
[  ok  ] tmux         tmux 3.7c
[  ok  ] linger       enabled
[  ok  ] image        hura-sandbox:latest built, claude 2.1.293
[  ok  ] templates    1 loaded
```

## Installing the pieces

**Docker Sandboxes.** Docker's own, under its own licence, and it wants a Docker
sign-in, which is why hura's installer does not install it for you. Docker
documents it for Ubuntu, but its release tarball is static, installs without
root, and runs on Arch as well:

```sh
# DockerSandboxes-linux-amd64.tar.gz from https://github.com/docker/sbx-releases/releases
mkdir sbx-dl && tar -xzf DockerSandboxes-linux-amd64.tar.gz -C sbx-dl
sbx-dl/docker-sbx/install.sh           # installs to ~/.docker/sbx
export PATH="$HOME/.docker/sbx/bin:$PATH"
```

`hurad` finds it there without the `PATH` change, which matters under systemd,
where a user service's `PATH` does not include it. Then sign in and choose the
policy every session's rules assume:

```sh
sbx login                              # prints a code and a URL to confirm it at
sbx policy init deny-all               # nothing leaves a sandbox unless a rule says so
```

`deny-all` is the one to pick. A session's template is a list of allows, which
only decides something on a runtime that refuses everything else; `balanced`
would add a baseline of hosts to every session, and `allow-all` would make the
rules decorative. `hurad doctor` says which is in force.

**Its daemon, as a unit of its own.** The runtime starts its daemon the first
time anything calls `sbx`, inside whatever made that call. When that is `hurad`
running as a systemd service, the daemon is in `hurad`'s cgroup, and restarting
`hurad` (which `hurad update` asks you to do) takes the daemon and every sandbox
with it. So it gets its own unit, [sbx-daemon.service](sbx-daemon.service):

```sh
cp docs/sbx-daemon.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now sbx-daemon
sudo loginctl enable-linger $USER      # WSL: or the daemon stops with your shell
```

Under it the daemon is its own unit's process, in its own cgroup; that was
measured, not assumed.

**Credentials.** One `[credentials.NAME]` table in `~/.config/hura/config.toml`
per credential the agents need, naming where its value comes from. hura never
reads the value: a session the credential is ticked for gets a secret of its own
from the runtime, which runs the command itself, and the sandbox only ever holds
a placeholder. A file only you can read is the simplest source:

```sh
mkdir -p ~/.config/hura/tokens && chmod 700 ~/.config/hura/tokens
umask 077
cat > ~/.config/hura/tokens/claude          # the token `claude setup-token` printed
cat > ~/.config/hura/tokens/azure-devops    # an Azure DevOps PAT, if you use one
```

```toml
[credentials.claude]
kind = "claude-code-oauth"
command = "tr -d '\r\n' < /home/me/.config/hura/tokens/claude"

[credentials.azure-devops]
kind = "azure-devops-pat"
command = "tr -d '\r\n' < /home/me/.config/hura/tokens/azure-devops"
```

Two details in those commands are load-bearing. The path is absolute, because
the runtime runs the command from a temporary directory of its own, so `~` and a
relative path are not guaranteed to mean what they do in your shell. And `tr`
strips the newline that `cat >` leaves at the end of the file, which would
otherwise end up inside an HTTP header. `command` can be any shell command that
prints the value (`pass show`, `op read`); `ref` takes a 1Password or AWS
Secrets Manager reference instead. [configuration.md](configuration.md) has the
kinds, and [git-hosts.md](git-hosts.md) the Azure DevOps details.

**`hurad` itself.** The policy templates and the whole image recipe (Dockerfile,
status hook, Claude settings) are compiled into the binary, so it needs nothing
from this tree at runtime. That is what makes a one-line install possible:

```sh
curl -fsSL https://raw.githubusercontent.com/tobiaswadsethdev/hura/main/install.sh | sh
```

It works out which release fits this machine, downloads it, **checks it against
the release's published `SHA256SUMS` and installs nothing if that does not
match**, puts `hurad` in `~/.local/bin`, and finishes by running `hurad doctor` so
the prerequisites above are named rather than discovered one at a time.

It does not remove an `hura` left over from an earlier install. Deleting a
binary somebody may still have running is not an installer's decision, but
nothing updates it any more, and `hurad` is what to run. Read it first if you
would rather not pipe a script into a shell: it is [install.sh](../install.sh) in
this repository, and downloading it and running it separately works exactly the
same.

Three things it takes, as flags or environment variables:

| | |
| --- | --- |
| `--bin-dir DIR` / `HURA_BIN_DIR` | where to install; default `~/.local/bin`, and it says so when that is not on your `PATH` |
| `--version vX.Y.Z` / `HURA_VERSION` | a specific release rather than the newest |
| `--from-source` / `HURA_FROM_SOURCE=1` | build with `cargo` instead of downloading |

Building it yourself is the other way, and the one to use from a checkout. It
is also the automatic fallback when no release is built for your architecture:

```sh
cargo install --path crates/hurad                                  # from a checkout
cargo install --git https://github.com/tobiaswadsethdev/hura hurad --locked   # without one
```

Then:

```sh
hurad image build                      # also happens on the first `hurad new`
hurad doctor
```

`hurad image build` builds `hura-sandbox:latest` with Docker and then loads it
into the sandbox runtime's own image store, which is where a sandbox is made
from; [sandbox-image.md](sandbox-image.md) is what is in it.

Start something: `hurad new --repo <url> --task "..." --provider claude`. For a
picker and a form instead of flags, that is [the desktop
application](#the-desktop-application), which is the interactive surface. There
was a terminal interface here until v0.4.0, and [desktop.md](desktop.md) is what
replaced it.

The desktop workspace talks to a server rather than to the sandboxes directly,
so it works whether they are on this machine or another one. It is the next
section; [desktop.md](desktop.md) is what the window does once it is running.

## The desktop application

A window onto an `hurad`. It holds no sandboxes and starts none itself: it dials
a server, pins that server's certificate, and asks. So the machine it runs on
needs none of the prerequisites above, and the server it dials can be this
machine, a box on the LAN, or the Linux side of the same laptop.

Whichever platform, the last step is the same: **the window pairs with a server
from its own screen**, so nothing above has to be installed beside it. Run
`hurad pair desktop --host <the address the window will dial>` on the server,
paste the `hura://…` line it prints into the window, and that is the install
finished. [desktop.md](desktop.md#connecting-it-to-a-server) is that step in
full, and [server.md](server.md) is the case where the two are on different
machines.

### Linux

Built from the tree. A Tauri bundle links against the webkit2gtk of the
distribution that built it, so a `.deb` or an AppImage published here would be a
promise about GTK versions it could not keep -- which is why the release page
carries a Windows installer and no Linux one.

The libraries, with their development headers:

| | |
| --- | --- |
| Arch | `sudo pacman -S webkit2gtk-4.1 gtk3 libsoup3 base-devel curl file openssl librsvg` |
| Debian, Ubuntu | `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev build-essential curl file libssl-dev librsvg2-dev` |
| Fedora | `sudo dnf install webkit2gtk4.1-devel gtk3-devel libsoup3-devel openssl-devel curl file librsvg2-devel` |

Tauri's own [prerequisites](https://v2.tauri.app/start/prerequisites/) page is
the list that is kept current; these are the three that matter -- `webkit2gtk-4.1`
is the engine, and the two bugs in [desktop.md](desktop.md#the-font-metrics-webkit-gets-wrong)
are its. Node 22 or newer and the same Rust as the rest of the tree are the
other two.

```sh
cd apps/desktop
npm install
npm run tauri build      # bundle in src-tauri/target/release/bundle/
npm run tauri dev        # or run it from the tree, which is what to do while working on it
```

**`npm run tauri dev` rather than the debug binary.** A development build loads
the frontend from Vite's dev server, and that is what starts it; running
`src-tauri/target/debug/hura-desktop` on its own gives a window that says
`Operation was cancelled` and reads exactly like a broken frontend.

### Windows

There is no `hurad` for Windows and there is not meant to be. The CLI drives
Docker, tmux and the sandbox runtime, and none of those are on that side; what
runs there is the window, which pairs itself. This is the arrangement the server was built
for: Linux in WSL doing the work, the window out on Windows.

Download the installer for the release you want from the [releases
page](https://github.com/tobiaswadsethdev/hura/releases) -- either of:

```
hura-desktop-vX.Y.Z-x86_64-pc-windows-msvc.msi          # Windows Installer
hura-desktop-vX.Y.Z-x86_64-pc-windows-msvc-setup.exe    # the same application, NSIS
```

Both are covered by the release's `SHA256SUMS`, the same file `install.sh`
verifies a Linux binary against, so an installer can be checked before it is
run:

```powershell
(Get-FileHash .\hura-desktop-vX.Y.Z-x86_64-pc-windows-msvc.msi -Algorithm SHA256).Hash.ToLower()
```

**A release built without a signing certificate is unsigned**, and SmartScreen
says so with a full-width warning before it will run one. The checksum above is
the integrity story either way -- it is the one that does not expire -- and the
release workflow signs the installers when a certificate is in the repository's
secrets (`WINDOWS_CERTIFICATE`, a base64 PFX, and
`WINDOWS_CERTIFICATE_PASSWORD`), skipping it when there is none so that a fork
can still cut a release.

WebView2 is the only runtime it needs, and Windows 11 ships with it; on Windows
10 the installer's own prompt or Microsoft's Evergreen bootstrapper supplies it.

Building it there instead needs Rust, Node 22 or newer, and the MSVC build tools
(the *Desktop development with C++* workload), then the same two commands as on
Linux. Only the client half of this repository compiles for Windows, which CI
checks on every change; `hurad` does not, and is not asked to.

**The window updates itself, once you say so.** It checks for a newer release
at launch, on focus after a few hours, and from *check* on the about screen
(the wordmark in the corner), and shows a badge when there is one; *install &
restart* downloads it, checks it against a signature made when the release was
built, runs the installer and relaunches. It never does that without being
clicked -- see [desktop.md](desktop.md#keeping-it-current).

Windows only, because Windows is the only platform with an installer to
replace: the Linux window is built from the tree, so there is nothing for an
updater to fetch.

**If the server is in WSL**, which is the case this was built for, the address
the window dials depends on how WSL is networked -- mirrored means
`localhost:17671` and NAT means an address that changes on every restart. `hura
doctor` on the Linux side says which is in force and what to dial. See [the WSL
case](server.md#the-wsl-case).

## Updating

`hurad update` is the install script's three steps performed by the binary that
is already there: read the release list, verify the download against
`SHA256SUMS`, and replace itself.

```sh
hurad update                 # to the newest release
hurad update --check         # say what that would do, and do none of it
hurad update --tag v0.1.0    # to one named release, to get back to one that worked
hurad update --force         # reinstall the version already running
```

The replacement is a rename over the running binary, which Linux allows and
which means a torn download cannot leave half an `hurad` on your `PATH`. A
session already running is untouched -- its agent lives in a sandbox, not in
this binary -- but the sandbox image is versioned separately, so `hurad image
build` after an update is what picks up a change to the image recipe.

`hurad doctor` reports when a newer release is out, whether or not anything is
going to act on it:

```
[ warn ] version      hurad 0.4.0; 0.4.1 is out
         fix: hurad update
```

### Updating without being asked

`hurad serve` looks for a newer release every six hours, and **downloads one
without installing it**. The verified binary waits beside the running one as
`.hurad-staged`, and the swap happens the next time `hurad` starts -- any start,
whether that is a `systemctl --user restart hurad` or your next `hurad ls`.

That split is the point. Replacing a binary is safe; replacing it *now* is not,
because a server driving four agents is a server somebody is using. Staging
costs one API call when there is nothing new, and when there is, the download,
the checksum and the version check all happen against a file nothing is
running.

A server that is already serving is never disturbed by the swap. Linux keeps a
running program on its old inode through a rename, so the process goes on as
the version it started as however many times the file underneath it changes;
what the new one is, is what the next start gets.

```toml
auto_update = false      # never reach github; `hurad update` still works by hand
```

The staged file is discarded rather than applied if it is not newer than what
is running -- stale after a manual `hurad update`, a downgrade, or a truncated
download that will not run. It never becomes newer, so carrying it around is
not worth the stat.

A binary installed with `cargo install` can still be updated this way, since it
is replaced where it stands. Going the other way -- back to a build from the
tree -- is `cargo install --path crates/hurad` again.

---

[← Documentation](README.md) · [README](../README.md)
