# Architecture

A tour of the code, for anyone about to change some of it. [PLAN.md](../PLAN.md)
has the decisions and the increments that got here; this is the map.

## The shape of it

```
      +-------------------------+     +---------------------------+
      |   hurad (the CLI)        |     |  apps/desktop (webview)   |
      | ls | diff | policy      |     |  projects | tabs | dock   |
      |    | events | attach   |     +-------------+-------------+
      +-----------+-------------+                   | tauri commands
                  |                   +-------------+-------------+
                  |                   |  hura-client (pinned TLS)  |
                  |                   +-------------+-------------+
                  |                                 | https + wss
                  |                   +-------------+-------------+
                  |                   |  hurad  (/rpc, /ws)        |
                  |                   +-------------+-------------+
                  |                                 |
                +-+---------------------------------+-+
                |             hura-core                |   nothing here draws
                |  ops, sessions, policy, events,     |
                |  seed, git, files, projects         |
                +-----------------+-------------------+
                                  |
                    +-------------+-------------+
                    |     Backend (a trait)     |   where a session runs
                    +-------------+-------------+
                                  |
                              Sandboxed
                                  |
   SessionStore              sbx-client
   (~/.config/hura/         (CLI subprocess)
    sessions.json)                |
                   Docker Sandboxes daemon (sbx)
                   and its proxy, on this machine
                           |
        +------------------+-------------------+
        |                  |                   |
    microVM A          microVM B           microVM C
   clone+agent        clone+agent         clone+agent
   +Docker Engine     +Docker Engine      +Docker Engine
```

Five crates, and an application:

| | |
| --- | --- |
| `crates/sbx-client` | everything the rest of the tool knows about Docker Sandboxes, behind one trait, so a change in the CLI's surface lands in one file. Measured rather than read off its documentation; the module comment says what was measured. Sessions ran on OpenShell before this, through a crate of the same shape |
| `crates/hura-core` | everything `hura` *does*: sessions, policy, events, seeding. No renderer may appear in it, which is what lets something other than a terminal sit on top |
| `crates/hura-proto` | one definition of every message on the wire, so a server and a client cannot drift. Built on `hura-core`, because the types it carries are the core's own rather than a second set kept in step by hand |
| `crates/hurad` | the whole Linux side: the clap CLI, and the server behind TLS, one token check and `/rpc`. Async only where it has to be -- everything it calls is blocking and goes to `spawn_blocking`. `crates/hura` carried the CLI and a ratatui TUI beside it until v0.4.0, when two front ends onto one set of sessions stopped earning their keep |
| `crates/hura-client` | the client half: paired servers, and one certificate-pinned connection to each. Its own crate because the CLI and the desktop application both need it, and a webview cannot pin a certificate for itself |
| `apps/desktop` | Tauri v2 and React. Deliberately *not* a workspace member: `cargo build --workspace` would otherwise need a GUI toolkit installed to check that a session store reconciles |

## Where things live

The module docs at the top of each file are the real documentation; this is the
index into them.

Everything is in `hura-core` unless the second column says otherwise.

| Module | What it owns |
| --- | --- |
| `main.rs` | *(hura)* the clap CLI, and dispatch into `ops` |
| `ops.rs` | the operations the CLI and the window both need, so neither reimplements the other. Everything here takes a `Backend` rather than a runtime client |
| `backend.rs` | *where* a session runs, as a trait, so the scripts never name the runtime. `backend/sandboxed.rs` is the one implementation: making a sandbox, its rules, its secrets, finding hura's among the runtime's. There used to be a second, a `git worktree` on the server with no isolation; it was removed to keep one of everything |
| `session.rs` | what a session *is*: identity, the derived branch and sandbox names, and the metadata record written inside the sandbox |
| `store.rs` | the local cache and its reconciliation against the sandboxes that exist; every write is locked |
| `removed.rs` | the names of destroyed sessions, kept until the sandbox behind them has actually gone. Deletion is asynchronous, and a sandbox still listed with its record already dropped is the exact shape of an orphan worth adopting -- so without this a removed session came back on the next refresh, and the name could not be used again |
| `seed.rs` | the detached script that clones, cuts the branch, writes the record and starts the agent |
| `status.rs` | what the agent is doing, from hooks and from its screen |
| `chat.rs` | a chat session: the agent driven through the Agent SDK by `hura-agent` inside the sandbox. The frames between it, the server and the window, the script that starts it, and the execs that list, open and close conversations. The SDK's own messages are carried whole rather than modelled |
| `policy.rs` | the templates, the mid-run widen/tighten, and `View`: the policy pane as facts, which each renderer words for itself |
| `endpoints.rs` | the global allow and block lists applied to every new session |
| `events.rs` | the allow/deny feed, merged and kept on disk per session |
| `forge.rs` | which git host a session works against, derived from the repo URL |
| `tracker.rs` | reading tickets from GitHub, Azure DevOps and Jira over REST, with a token handed in -- the desktop application calls it with its own, and the server never does. The parsers are pure and tested against captured answers, because reading somebody else's JSON is the part that is easy to get wrong quietly |
| `image.rs` | the sandbox image, with its whole build context embedded in the binary; built by Docker and loaded into the runtime's own store |
| `credentials.rs` | the credentials a session can be given, and the sandbox-scoped secret each becomes. hura holds a command or a reference, never a value |
| `toolchain.rs` | the toolchains, their image variants, and the registry each one opens |
| `skills.rs` | packing host skills into a session, and the server-side library a client pushes its own into |
| `mcp.rs` | MCP servers on the host, and the endpoints granted for them. `mcp/managed.rs` is the half `hurad` runs itself: an image and a port instead of a url, the container's lifecycle, and what Docker says about it |
| `secrets.rs` | the values a managed MCP container is given. In one way only: `get` is `pub(crate)`, so no request handler can reach a value |
| `integrations.rs` | the MCP catalog, the secret names and the skill library as one answer, which every action on that screen returns |
| `repos.rs` | the git repositories on the machine `hurad` runs on -- the only module that reads that host's filesystem |
| `projects.rs` | the repositories someone has said they are working on, which is what worktrees are grouped under. A decision, not a discovery: stored rather than derived from the sessions that exist |
| `git.rs` | the working copy inside a sandbox as git describes it, and the operations on it. The status parser is pure, because git's output is the part that is easy to get subtly wrong and impossible to notice |
| `files.rs` | reading a worktree's files from outside it, one directory at a time. Read-only: the agent owns the working copy |
| `comments.rs` | review comments on a diff, kept per session until they go to the agent as one message |
| `config.rs` | `~/.config/hura/config.toml`, and which default wins |
| `doctor.rs` | the preflight checks, each carrying its fix |
| `update.rs` | fetching, verifying and replacing the `hurad` binary itself |
| `ansi.rs` | one tokenizer for captured screens, into style types of its own, shared by everything that shows them and the matcher that reads them |
| `pane.rs` | the markup the text panes share, so styling stays in one place |
| `attach.rs` | *(hurad)* raw mode, and handing this terminal to the agent |
| `lib.rs` | *(hura-client)* the servers this machine is paired with, pairing with one, and one request against one. `pair` is shared: `hurad connect` and the desktop application's servers screen are both it |
| `trackers.rs` | *(hura-client)* the trackers this machine reads tickets from, and their tokens, kept owner-only beside the paired servers and never sent to one |
| `pin.rs` | *(hura-client)* judging a server by its certificate's fingerprint and nothing else |
| `http.rs` | *(hura-client)* enough HTTP/1.1 to ask an `hurad` a question |
| `state.rs` | where secrets live: keys, tokens, and saved connections |
| `lib.rs` | *(hura-proto)* every message on the wire, carrying the core's own types |
| `stream.rs` | *(hura-proto)* the multiplexed websocket: the channels, and the frames both ends speak |
| `ws.rs` | *(hura-client)* the streaming half, and the pty a terminal channel needs on this side of it |
| `rpc.rs` | *(hurad)* one request in, one outcome out; every arm a call into `ops` |
| `serve.rs` | *(hurad)* the routes, the token check, and keeping blocking work off the runtime |
| `stream.rs` | *(hurad)* the channels a client subscribes to, the pty behind a terminal, and the connection to a chat session's host through a forward |
| `App.tsx` | *(desktop)* the workspace: the project tree, the tabs and the dock |
| `charSize.ts` | *(desktop)* the font metrics WebKit gets wrong, and the probe that corrects them |
| `panes/` | *(desktop)* the terminal, a conversation, a file in Monaco, and a file's diff with the review on it |
| `chat/` | *(desktop)* a transcript folded into things to draw, tool calls as cards, and what the agent is waiting on you for |
| `hura-agent.mjs` | *(the image)* the process that runs the Agent SDK inside a chat session's sandbox: conversations, their transcripts, permission requests, and the status and usage files. `images/hura-base/` |

## Three rules worth knowing before you change anything

**Nothing in `hura-core` may depend on a renderer.** It is the invariant the crate
split exists to hold, and it is load-bearing rather than tidy: a core that knows
about a renderer cannot be linked into a server. It is also easy to breach by
accident, because reaching for a colour type in a module that builds a pane body
is the obvious thing to do -- `ansi.rs` tokenizes into a `Style` of its own for
exactly that reason, and raw-mode attaching lives in `attach.rs` in the binary
rather than in the core.

The rule used to be tested by a ratatui TUI and a headless server both linking
the core. The TUI went in v0.4.0, so what enforces it now is the webview: a
renderer that cannot link Rust at all, and reaches the core only through
`hura-proto`. That is a stricter test than the one it replaced.


**Nothing that paints does I/O.** Every runtime call is a subprocess round trip
costing hundreds of milliseconds, so none of them happen on a path that draws.
The window reaches them over `/rpc`, and `hurad` runs each on `spawn_blocking`. A
feature that needs to ask the runtime something adds an op, not a call in a
handler that renders.

**The sandbox is the source of truth.** Seeding writes
`/sandbox/.hura/meta.json`, so a session describes itself and survives losing the
local cache. The runtime has no labels, so a sandbox's name is its identity:
`hura-<session>`, which says both that it is hura's and which session it is.

A sandbox stops when the runtime's daemon does, and any exec starts it again. So
nothing that only *looks* at a session execs into a stopped one: a refresh reads
it as idle, and it is started by being opened, when the status poll brings its
agent back if it had one. See `seed::resume_check`.

The local cache is disposable: each session's record lives inside its own
sandbox, so deleting `~/.config/hura/sessions.json` and running `hurad ls`
re-adopts everything still running. Every write to it is a locked
read-modify-write, because more than one writer is the normal case -- a window
reconciling the list every second while `hurad new` in another terminal walks a
session from `seeding` to `ready`, which on a large repository takes minutes.

**Seeding runs inside the sandbox, detached from the command that asks for it.**
`hurad new` writes a script into the sandbox, starts it with `setsid`, and then
only *watches* it: the clone, the work branch, the metadata record and the agent
all happen in there, and they finish whether or not the tool that started them is
still running. Killing `hurad new` with `SIGKILL` two seconds into a clone leaves a
session that comes up complete on its own.

The seeder reports each step into `/sandbox/.hura/seed.state` and its output to
`seed.log`, which is what makes the difference between the three things that used
to look identical from outside: still cloning, finished while nobody was looking,
and stopped. The first refresh of any `hurad` command reads that file for a session
whose record still says `creating` or `seeding` and catches the record up --
`seed-kill: seeding -> ready (seeding finished)` -- or marks it failed with the
reason git gave. A seeder still running is left alone, however long it takes.

## Tests

The suite is hermetic on purpose, so a contributor with no sandbox runtime can
still change almost anything:

* pane classification runs against real captures in `crates/hura-core/tests/panes/`,
  included at compile time by `status.rs`;

What that cannot cover is the runtime's contract, which lives in ignored tests
in `crates/sbx-client/tests/live.rs` and needs a signed-in `sbx` and KVM. They
create and remove real sandboxes named `hura-test-*`, one at a time:

```sh
cargo test -p sbx-client -- --ignored --test-threads=1
```

[CONTRIBUTING.md](../CONTRIBUTING.md) has the rest of the development loop.

---

[← Documentation](README.md) · [README](../README.md)
