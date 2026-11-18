# Worktree sessions

A session is a sandbox: its own Linux VM, under a network policy, with the
credentials swapped in by the runtime's proxy. Some work cannot happen there.
An iOS app is built with Xcode, on a Mac, and the sandbox is Linux.

So a server can also make **worktree sessions**: a plain `git worktree` of a
project's checkout, on the server itself, with the agent running there. They
are off unless the config file turns them on:

```toml
worktrees = true
# worktree_root = "~/.local/share/hura/worktrees"
```

Then the create form, on the desktop and on the phone, has a choice of where
the session runs, and `hurad new --worktree --repo ~/dev/thing` makes one from
the command line.

## What you give up

A worktree session runs **with the server's own rights**, and the apps say so
wherever one appears: a shield with a slash through it on the desktop, an
orange one on the phone.

* **No policy.** The agent reaches whatever the server's network reaches.
  There are no events and no policy pane, because nothing is deciding anything.
* **The server user's credentials.** It pushes with their git credentials,
  signs in to Claude as they have, and reads their files. There is no
  credential to choose.
* **The server user's tools.** No toolchains: what is installed on the server
  is what it has. Skills and MCP servers are not installed for it; the ones in
  the server user's own `~/.claude` apply, as they do to any `claude` they run.
* **No previews.** Its ports are the server's own, so the ports pane is not
  shown. A dev server it starts is on the server's loopback.

Turn it on for a machine that is yours and whose user is the one you would
run Claude Code as anyway.

## How it works

* The worktree is added to the **project's checkout** on the server, so a
  worktree session needs a project (or `--repo` naming a checkout). It shares
  that checkout's object store: no clone, and its branch is visible there.
* The working copies go under `worktree_root`, one directory each; the records
  under `$XDG_STATE_HOME/hura/worktrees/<name>`, never inside the worktree.
* The agent runs in the **server user's tmux**, in a session named
  `hura-<name>`; its shells are `hura-<name>-shell-N`.
* A **terminal** agent is the server's `claude`, started with `--settings`
  pointing at hooks that report its state, so the list still knows when it is
  waiting on you. The first time it opens a new worktree, Claude Code asks
  whether to trust the folder; answer it in the terminal.
* A **chat** agent is the same host the sandbox image carries, written out by
  `hurad`, running on the server's `node`. The Agent SDK it needs is installed
  under `$XDG_STATE_HOME/hura/agent-host` the first time, paired with the
  server's Claude Code. Each chat host listens on a port of its own, from
  47800 up.
* **Removing** one stops its tmux sessions and removes the worktree. Its
  branch is deleted only when there is nothing on it; a branch carrying
  commits is left in the checkout.

`hurad doctor` checks for `git`, `tmux`, `claude`, `node` and `npm` when
worktrees are on.

## On Windows

A server [on Windows](install.md#running-the-server-on-windows) makes worktree
sessions too, for the work that has to happen on Windows. The scripts are the
same ones, run by Git Bash, and **psmux**, a tmux for Windows, holds the
agents and shells:

| | |
| --- | --- |
| Git for Windows | its `bash.exe` runs every script, and every pane is a login Git Bash. Claude Code on Windows needs it anyway |
| [psmux](https://github.com/psmux/psmux) | `winget install psmux`, or `cargo install psmux`. hura runs it as `psmux` |
| jq | `winget install jqlang.jq`. The hooks read the agent's state with it, and Git for Windows does not carry it |
| Claude Code, Node.js and npm | as everywhere: the agent, and the chat host |

What is different there:

* **Attach by hand** with `psmux attach -t hura-<name>`, from a terminal.
* **Scripts longer than about 8 KiB go to Git Bash as a file** in the
  temporary directory, because it cuts a longer argument off.
* **A pane is Git Bash, not PowerShell**, which is psmux's default, because
  hura types POSIX shell into it. Your own psmux sessions are left as they are.
* **A restart of the machine brings agents back** as on Linux, although
  Windows keeps the temporary directory the marker is in: its name carries
  when the machine started.

`hurad doctor` checks for Git Bash, `git`, `psmux`, `claude`, `node`, `npm`
and `jq` there.
