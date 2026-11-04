# Toolchains

A sandbox that can only clone and read is a sandbox that can only write code
nobody has compiled. The base image carries node and python, because the
template it is built on does, at whatever versions its Ubuntu froze. Anything
else (the .NET SDK, a Rust toolchain, a current node) is asked for per session:

```sh
hurad new --repo <url> --task "fix the failing test" --toolchain dotnet
hurad new --repo <url> --task "..."                  --toolchain dotnet,rust
hurad toolchains                                     # what is available
```

In the window it is a field on the create form, beside the policy, and it usually
arrives filled in: a checkout with a `Cargo.toml` in it comes up with `rust`
ticked, one with a `.csproj` a level down comes up with `dotnet`. `space`
toggles, and an answer you change by hand stays changed.

| Toolchain | What it installs | What it may reach |
| --- | --- | --- |
| `dotnet` | the newest .NET SDK, plus any SDK the checkout's `global.json` pins, in `/usr/local/dotnet` | `api.nuget.org`, read-only |
| `rust` | rustc, cargo, rustfmt and clippy, in `/usr/local/rust` | `index.crates.io` and `static.crates.io`, read-only |
| `node` | the newest node LTS release and its npm, in `/usr/local/node` | `registry.npmjs.org`, read-only |
| `playwright` | the system libraries Playwright's Chromium links, and no browser | `cdn.playwright.dev`, its mirror `playwright.download.prss.microsoft.com`, and the Chrome for Testing bucket on `storage.googleapis.com`, read-only |

"Newest" is resolved when the image is built: for .NET the newest SDK of the
newest channel in active support (10.0.403 when this was written), for node the
newest LTS (26.13.0). The base image's own node, Ubuntu's 22, stays at
`/usr/bin/node`; the node toolchain's goes ahead of it on `PATH`.

`playwright` is the odd one out: it installs no Playwright and no browser. The
browser build Playwright wants is pinned by the Playwright version a project
depends on, so `npx playwright install chromium` in the session fetches the
right one into the agent's home, from the hosts the toolchain opens. What a
session cannot fetch is the twenty-odd system libraries that browser links
(glib, the X11 client libraries, gbm and so on; measured, without them it
does not start), because installing them is `apt` from a host no session
reaches. The layer installs Playwright's own list for the base image's Ubuntu
release, so with it a page renders, with the base's fonts. It is about 200 MB,
most of it Mesa behind `libgbm1`, which is why it is a toolchain to ask for
and not part of the base. Firefox and WebKit need other libraries and are not
covered. The create form ticks it for a checkout with a `playwright.config.*`
at its root or a level down; Playwright tests deeper in a repository are ticked
by hand.

## Why the image, and not the sandbox

The agent has `sudo` in its sandbox, so nothing stops it writing to
`/usr/local`; the wall is the microVM around it, not the file permissions
inside. It still cannot install a toolchain, because no policy template lets a
sandbox reach a download host. Widening the policy far enough for
`dotnet-install.sh` to work would hand every session a route to arbitrary
tarballs on the internet, which is most of what the isolation is for.

So the toolchain is the image's business, exactly as the agent's own version
is, and for reproducibility as much as for the policy: every session asking for
`dotnet` gets the same SDK. It is resolved from the publisher's release manifest
at build time, checked against the checksum published beside it, and the build
fails rather than shipping something that did not verify.

## SDKs a checkout pins

A `global.json` can pin the SDK a repository builds with, and with
`"rollForward": "disable"` it accepts that exact version and no other: on an
image whose SDK is one patch newer, every `dotnet` command in the repository
fails, `dotnet --version` included. So a session asking for `dotnet` reads the
`global.json` files of the checkout it names (its root and one level down, the
same places the create form looks for project files), and when the SDKs in the
image do not satisfy a pin, the exact version pinned is added to the variant as
one more layer before the sandbox is made:

```
adding .NET SDK 10.0.301 to hura-sandbox:dotnet-node, pinned by the checkout's global.json ...
```

`rollForward` is honoured, so a pin of `10.0.400` with the default
(`latestPatch`) is met by 10.0.403 and adds nothing. The SDKs install side by
side and the `dotnet` host picks the one each repository asks for. An SDK added
this way stays in the variant for every later session, until the variant is
pulled or built again; the next session needing it adds it back.

The checkout is found by its `origin`: every repository under the configured
roots whose remote is the session's is read. That is the local working copy, so
a pin changed but not yet pushed is read the same as one that has been.

## One image per set of toolchains

Each set is its own tag, layered onto the base image:

```
hura-sandbox:latest        the base, what a session with no toolchain runs
hura-sandbox:dotnet        the base plus the .NET SDK
hura-sandbox:dotnet-rust   the base plus both
```

Docker shares the base's layers between all of them, so a variant costs its own
toolchains and not another copy of the base underneath. A Rust session never
carries the .NET SDK, and its policy never mentions nuget.

The tag is a pure function of the *set*, so `--toolchain rust --toolchain dotnet`
and `--toolchain dotnet,rust` name one image rather than building two identical
ones. Every set is published nightly beside the base (see
[sandbox-image.md](sandbox-image.md#published-images)), so the first session
asking for one pulls it, or it can be fetched ahead of time:

```sh
hurad image pull --toolchain dotnet,node
hurad image build --toolchain dotnet,rust    # or build it here
```

Either way it is then loaded into the sandbox runtime's own image store, which
is what a sandbox is made from. A create from the window pulls (or, failing
that, builds) and loads whatever variant it needs on the server, so the first
session asking for a new set of toolchains takes as long as that download;
fetching ahead of time is what makes it quick.

A variant is `FROM hura-sandbox:latest`, which means rebuilding the base for a
newer agent leaves the variants behind on the old one. Nothing about that looks
wrong from outside: sessions start, the toolchain works, and the agent is
whatever version it was. `hurad doctor` is what says so:

```
[  ok  ] image        hura-sandbox:latest built, claude 2.1.293
[ warn ] toolchains   hura-sandbox:dotnet older than hura-sandbox:latest, so still on its previous agent
         fix: hurad image build --toolchain dotnet
```

When they are current it reports what each one actually carries, read from a
manifest the layers write inside the image rather than inferred from the tag:

```
[  ok  ] toolchains   hura-sandbox:dotnet-node (dotnet 10.0.403, dotnet 10.0.401, node 26.13.0); hura-sandbox:rust (rust 1.98.0)
```

## The registries a toolchain may read

A toolchain is not only an install. `cargo build` on anything with a dependency
needs crates.io, and that endpoint is opened for the session that asked for the
toolchain and for no other. The registries of every toolchain a session has go
in one rule on its sandbox, and the policy pane shows it like any other rule:

```
  index.crates.io:443     GET, HEAD, OPTIONS   /**
  static.crates.io:443    GET, HEAD, OPTIONS   /**
```

Which also means `--toolchain` is a *policy* choice as much as an image one.

**Read-only, not open.** A registry fetch is thousands of unpredictable paths,
so an allow-list would either be wrong or be `/**`; "read anything here, write
nothing" is what is meant, and publishing a package is not something a
sandboxed agent should be able to do by accident.

**For the whole sandbox, not for cargo.** A sandbox's rules are about where a
request goes, not which program sent it, so anything in a `rust` session can
read crates.io, not only cargo. That is the same trade every rule makes, and it
is a small one here: what the rule opens is reading a public registry.

`hurad policy <name> --widen` is the same idea for a session that turns out to
need more: npm, PyPI and Docker Hub (`registry-1.docker.io`, `auth.docker.io`
and its two CDN hosts), read-only, added to a running session.

## What the layers put where

The toolchains themselves are in `/usr/local`, owned by root. Everything a build
*writes* goes under the agent's home, `/home/agent`:

| | |
| --- | --- |
| `CARGO_HOME` | `/home/agent/.cargo`: the registry index, the crate cache, `cargo install` output |
| `NUGET_PACKAGES` | `/home/agent/.nuget/packages` |
| `DOTNET_CLI_HOME` | `/home/agent/.dotnet` |

and the three the SDK is quietened with: `DOTNET_CLI_TELEMETRY_OPTOUT`,
`DOTNET_NOLOGO` and `NUGET_CERT_REVOCATION_MODE=offline`. `SSL_CERT_DIR` is
`/home/agent/.aspnet/dev-certs/trust:/usr/lib/ssl/certs`, for the HTTPS
development certificate: an ASP.NET Core frontend commonly runs `dotnet
dev-certs https --trust` before its dev server starts, and on Linux that writes
the certificate to `~/.aspnet/dev-certs/trust` for OpenSSL and then exits 4,
failing the dev server, unless the directory is in `SSL_CERT_DIR` (measured, on
a webpack config that does exactly that). The system's own directory is listed
after it because `SSL_CERT_DIR` replaces the default rather than adding to it;
the runtime's proxy CA is in the bundle `SSL_CERT_FILE` names, so outbound HTTPS
is unaffected.

These are set twice, as `ENV` in the image and as `set-environment` in
`/etc/tmux.conf`, the way the base image says its locale in both places. The
sandbox runtime passes an image's environment through to an exec, so `ENV`
alone now reaches the agent as well as a shell; the tmux lines are from a
runtime that did not, and cost nothing to keep.

The .NET SDK's telemetry and its first-run workload banner are both turned off,
for the reason the image already turns off Claude Code's auto-updater: the
sandbox denies the traffic behind them, and a denial with nothing worth
investigating behind it is noise in the events pane.

So is NuGet's *online* certificate-revocation check, and that one was measured
rather than predicted. A `dotnet add package Newtonsoft.Json` succeeded and left
six denials in the feed, all of them NuGet checking the signing certificates
against `www.microsoft.com/pkiops/crl/...`, `crl3.digicert.com` and
`ocsp.digicert.com`. The restore does not need them (the check is soft-fail,
which is why it worked), so the choice was between allowing three more hosts
and not making the request. `NUGET_CERT_REVOCATION_MODE=offline` does the
latter. Signature verification itself is untouched.

The dotnet layer installs no packages of its own: the base already has what the
SDK needs, and the layer checks that some `libicu` is there, by any version
(Ubuntu 26.04 has 78), rather than naming one that a newer base would not have.

Its install is a script, `images/hura-base/toolchains/dotnet-sdk`, copied into
the image rather than inlined in the layer, because the layer that adds a
checkout's pinned SDKs runs it too. It installs the newest SDK first and never
overwrites a file already there, so the `dotnet` host is always the newest one's
and an older SDK added later only adds its own directories. An older 10.0 SDK
does print "An issue was encountered verifying workloads" during a build, since
it finds the newer SDK's workload manifests; the build itself is unaffected.

## Adding one

`crates/hura-core/src/toolchain.rs` is one table, and a toolchain is an entry in it
plus a Dockerfile fragment under `images/hura-base/toolchains/`. The entry names
the registries it reads and the markers that make the create form tick it. The
tests in that module check the halves against each other: every layer records
itself in the manifest `doctor` reads, and no toolchain ships without a registry
it could fetch from. A registry is a host read whole, or, on a host shared with
everything else (Google's storage, for Playwright), one path on it.

---

[← Documentation](README.md) · [README](../README.md)
