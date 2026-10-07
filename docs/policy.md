# Policy

The isolation is the point, so it is visible rather than buried in a YAML file.
The **policy** pane shows the rules the gateway is actually enforcing, per
binary, and the **events** pane is the allow/deny feed behind them:

```
┌ events (UTC) - add-tests ───────────────────────────────────────────────────┐
│  11:15:02  allow  GET github.com:443/octocat/Hello-World.git/info/refs [git]│
│▌ 11:15:02  DENY   /usr/bin/curl(93) -> pastebin.com:443                     │
│▌           endpoint pastebin.com:443 is not allowed by any policy           │
└─────────────────────────────────────────────────────────────────────────────┘
```

The events feed is **kept on disk**, one file per session under
`~/.config/hura/events/`, because the gateway's log is a rolling window and hura is
what makes it roll: every exec it takes to read a sandbox writes three lines of
its own, so at these poll intervals a 1500-line window covers about two minutes
and held *one* event worth showing. Each fetch is merged into what the session has
already shown, deduplicated and trimmed to the last few thousand, so the feed is a
record rather than a peephole -- and closing the tool no longer looks like it wiped
the log. Destroying a session takes its history with it.

`hurad policy <name> --widen` opens egress to the package registries and
`--tighten` closes it back, without restarting the agent -- for the task that
turns out to need a dependency installed.

Only the network section. The filesystem and process sections are fixed when
the sandbox is created, and the gateway will accept a change to them, report it
as effective, and never enforce it -- so nothing here offers one, and the policy
view labels those sections rather than pretending they are live.

The preset offers npm and PyPI and not crates.io or nuget, which is not an oversight:
it grants them to `/usr/bin/node` and `/usr/local/bin/uv`, and those are in every
sandbox because the base image has them. A rule for cargo in a sandbox with no
cargo in it would be decoration -- the same argument `net-open.yaml` makes. A
toolchain is what brings both halves: `hurad new --toolchain rust` runs the session
on an image carrying cargo *and* opens crates.io for it, so a rule like

```
network - allow_index_crates_io_443
  binaries    /usr/local/rust/bin/cargo
  endpoint    index.crates.io:443  rest  enforce  read-only
```

in the pane came from `--toolchain`, granted at create to that binary and to
nothing else in the sandbox. See [toolchains.md](toolchains.md).

## Acting on a denial

`--widen` and `--tighten` are one preset, all or nothing. The events feed is
where the *specific* answer lives -- `hurad events <name>` names the endpoint and
the binary of each denial, and the global lists are what turn one into a
standing rule.

**From the desktop**, right-click an endpoint in the events pane: *allow in this
session*, *allow in every new session too*, or *block…*. Each is the live
`policy update` below, followed -- only once it has landed -- by a write to the
global list when every new session is asked for, and each answers with the
policy re-read. See [desktop.md](desktop.md#the-traffic-pane).

A session-level change goes through the same live `policy update` that
`--widen` uses. Recording the endpoint in a global list applies it to every
`hurad new` from then on:

```sh
hurad endpoints                                     # what is on them
hurad endpoints --allow crates.io:443 --binary /usr/bin/cargo
hurad endpoints --block pastebin.com:443
```

That writes the same file under the same lock, and applies to sandboxes started
from then on rather than to one already running.

An allow binds the endpoint to **the binary the event named**, not to the
sandbox: allowing `github.com:443` off a denied `curl` grants it to curl and
leaves git's own rule alone. An event decided by an L7 rule --
`GET httpbin.org:443/ip`, which names a method and a path and no binary -- has
no binary of its own to bind to, so the desktop offers the binaries the rules
already sending there name. The panel starts on the path that was refused
(see [only some paths](#only-some-paths)), and the whole host is still one
choice away: full access for those binaries lifts the path restriction, since
access and rules together grant the union. With no rule naming the endpoint
there is nothing to offer, and no allow is issued: an endpoint rule with no
binaries grants nothing.

**A block is a removal, not a veto.** OpenShell denies by default and has no
deny-that-outranks-an-allow at L4, so blocking `pastebin.com` is a no-op -- it was
never reachable -- and blocking `platform.claude.com` is real, because
`feature-work.yaml` grants it. The pane says which, per entry:

```
── global lists - applied to every new session
  allow       pastebin.com:443  NOT in this policy
              /usr/bin/curl
  block       platform.claude.com:443  STILL in this policy
  block       nowhere.example.com:443  gone from this policy
```

The third column is the point: a list entry describes what a *new* session gets,
and the session in front of you may predate it or have moved since. The lists
live in `~/.config/hura/endpoints.json`, are written under a lock like the session
cache, and are applied to a fresh sandbox in one `policy update` before the clone
starts -- so nothing has run in it yet. A block that fails to apply **fails the
create**; an allow that fails is a warning. The two are not symmetric: a missing
allow announces itself the moment the agent tries, and a missing block never
mentions itself again.

There is no key for taking an entry off a list -- `A` and `B` move an endpoint
between them, and removing it outright means editing the file, which is plain
JSON and hand-editable.

## Only some paths

An allow does not have to be the whole host. *Only these paths* in the panel
under a denial takes a method and a path glob per row
(`GET /contoso/tools/_packaging/feed/nuget/v3/**`), and the binaries get those
and nothing else on the host: the rule carries method and path rules and no
access class, which is default-deny. `*` matches one path segment and `**` any
number.

A denial of the host names no path, because the connection was refused before
any request was made. So the first paths are typed, or pasted as a URL. After
that the host is inspected, and anything outside the paths is refused with its
request in the feed (`GET pkgs.example.com:443/...`), so the next path can be
read off the denial. *Allow this path…* opens the panel on it.

The gateway adds paths with `policy update --add-allow`, which picks the rule to
add them to by host and port alone, and `--binary` only rides on a new
endpoint. That leaves two shapes it can take and one it cannot:

- **no rule names the endpoint**: a new rule, for the binaries ticked;
- **one rule names it and already grants those binaries**: the paths join it,
  for every binary it grants. This is the case for a path the rule itself
  refused;
- **anything else** is refused with the rule named. Paths for `dotnet` on
  `dev.azure.com`, which `azure_git` grants to git and curl, would need a
  second rule for one endpoint, and the gateway will not guess which of the two
  to widen. That one can only be the whole host, or a policy file of your own.

With *every new session too*, the entry goes on the allow list with its paths.
A second path for the same binaries is added to the entry, since a feed needs
its index, its metadata and its packages before a restore works. At create the
paths are planned against the policy the template gave, the same way. The
command line writes the same entries:

```sh
hurad endpoints --allow pkgs.example.com:443 --binary /usr/share/dotnet/dotnet \
    --path "GET /contoso/tools/_packaging/feed/nuget/v3/**"
```

---

[← Documentation](README.md) · [README](../README.md)
