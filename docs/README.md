# Documentation

| | |
| --- | --- |
| [install.md](install.md) | prerequisites, Docker Sandboxes, credentials, `hurad` itself, and the desktop application on Linux and Windows |
| [desktop.md](desktop.md) | the desktop workspace: projects and worktrees, files, git, the editor, the review |
| [configuration.md](configuration.md) | `~/.config/hura/config.toml`, and which default wins |
| [policy.md](policy.md) | the rules being enforced, the audit feed, and acting on a denial |
| [tickets.md](tickets.md) | your tickets and pull requests, reviewing a pull request, notifications, and sessions from either |
| [git-hosts.md](git-hosts.md) | GitHub and Azure DevOps, and how pushing keeps the token off your host |
| [worktrees.md](worktrees.md) | sessions that are a plain git worktree on the server, for work a Linux VM cannot do |
| [toolchains.md](toolchains.md) | node, .NET and Rust in a sandbox, and the registry each one may reach |
| [skills.md](skills.md) | carrying your own skills into a sandbox, from either machine |
| [mcp.md](mcp.md) | MCP servers the server runs or you do, their secrets, and what an MCP server costs you |
| [server.md](server.md) | running `hurad`, pairing a client on another machine, and what a token is worth |
| [notifications.md](notifications.md) | being told on a phone when a session waits on you, and answering it from there |
| [relay-api.md](relay-api.md) | the push relay between `hurad` and the iOS app: its API, and how notification text is encrypted |
| [sandbox-image.md](sandbox-image.md) | what the image bakes in, and why the agent runs in auto mode |
| [architecture.md](architecture.md) | the crates, the modules, and three rules worth knowing before changing them |
| [sbx-daemon.service](sbx-daemon.service) | the systemd user unit for the sandbox runtime's daemon |

Contributing is covered in [CONTRIBUTING.md](../CONTRIBUTING.md); the entry
point for everything is the [README](../README.md).
