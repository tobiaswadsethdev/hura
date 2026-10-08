# Toolchains

A sandbox that can only clone and read is a sandbox that can only write code
nobody has compiled. The base image carries node and python, because the
template it is built on does. Anything else (the .NET SDK, a Rust toolchain) is
asked for per session:

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
| `dotnet` | the .NET SDK, current LTS channel, in `/usr/local/dotnet` | `api.nuget.org`, read-only |
| `rust` | rustc, cargo, rustfmt and clippy, in `/usr/local/rust` | `index.crates.io` and `static.crates.io`, read-only |
| `node` | nothing; the base image already has it | `registry.npmjs.org`, read-only |

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
ones. It is built on first use, or ahead of time:

```sh
hurad image build --toolchain dotnet,rust
```

Like the base, a variant is built with Docker and then loaded into the sandbox
runtime's own image store, which is what a sandbox is made from. A create from
the window builds and loads whatever variant it needs on the server, so the
first session asking for a new set of toolchains takes as long as that build;
building ahead of time is what makes it quick.

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
[  ok  ] toolchains   hura-sandbox:dotnet (dotnet 9.0.317); hura-sandbox:rust (rust 1.98.0)
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
`DOTNET_NOLOGO` and `NUGET_CERT_REVOCATION_MODE=offline`.

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

## Adding one

`crates/hura-core/src/toolchain.rs` is one table, and a toolchain is an entry in it
plus a Dockerfile fragment under `images/hura-base/toolchains/`. The entry names
the registries it reads and the markers that make the create form tick it. The
tests in that module check the halves against each other: every layer records
itself in the manifest `doctor` reads, and no toolchain ships without a registry
it could fetch from.

---

[← Documentation](README.md) · [README](../README.md)
