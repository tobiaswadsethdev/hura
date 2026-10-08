# MCP servers

The agents can be given MCP servers, and the servers run **on the host, in their
own containers, holding their own credentials**. Nothing about Jira or Azure
DevOps ever lands on a sandbox filesystem; the sandbox is granted one endpoint
per server. A server on this machine is published on `127.0.0.1`, the sandbox
names it `host.docker.internal`, the sandbox runtime's proxy takes that to this
machine's loopback, and the session gets a rule allowing `localhost` at that
port. Measured from inside a session, against a server on `127.0.0.1:9123`:

```
with the rule      POST http://host.docker.internal:9123/mcp   200
without it         POST http://host.docker.internal:9123/mcp   403  Approval required for localhost:9123
```

**The grant is to the whole sandbox.** A sandbox's rules are about where a
request goes, not which program sent it, so anything in the session can reach a
server it is given, not only the agent. The window's policy pane says the same
under every session's rules.

## Two shapes: one you run, one the server runs

Name them in the config file, one table each. A table has either a `url` (a
server somebody else operates) or an `image`, which makes it **managed**:
`hurad` starts the container, restarts it, holds its secrets and can say what it
is doing.

```toml
# Managed: described here, run by hurad.
[[mcp]]
name    = "jira"
image   = "ghcr.io/sooperset/mcp-atlassian:latest"
port    = 9000
args    = ["--transport", "streamable-http", "--port", "9000"]
env     = { JIRA_URL = "https://your-org.atlassian.net", JIRA_USERNAME = "you@example.com" }
secrets = ["JIRA_API_TOKEN"]              # names; the values live on the server

# Yours: a url. Nothing here starts or stops it.
[[mcp]]
name = "azure-devops"
url  = "http://host.docker.internal:9001/mcp"
transport = "http"                        # or "sse"; http is the default
```

**A managed entry has no url, and that is the point.** `hurad` publishes the
container on `127.0.0.1:<port>` and derives its url,
`http://host.docker.internal:<port>/mcp`, from the same port, so the two
mistakes a hand-written url invites (a container name no sandbox can resolve,
and a `localhost` that means the sandbox itself) cannot happen. It is published
on loopback only, so nothing else on the network reaches it. Each managed entry
needs a port of its own, and the config file is refused when two share one.

The keys that belong to a managed entry are refused beside a `url` rather than
ignored, and an entry with both is refused outright -- it would be a url
pointing somewhere other than the container beside it, which nobody notices
until an agent reports a dead tool.

### Secrets

The values live on the server, in `$XDG_STATE_HOME/hura/secrets.json`, 0600 --
beside the pairing tokens and the TLS key, and protected exactly as well. The
config file holds only the *names*.

**A value goes in and never comes back out.** The window can store one and forget
one, and the protocol carries names and whether each is set; there is no request
that returns a value and there will not be one. From the server itself:

```sh
printf %s "$JIRA_API_TOKEN" | hurad secret JIRA_API_TOKEN   # stdin, not an argument
hurad secrets                                               # the names, never the values
hurad secret JIRA_API_TOKEN --forget
```

Stdin rather than an argument on purpose: an argument lands in the shell history
and in `ps` output, and this is a credential a container will hold for months.
The same care applies when `hurad` starts the container -- the value is put in the
child's environment and the argument list carries only the name.

This is not encryption at rest, and calling a file `secrets.json` invites the
assumption that it is. Anyone who can read it can already run commands as the
user whose containers would use it; a key stored beside the thing it encrypts
would be theatre.

### Starting and stopping

`hurad` brings every managed container up when it starts, and before seeding a
session. Anything already running is left alone -- restarting it would drop the
agent connections of every live session using it. The **integrations** screen in
the window has the buttons; a headless server has the same thing:

```sh
hurad mcp                                # the catalog, and what each one is doing
hurad mcp --action start   jira          # bring one up
hurad mcp --action restart jira          # recreate it from the catalog entry
hurad mcp --action stop    jira
```

`restart` recreates the container from the config file rather than restarting the
one that is there, which is what you want after changing a secret, an argument or
the image tag: the container is the *deployment* of a catalog entry, not a thing
with a life of its own.

The states are worth knowing, because two of them look like health and are not:

| | |
| --- | --- |
| `running` | up, and published on `127.0.0.1` at its port, the only sense in which a sandbox can reach it |
| `crashing` | started, exited, started again. `--restart unless-stopped` means Docker reports a container it is restarting as *running*, so this is measured from the restart count and shows the container's last output |
| `detached` | running, but not published on `127.0.0.1` at its port. Fine in `docker ps`, unreachable from every sandbox. A container started before sessions moved to Docker Sandboxes looks like this, and `hurad` recreates it when it starts |
| `stopped` | it exists and is not running, with its last output |
| `absent` | never started, or stopped from here |
| `external` | a `url` entry: whoever runs it started it, and this server has no say |

## Running one yourself

Everything below is the `url` shape: your container, your `docker run`, your
problem when the host reboots. It is still the right answer for a server that
needs more than an image and a port (a shim, a mount, a second process), and
it is what the managed shape was measured against.

The url is what the **sandbox** sees, which is not what your browser sees.
`localhost` and `127.0.0.1` in there are the sandbox itself, and are refused
when the file is read, because they are correct on the host, wrong in the
sandbox, and invisible until an agent is running. A sandbox cannot reach a
container on this machine by its name either: it is a microVM, not a container
on any of this machine's Docker networks. What works:

* **`host.docker.internal` and a port published on `127.0.0.1`**, for anything
  on this machine, in a container or not. The runtime's proxy takes that name
  to this machine's loopback, and the session's rule names `localhost` and the
  port. This is the shape a managed entry gives you without asking.
  `host.openshell.internal`, from before the move to Docker Sandboxes, is still
  recognised as the same thing; change it when convenient.
* **a host name of its own**, for a server on another machine, which the session
  is given a rule for like any other host.

Jira and Confluence, with the credentials staying in the container:

```sh
docker run -d --name mcp-atlassian -p 127.0.0.1:9000:9000 \
  -e JIRA_URL=https://your-org.atlassian.net \
  -e JIRA_USERNAME=you@example.com -e JIRA_API_TOKEN="$JIRA_API_TOKEN" \
  ghcr.io/sooperset/mcp-atlassian:latest --transport streamable-http --port 9000
```

Azure DevOps needs one extra part: `@azure-devops/mcp` speaks stdio only, so it
runs behind an HTTP shim. Its `pat` mode reads `PERSONAL_ACCESS_TOKEN`, and wants
the base64 of `:<pat>` -- it decodes the value and drops everything up to the
first colon, which is Azure DevOps' usual empty-username Basic auth:

```sh
docker run -d --name mcp-azure-devops -p 127.0.0.1:9001:9001 \
  -e PERSONAL_ACCESS_TOKEN="$(printf ':%s' "$AZURE_DEVOPS_PAT" | base64 -w0)" \
  node:22-alpine npx -y supergateway \
    --stdio "npx -y @azure-devops/mcp <org> -a pat" \
    --outputTransport streamableHttp --port 9001 --stateful
```

Both serve `/mcp`, and their urls are `http://host.docker.internal:9000/mcp`
and `http://host.docker.internal:9001/mcp`. The Azure DevOps one was run against
a real session when this page was first written (the agent reported
`azure-devops: ... ✔ Connected`, with Azure DevOps MCP 2.9.0 answering behind
the shim). The Atlassian one was started and answered on `/mcp` with
placeholder credentials; its own flags are documented by that image.

Registration happens **inside the sandbox, before the agent starts**: the
seeder runs `claude mcp add --scope user` as its own `mcp` step, because the
agent reads its servers at startup and registering them afterwards would leave
the first session of every sandbox without tools. The endpoints are opened by
one rule added when the sandbox is created, so it is in force before anything
can use them. A session records the servers it was created with, and the facts pane
lists them by name; changing the file changes the next session, not a running
one.

`hurad doctor` checks each of them, because a server that is not running, or
one running where no sandbox can reach it, produces a session whose agent
reports its tools as **needing authentication**, which sends you looking in
entirely the wrong direction. A managed entry is asked of the same code the
integrations screen uses, so a check that passes here cannot disagree with a
screen that says something is wrong. One of your own named as
`host.docker.internal` is connected to on `127.0.0.1` at its port. And a url
naming one of this machine's containers is said to be unreachable outright:

```
[ warn ] mcp          jira: `mcp-atlassian` is a container here, and a sandbox cannot reach containers by name; publish it on 127.0.0.1:9000 and use `http://host.docker.internal:9000`
         fix: a managed one starts from the window's integrations screen; one of your own is published on 127.0.0.1 and named as `host.docker.internal` in its url
```

**What this costs.** An MCP server is a hole in the sandbox, and worth being
plain about, which is why the sentence below is also in the window, beside the
list, rather than only here where nobody re-reads it: the agent gains
everything the server can do, using the host's credentials, and the proxy can
only see it as `POST /mcp`. Every MCP call is the same request shape, so the
method/path rules that make the git endpoints sharp buy nothing here: a server
that can transition Jira issues means a sandboxed agent can transition Jira
issues. And since the grant is to the whole sandbox, so does anything else
running in the session. That is a fine trade for Jira and
Azure DevOps, whose blast radius is a work item. It is a terrible one for a
filesystem or Docker MCP server on the host, which would be a straight sandbox
escape, and hura cannot tell the difference for you.

The transport is not a problem the way it might look: streaming responses are
not buffered by the inspecting proxy. An SSE stream emitting an event a second
arrived event by event, a second apart, inside the sandbox.


---

[← Documentation](README.md) · [README](../README.md)
