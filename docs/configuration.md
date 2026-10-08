# Configuration

Every default `hurad` takes is a flag, and `hurad config --init` writes a file that
stops them being typed. `~/.config/hura/config.toml`, beside the session cache,
all keys optional:

```toml
repo       = "https://github.com/octocat/Hello-World" # `hurad new` with no --repo
base       = "develop"                                # unset: the remote's default
policy     = "feature-work"                           # a template, or a path to a TOML file
providers  = ["claude", "azure-devops"]               # credentials for a new session, by name
repo_roots = ["~/dev", "~/work"]                      # where the picker looks
branch_prefix = "tobias"                              # <prefix>/<name> for a work branch
refresh    = "1s"                                     # unused since v0.4.0; still parsed
auto_update = true                                    # download new releases ahead of a restart
interface  = "chat"                                   # a new session's agent: "chat" or "terminal"

sbx            = "/opt/sbx/bin/sbx"                   # unset: PATH, then ~/.docker/sbx/bin/sbx
sandbox_cpus   = 4                                    # unset: every host CPU
sandbox_memory = "8g"                                 # unset: half the host's memory

skills     = ["ship-pr"]                               # copied into every session

[credentials.claude]                                  # one table per credential
kind    = "claude-code-oauth"                         # see below
command = "tr -d '\r\n' < /home/me/.config/hura/tokens/claude"

[credentials.azure-devops]
kind = "azure-devops-pat"
ref  = "op://Work/ado-pat-encoded/credential"         # or a reference, not a command

[[mcp]]                                               # one table per MCP server
name = "jira"                                         # see docs/mcp.md
url  = "http://host.docker.internal:9001/mcp"         # ... a server you run

[[mcp]]
name    = "sentry"                                    # ... or one hurad runs
image   = "ghcr.io/example/mcp-sentry:1.4"
port    = 9000
secrets = ["SENTRY_TOKEN"]                            # names; values live on the server
```

Trackers are not in this file. They are set up in the desktop application and
kept on the computer it runs on, tokens included -- see
[tickets.md](tickets.md). A `[[tracker]]` table left from an older version is
accepted and ignored.

Everything in it is a *default*: a flag on the command line wins, and so does an
explicit choice in the create form. `hurad config` prints what is in force with
`*` for the file's answers and `-` for the built-in ones.

## Editing it from the window

**settings** in the desktop application's header writes the five keys that are
about what a new session starts with: `branch_prefix`, `base`, `policy`,
`providers` and `auto_update`. The server's file, not the client's: a work
branch is named the same way whether the session was started from the window or
from `hurad new`, and a window keeping its own prefix would be a second
convention that disagrees with the first. See [desktop.md](desktop.md#settings).

The rest of the file is not editable from there, and the omissions are the
point. `repo_roots`, `skills` and `sbx` are paths on the server;
`[credentials.*]` and `[[mcp]]` are tables, each one a decision about what an
agent of yours can reach, and the integrations screen already says so about the
MCP half.

**The file is edited, not regenerated.** Each key is found and replaced where it
stands, so every comment `hurad config --init` wrote is still there afterwards --
which matters because the comments are most of what the file is for. Clearing a
field removes the key rather than writing an empty one, because an absent key is
what gets the built-in default and `policy = ""` would say something the parser
does not mean. And the new text is parsed *before* it is written: every command
except `hurad doctor` refuses to run against a config it cannot read, so a
settings screen that could save an invalid one would be able to break the server
from inside its own UI.

**A file that cannot be read stops the command**, rather than being quietly
replaced by the defaults -- a key that does nothing is indistinguishable from a
key that is not working, so a misspelled one is named back at you:

```
hura: ~/.config/hura/config.toml: TOML parse error at line 1, column 1
  |
1 | polciy = "feature-work"
  | ^^^^^^
unknown field `polciy`, expected one of `gateway`, `repo`, `base`, `policy`, ...
```

The one exception is `hurad doctor`, which is the command you reach for when
something is wrong: it reports the error as a failed check and carries on with
the defaults. It also checks that every name in `providers` is a
`[credentials.NAME]` table in the same file, since a stale name is the quietest
failure here: the form does not tick it, the sandbox comes up without the
credential, and the clone fails for what looks like an authentication problem
several steps later.

## Credentials

One `[credentials.NAME]` table per credential a session may be given. The name
is yours, and it is what `providers`, `--provider` and the create form's boxes
use. Each table has a `kind`, and exactly one of `command` or `ref`:

| `kind` | what it is | sent to | the sandbox sees |
| --- | --- | --- | --- |
| `claude-code-oauth` | the token `claude setup-token` prints | `api.anthropic.com`, `platform.claude.com` | `CLAUDE_CODE_OAUTH_TOKEN` |
| `azure-devops-pat` | an Azure DevOps personal access token | `dev.azure.com` | `AZURE_DEVOPS_PAT` |
| `github` | a GitHub token | `github.com`, `api.github.com` | `GITHUB_TOKEN` |

`command` is run on this machine, in a shell, and what it prints is the value.
`ref` is a 1Password `op://` reference or an AWS Secrets Manager ARN, which the
runtime resolves on this machine. **hura never reads the value.** Creating a session
with a credential ticked gives that session's sandbox a secret of its own, made
from the same `command` or `ref`, and puts a random placeholder in the variable
in the last column. The runtime's proxy writes the real value over the
placeholder in the headers of requests to the kind's hosts, and nowhere else:
not in a query string or a body, and not on any other host. A token printed
inside the sandbox, or sent to a host the agent was talked into, is the
placeholder.

The runtime runs a `command` from a temporary directory of its own, so give
paths in full; `~` and a relative path are not guaranteed to mean what they do
in your shell. Strip the trailing newline (the `tr -d '\r\n'` above), which
would otherwise end up in a header.

An Azure DevOps PAT is HTTP Basic with an empty user, so what belongs in the
header is `base64(":" + PAT)`. A `command` prints the PAT itself and hura wraps
it so the runtime stores the encoded form. A `ref` cannot be wrapped, so for
Azure DevOps it has to name a secret that already holds the encoded form.

A session gets at most one credential of each kind, because each kind has one
variable for its placeholder; a create naming two is refused. Removing the
session's sandbox removes its secrets. [git-hosts.md](git-hosts.md) is how the
git side uses them.

## Other keys

`policy` is a template name, or a path to a template file of your own in the
same TOML shape as the built-in ones (`hurad policies` lists those). An
OpenShell policy (`.yaml`) is refused with an explanation, because its shape
does not carry over; [policy.md](policy.md) has the rules.

`sbx` is the Docker Sandboxes CLI, for when it is neither on `PATH` nor where
its installer puts it. `sandbox_cpus` and `sandbox_memory` (`"8g"`, `"512m"`)
size each session's sandbox. Unset, the runtime gives every sandbox every host
CPU and half the host's memory, which is generous for one session and too much
for several at once.

`gateway` named the OpenShell gateway sessions ran on. Nothing reads it now; it
is still accepted, and ignored, so an existing file keeps loading.

`refresh` is one number rather than six because the intervals underneath it are
measured and related to each other; it scales all of them, so `"4s"` polls a
quarter as often (41 execs in a 30 second window became 13) and `"500ms"` twice
as often. 250ms to 60s -- below that the terminal interface's 100ms input tick became the
limit and the extra `git status` inside every sandbox buys nothing. Nothing has
read it since that interface went in v0.4.0; the key is still parsed so a file
written before then still loads.

`auto_update` is on unless it is turned off. What it permits is a *download*:
`hurad serve` checks every six hours and leaves a verified binary beside the
running one, and the swap happens at the next start rather than under a live
session -- [install.md](install.md#updating-without-being-asked) is the whole
of it. `false` for a machine that would rather not reach github at all, which
also turns the check off, not just the install.

Where a default meets something hura already works out for itself, the more
specific answer wins:

* `providers` **replaces** the create form's guesswork, because an explicit list
  beats a heuristic and merging the two would attach a credential nobody asked
  for.
* `base` only fills a **detached HEAD**: the branch a checkout is sitting on is
  evidence about that repository, and a config entry is a guess about all of them.
* `repo` moves the picker's **cursor**, not its filter, so every other repository
  is still one keystroke away -- and typing drops the preference for good.
* `repo_roots` **replaces** the conventional places rather than adding to them,
  and `HURA_REPO_ROOTS` still wins over it.

`repo_roots` is about the machine that *runs* the sessions, which with a server
is not the machine with the window on it.

`worktree_root` configured worktree sessions, which have been removed. It is
still accepted, and ignored, so an existing file keeps loading.

Two things are deliberately *not* in this file. **Secret values** are not: a
credential is a command or a reference, and a managed MCP server's secrets are
named here and stored in `$XDG_STATE_HOME/hura/secrets.json`, because a config
file is the kind of thing people copy between machines and paste into an
issue. And **uploaded
skills** are not listed at all: what a client has pushed into the server's
library is a directory listing rather than a decision, and a second list to keep
in step with it would only ever be wrong. See [mcp.md](mcp.md) and
[skills.md](skills.md).


---

[← Documentation](README.md) · [README](../README.md)
