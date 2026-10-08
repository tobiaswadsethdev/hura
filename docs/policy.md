# Policy

The isolation is the point, so it is visible rather than buried in a file. The
**policy** pane shows the rules the sandbox runtime is actually enforcing for a
session, and the **events** pane is the allow/deny feed behind them:

```
┌ events (UTC) - add-tests ───────────────────────────────────────────────────┐
│  11:15:02  allow  HTTP:GET  GET github.com:443/octocat/Hello-World.git/...  │
│▌ 11:15:02  DENY   NET:OPEN  pastebin.com:443                            x3  │
│▌           No matching allow rule (default deny)                            │
└─────────────────────────────────────────────────────────────────────────────┘
```

## What a rule is

Every session runs in its own microVM, and everything that leaves it goes
through the runtime's proxy on this machine. A rule is about a destination:

* **hosts**: a host, a glob or an IP, with an optional port. A bare host is any
  port and not its subdomains; `*.example.com` is one label, `**.example.com`
  any number, and `**` is everything.
* **any request**, or only some **methods on a path**: `POST /**/git-receive-pack`.
  A rule like that makes the proxy inspect every request to the host, and
  anything on it the rules do not name is refused.
* **allow** or **deny**. A deny outranks every allow, so blocking an endpoint
  really closes it, whatever opened it.

Rules are for the whole sandbox, **not for one program in it**. Under OpenShell a
rule named the binaries it let out (`git` to github.com, `claude` to the model
API, nothing else); the sandbox runtime has no way to say that, and the agent has
sudo inside its VM, so it could not be enforced anyway. What keeps a session
narrow is which hosts it may reach and, for the ones that matter, which requests.

The runtime itself refuses what no rule allows, which is its `deny-all` preset.
`hurad doctor` checks that, and fails when the runtime's own global policy would
let every session reach more than its rules say.

## Templates

A session starts from a template: a list of rules in TOML, in `policies/`.

| | |
| --- | --- |
| `readonly-explore` | clone from GitHub or Azure DevOps; no push, no model API |
| `feature-work` | the model API, clone and push to both forges, Azure DevOps' REST API for pull requests, and api.github.com |
| `net-open` | `feature-work`, plus npm and PyPI for reading |

```toml
# git over smart HTTP, read and write
[[rule]]
hosts = ["github.com:443"]
methods = ["POST"]
path = "/**/git-receive-pack"
```

`--policy` takes a template name or a path to a file of your own in that shape.
An OpenShell YAML policy is refused by name: its binaries and filesystem
sections have nothing to apply to.

A host with method rules is only reachable through the proxy, which every tool in
the image uses by its `HTTPS_PROXY` variable. A client that ignores the variable
is still proxied, transparently, for a host opened to any request, but refused
for one narrowed to paths, since a transparent connection cannot be inspected.
That includes containers the session starts with its own Docker Engine, which
are not given the variable: pass `-e HTTPS_PROXY` to one that needs a path-ruled
host.

## Widening and tightening

`hurad policy <name> --widen` opens the package registries for reading (npm,
PyPI and Docker Hub, for a session pulling images) and `--tighten` closes them
again, without restarting the agent: for the task that turns out to need a
dependency installed. Read-only means GET, HEAD and OPTIONS on any path.

A toolchain brings its own registry: `hurad new --toolchain rust` runs the
session on an image carrying cargo and opens crates.io for reading. See
[toolchains.md](toolchains.md).

## Acting on a denial

The events feed is where the specific answer lives: `hurad events <name>` names
the endpoint of each denial and, for a request the proxy judged by its path, the
method and path it refused. The runtime counts rather than lists, so one line
stands for every time the same thing happened since its daemon started, `x3`.

Three denials are in every session's feed from its first seconds, and they are
not the agent: while making a sandbox the runtime runs an `apt-get update`,
which reaches for `archive.ubuntu.com:80`, `security.ubuntu.com:80` and
`download.docker.com:443` and is refused like anything else. Nothing in the
session needs them; `sudo apt-get install` does, and `--widen` does not open
them.

**From the desktop**, right-click an endpoint in the events pane: *allow in this
session*, *allow in every new session too*, *allow only some paths…*, or
*block…*. Each changes the session's rules at once and answers with the policy
re-read. See [desktop.md](desktop.md#the-traffic-pane).

* An **allow** opens the whole host, or only the methods and paths you name. A
  deny the session has for the endpoint goes first, since it would otherwise
  outrank the allow.
* A **block** is a deny. The session's own allows of the endpoint go with it,
  because the runtime refuses a deny that names exactly what an allow names.
* Any rule can be **removed** from the policy pane, which is how a change made
  here is taken back.

**Every new session too** writes the global lists, `~/.config/hura/endpoints.json`,
which are applied to a sandbox at create, before anything runs in it. A block
on the list is applied after taking the template's allows of that endpoint away,
and a block that fails to apply **fails the create**; an allow that fails is a
warning. The two are not symmetric: a missing allow announces itself the moment
the agent tries, and a missing block never mentions itself again.

```sh
hurad endpoints                                     # what is on them
hurad endpoints --allow crates.io:443
hurad endpoints --allow pkgs.example.com:443 --path "GET /contoso/_packaging/feed/nuget/v3/**"
hurad endpoints --block pastebin.com:443
```

The policy pane says, for each entry, whether the session in front of you has
it, since a list entry describes what a *new* session gets and this one may
predate it:

```
── global lists - applied to every new session
  allow       docs.rs:443  in this session
  allow       crates.io:443  NOT in this session
  block       pastebin.com:443  in this session
```

There is no key for taking an entry off a list: removing it means *remove from
the list* in the policy pane, or editing the file, which is plain JSON.

## Only some paths

An allow does not have to be the whole host. *Only these paths* under a denial
takes a method and a path glob per row (`GET /contoso/tools/_packaging/feed/nuget/v3/**`),
and the session gets those and nothing else on the host. `*` matches one path
segment and `**` any number. A path starts with `/` and has no query, fragment,
percent-encoding or `..`.

A denial of the host names no path, because the connection was refused before
any request was made. So the first paths are typed, or pasted as a URL. After
that the host is inspected, and anything outside the paths is refused with its
request in the feed (`GET pkgs.example.com:443/...`), so the next path can be
read off the denial. *Allow this path…* opens the panel on it.

---

[← Documentation](README.md) · [README](../README.md)
