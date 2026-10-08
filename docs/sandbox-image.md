# The sandbox image, and the agent inside it

Each agent runs under a tmux session *inside* its own sandbox, so it keeps
working whether or not anything is attached to it.

## What it is built on

`hura-sandbox:latest` is `FROM docker/sandbox-templates:shell-docker`, Docker's
template for a sandbox with a shell in it: Ubuntu with git, node, python, uv, gh
and socat, and a Docker Engine of its own. The workload runs as the template's
`agent` user (UID 1000, `HOME=/home/agent`), who has `sudo`. The boundary is the
microVM around the sandbox, not the file permissions inside it.

hura adds tmux, Claude Code at a pinned version, `hura-status` and `hura-usage`,
`hura-agent` with the Agent SDK beside it, and Claude Code's settings in
`/home/agent/.claude/settings.json` and `/home/agent/.claude.json`. It makes
`/sandbox` and gives it to `agent`; everything hura puts in a session is under
it, with the clone at `/sandbox/repo`. The whole image is 718 MB, where the one
it replaces was 5.2 GB. It is called `hura-sandbox` rather than the old
`hura-base` so that an image built for OpenShell cannot be taken for it, though
its recipe is still in `images/hura-base/`.

It has to stay a `FROM`. The template's labels carry over to the image, and two
of them, `com.docker.sandboxes.flavor=shell-docker` and
`com.docker.sandboxes.start-docker=true`, are what make the runtime start the
Docker Engine inside it. A copy of the template's contents would lose them.

The sandbox runtime cannot build an image, so `hurad image build` builds it with
Docker and then hands it over: `docker save` into `sbx template load`, since a
sandbox is made from the runtime's own store. On a 5 GB image that measured
about 23 seconds to save and 3.5 to load. The image counts as ready only when
the runtime holds the same image id Docker built, and is loaded again
otherwise, so a rebuild cannot leave sessions starting from the previous one. A
create builds or loads it when it has to. `hurad doctor` checks both halves:
`image` for what Docker built, `templates` for what the runtime holds.

## Claude Code's settings

The image bakes a `settings.json` for the agent, because a fresh sandbox has a
fresh `HOME` and an agent that has to be configured on arrival is an agent that
stops to ask:

| | |
| --- | --- |
| `model` | `opus[1m]` -- an alias, so it follows the newest Opus and keeps the million-token context |
| `permissions.defaultMode` | `auto`, so the agent handles its own permission prompts |
| `attribution` | `commit` and `pr` both empty, so nothing is stamped -- an empty string is what silences it, where an absent key means the default trailer |
| `copyOnSelect` | off. Not a `settings.json` key -- it lives in the global `.claude.json` the image also writes, and defaults to on; selecting text to read it should not take the clipboard of a terminal you are borrowing |
| `env` | the auto-updater, non-essential traffic and the plugin marketplace, all off |
| `hooks` | the status reporter, so the state column has something to read |

`.claude.json` also marks onboarding as done and trusts `/sandbox` and
`/sandbox/repo`, so a session does not open on the theme picker and then the
"do you trust this folder?" prompt.

**Auto mode** is Claude Code's own middle setting: it judges each tool call and
executes what it considers safe, rather than stopping for every edit
(`acceptEdits` stops for everything that is not one) or not asking at all
(`bypassPermissions`). Claude Code's own advice is to use it "only in isolated
environments", which is the one thing hura can actually promise -- and it is the
whole reason to run several agents at once, since an agent that stops on the
first edit is an agent you are still babysitting. `Shift+Tab` inside a session
changes it, and `/model` changes the model, for that session.

The three environment variables are all there because the sandbox *denies* the
traffic behind them, and a denial with nothing worth investigating behind it is
noise in the events pane. With them set, a session that clones, edits and answers
produces a feed with no denials in it at all.

**`ANTHROPIC_API_KEY` is taken away.** The sandbox runtime sets it to a sentinel
(`proxy-managed`) in every exec, for an Anthropic integration of its own that
hura does not use. Claude Code seeing it stops on a "Detected a custom API key"
prompt, and would prefer it to the session's own credential. So the agent's
start scripts unset it, and `/etc/tmux.conf` has `set-environment -gu
ANTHROPIC_API_KEY`, since a tmux server keeps the environment of whichever exec
started it.

**The image's `ENV` reaches an exec now.** OpenShell stripped it, which is why
the locale (`LANG`, `LC_ALL`) and `COLORTERM` are also said in `/etc/tmux.conf`
and in the script `hurad attach` runs. The sandbox runtime passes it through,
and those copies stay so that nothing depends on which one ran.

## The agent's version

`hurad image build` installs the newest Claude Code release, because the
template has none and the agent should not be the one picking its version: a
session under `feature-work` cannot reach the download service, and a
self-update would leave each session on whatever it found that day. The version
is resolved on the host and passed in as a build arg, so a rebuild really does
fetch what is newest instead of being answered from a cached layer, and the
download is checked against the release manifest's SHA-256. `hurad doctor`
reports what the built image carries and warns when a newer release is out.

The Dockerfile's `CLAUDE_VERSION` build arg pins a specific one, building from a
checkout:

```sh
docker build --build-arg CLAUDE_VERSION=2.1.293 -t hura-sandbox:latest images/hura-base
```

The next create sees that the runtime does not hold that image and loads it.
(`hurad image build` would build the newest again.) That has been needed once
already: Claude Code
2.1.294 was released before an Agent SDK release paired with it, and the build
refuses a Claude Code that no SDK release names (see below).

## The chat agent's host

A chat session's agent is not a terminal, and the image carries what runs it:
`hura-agent`, in `/usr/local/lib/hura-agent` beside the Agent SDK, with a link
on `PATH`. The seeder starts it in the agent's tmux session where a terminal
session gets `claude`, and it runs Claude Code through the SDK with
`pathToClaudeCodeExecutable` set to the image's own `/usr/local/bin/claude`, the
same binary a terminal session runs, under the same rules.

The SDK release is chosen by the Claude Code version the image installs: every
release names the one it was made with in its `claudeCodeVersion`, and the two
speak a control protocol to each other. Its own copy of Claude Code, an optional
dependency, is not installed. Like `claude`, it is owned by root.

The image is labelled `hura.chat`, which is what lets the first chat session on
an older image rebuild it rather than wait for a host that is not there, and what
`hurad doctor` checks.

Three things measured against Claude Code 2.1.289 rather than read in the SDK's
types. Turn state (idle, running, waiting on you) is only sent when
`CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS` is set, which the host sets. The
rate-limit windows arrive in a `rate_limit_event` under `unifiedWindows`, as
fractions, with the documented `utilization` absent. And `hura-status` stands
aside when `HURA_CHAT` is set, because the hooks in `settings.json` still fire
under the SDK and would otherwise write over the host's own, better informed
status.

## Docker inside a session

The template's Docker Engine runs in the sandbox, so an agent can build and run
containers: `docker run hello-world` and a Compose v5 project both worked in a
session, once the registries were open (`hurad policy <name> --widen` opens
Docker Hub with npm and PyPI, read-only). The inner engine's own traffic goes
through the sandbox's proxy and policy like everything else.

Containers it starts do not get the proxy variables the sandbox has. Their
traffic is still proxied, transparently, and held to the same rules, but a host
that has method and path rules (the git hosts under `feature-work`, for one)
is only reachable through the proxy by name. A container that needs one wants
`-e HTTPS_PROXY=http://gateway.docker.internal:3128`, and to trust the proxy's
CA certificate, which the sandbox keeps at
`/usr/local/share/ca-certificates/proxy-ca.crt`.

## The sandbox stays up

A sandbox stops itself 30 seconds after its last exec unless it is detached.
hura detaches every session's sandbox when it creates it, so an agent working
with nothing attached is not stopped under it.

## Toolchains

The image above is the *base*, and what a session with no toolchain runs. A
session that has to compile something runs a variant of it (`hura-sandbox:dotnet`,
`hura-sandbox:dotnet-rust`), built by layering the toolchain onto this image, so
Docker shares the layers underneath and a Rust session does not carry the .NET
SDK.

A toolchain is installed here for the reason Claude Code's version is: no
policy template lets a sandbox reach a download host, so the agent could not do
it from inside, and widening the policy far enough that it could would defeat
the point. [toolchains.md](toolchains.md) is the whole story: the variants, the
registries each toolchain opens, and what to change to add one.

## The status line, and why the image has one

For a terminal agent; a chat agent has no status line, and its host writes the
same file from what the SDK reports.

Claude Code hands out what it knows about cost and rate limits in exactly one
place: the `statusLine` command it invokes on every render, with a JSON payload
on stdin. There is no file it keeps them in and no endpoint to ask.

So the image bakes in `hura-usage` and points `statusLine` at it. It writes the
payload to `/sandbox/.hura/usage.json`, where the same poll that reads the hook
file picks it up, and prints the line the agent shows:

```
  Opus 5 (1M context)  $0.07  5h 32%  ctx 2%
```

The whole payload is kept rather than a selection of fields, because the shape
belongs to Claude Code and grows -- and the reader takes what it recognises, so
an older agent shows less rather than nothing. Two things worth knowing, both
measured rather than assumed: `rate_limits` only appears once the agent has
actually called the API, so a session sitting at a login prompt has none; and
`resets_at` is epoch seconds, where the obvious reading of the changelog is an
ISO instant.

---

[← Documentation](README.md) · [README](../README.md)
