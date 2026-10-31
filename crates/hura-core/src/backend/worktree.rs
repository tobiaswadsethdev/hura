//! A session that is a `git worktree` on the server, with no sandbox around it.
//!
//! **It runs with the server's own rights, and every part of this file is
//! shaped by saying so.** The agent is an ordinary process under the server's
//! account: it reads the server user's files, pushes with their git
//! credentials, signs in to Claude as they have, and reaches whatever the
//! network lets it reach. There is no policy to show and no decisions to feed,
//! so the policy is empty and changing it is refused with a sentence.
//!
//! Why have it at all: the sandbox is a Linux VM, and some work is not Linux
//! work. An iOS app is built with Xcode, on a Mac, and a Mac running `hurad` is
//! exactly where that can happen. It is off unless the config file says
//! `worktrees = true`, so a server that never asked for it never offers it.
//!
//! ## The record does not live in the worktree
//!
//! A sandbox is the source of truth about itself: `meta.json` sits inside it.
//! Writing `.hura/` into a working copy would put it in every `git status` the
//! agent runs and one `git clean -fdx` from being deleted, so the record lives
//! beside the server's other state, at `$XDG_STATE_HOME/hura/worktrees/<name>/`,
//! and adoption after a lost cache is that directory reconciled against the
//! worktrees that still exist.
//!
//! ## tmux is the server's
//!
//! Every sandbox has its own tmux server and can call its agent's session
//! `agent`. Here they share one, so the names have to be the session's: the
//! agent is `hura-<name>` and its shells are `hura-<name>-shell-N`. Without
//! that, two worktree sessions would attach to each other's agent, and a
//! person's own tmux sessions would show up as shells.
//!
//! ## What the image would have carried
//!
//! The sandbox image brings the chat host, the hooks that report a terminal
//! agent's state, and the status line that records its spend. Here they are
//! written out of this binary into one directory under the state directory --
//! the same files, which read where to write from `HURA_DIR` -- and the chat
//! host's Agent SDK is installed beside them the first time a chat session
//! needs it. Claude Code itself, `node` and `tmux` are the server's own.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use super::{Backend, Error, ExecOutput, Paths, Result, Torn};
use crate::config::Config;
use crate::doctor::Check;
use crate::events::Event;
use crate::ops::{Draft, ProviderChoice};
use crate::policy;
use crate::projects;
use crate::removed;
use crate::seed::sh_quote;
use crate::session::{Kind, Session, State};
use crate::store;

/// The files the sandbox image carries for an agent, built into this binary.
const HURA_AGENT: &str = include_str!("../../../../images/hura-base/hura-agent.mjs");
const HURA_STATUS: &str = include_str!("../../../../images/hura-base/hura-status");
const HURA_USAGE: &str = include_str!("../../../../images/hura-base/hura-usage");

/// The ports a worktree session's chat host may be given. Clear of
/// [`crate::chat::PORT`] and of the relay ports, and kept per record so two
/// sessions never share one.
const AGENT_PORTS: std::ops::RangeInclusive<u16> = 47_800..=47_999;

/// What every refusal to touch a policy says.
const NOT_ISOLATED: &str = "this session is a git worktree on the server, running with the \
     server's own rights: it has no network policy to change";

pub struct Worktree {
    /// Where the working copies go. One directory per session.
    root: PathBuf,
    /// Where the records go. Deliberately not under [`Self::root`]; see the
    /// module comment.
    state: PathBuf,
    /// Where the chat host, the hooks and the SDK go.
    support: PathBuf,
}

impl Worktree {
    pub fn new(root: PathBuf) -> Self {
        let state = crate::state::dir();
        Worktree::at(root, state.join("worktrees"), state.join("agent-host"))
    }

    pub fn at(root: PathBuf, state: PathBuf, support: PathBuf) -> Self {
        Worktree {
            root,
            state,
            support,
        }
    }

    pub fn from_config(cfg: &Config) -> Self {
        Worktree::new(
            cfg.worktree_root
                .clone()
                .unwrap_or_else(Worktree::default_root),
        )
    }

    /// `$XDG_DATA_HOME/hura/worktrees`, or `~/.local/share/hura/worktrees`.
    ///
    /// Data rather than state: a worktree is work, with commits in it that may
    /// not have been pushed anywhere. State is what a machine can regenerate,
    /// and this is the one directory here that it cannot.
    pub fn default_root() -> PathBuf {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".local").join("share")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("hura")
            .join("worktrees")
    }

    /// The directory this session's working copy is in, whether or not it
    /// exists yet. The record's `workdir` wins, so a session made under a
    /// root that has since been reconfigured keeps working.
    fn dir(&self, session: &Session) -> PathBuf {
        match &session.workdir {
            Some(p) => PathBuf::from(p),
            None => self.root.join(&session.name),
        }
    }

    fn record_dir(&self, name: &str) -> PathBuf {
        self.state.join(name)
    }

    /// The checkout a worktree is added to.
    ///
    /// A project is the ordinary answer: it stores the path of the checkout it
    /// was named from. The fallback is a `repo` that is itself a path on this
    /// machine, which is what `hurad new --worktree --repo ~/dev/thing` means.
    /// A clone URL alone cannot answer it -- the point of a worktree is to
    /// share an object store that already exists, and nothing here will clone
    /// one to make that true.
    fn source_checkout(&self, session: &Session) -> Result<PathBuf> {
        if let Some(name) = &session.project
            && let Some(p) = projects::list().into_iter().find(|p| &p.name == name)
        {
            return Ok(PathBuf::from(p.path));
        }
        let direct = PathBuf::from(&session.repo);
        if direct.join(".git").exists() {
            return Ok(direct);
        }
        Err(Error::Local(format!(
            "a worktree session needs a checkout on the server to add to. \
             `{}` is not one, and {}. Start it in a project, or give a path \
             on this machine.",
            session.repo,
            match &session.project {
                Some(p) => format!("project `{p}` is not there either"),
                None => "no project was named".to_string(),
            }
        )))
    }

    /// Write the chat host, the hooks and their settings where the sessions
    /// find them. Once per process: they are this binary's, so an update to
    /// it is an update to them the next time it starts.
    fn install_support(&self) -> Result<()> {
        static DONE: OnceLock<std::result::Result<(), String>> = OnceLock::new();
        DONE.get_or_init(|| {
            let d = &self.support;
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            for (name, text, exec) in [
                ("hura-agent.mjs", HURA_AGENT, false),
                ("hura-status", HURA_STATUS, true),
                ("hura-usage", HURA_USAGE, true),
                ("settings.json", &settings(d), false),
            ] {
                let path = d.join(name);
                std::fs::write(&path, text)
                    .map_err(|e| format!("could not write {}: {e}", path.display()))?;
                #[cfg(unix)]
                if exec {
                    use std::os::unix::fs::PermissionsExt as _;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
                }
                #[cfg(not(unix))]
                let _ = exec;
            }
            Ok(())
        })
        .clone()
        .map_err(Error::Local)
    }

    /// Run something on the server, and report it the way an exec reports.
    ///
    /// A command that ran and returned non-zero is an [`ExecOutput`] with that
    /// code, exactly as a sandbox exec would be. A command that could not be
    /// started is an error, because nothing ran and there is nothing to quote.
    ///
    /// Each in a process group of its own, so the seeder it may leave running
    /// is not stopped by a Ctrl-C meant for a `hurad serve` in a terminal --
    /// which is the job `setsid` does in a sandbox, and macOS has no `setsid`.
    fn run(&self, argv: &[&str], input: Option<&[u8]>) -> Result<ExecOutput> {
        let (bin, args) = argv
            .split_first()
            .ok_or_else(|| Error::Local("nothing to run".into()))?;
        let mut cmd = Command::new(bin);
        // An explicit directory rather than whatever the server was started
        // in, which may have been deleted under a long-lived process. Every
        // script says where it wants to be anyway.
        cmd.current_dir("/")
            .args(args)
            .env("PATH", search_path())
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            cmd.process_group(0);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Local(format!("could not run `{bin}`: {e}")))?;
        if let Some(input) = input {
            use std::io::Write as _;
            // Written from a thread, so a command that prints before it has
            // read everything cannot deadlock against this write.
            let mut stdin = child.stdin.take().expect("stdin was piped");
            let input = input.to_vec();
            std::thread::spawn(move || {
                let _ = stdin.write_all(&input);
            });
        }
        let out = child
            .wait_with_output()
            .map_err(|e| Error::Local(format!("`{bin}` did not finish: {e}")))?;
        Ok(ExecOutput {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            exit_code: out.status.code().unwrap_or(-1),
        })
    }

    /// Stop the agent and every shell beside it, before the worktree goes:
    /// they are running in it, and a tmux session whose directory has been
    /// deleted is a pane that cannot be used and will not go away on its own.
    fn kill_tmux(&self, session: &Session) {
        let script = format!(
            "for s in $({tmux} list-sessions -F '#{{session_name}}' 2>/dev/null); do \
               case \"$s\" in {agent}|{prefix}*) {tmux} kill-session -t \"$s\" 2>/dev/null || true;; esac; \
             done",
            tmux = self.tmux(),
            agent = sh_quote(&session.tmux),
            prefix = sh_quote(&self.shell_prefix(session)),
        );
        let _ = self.run(&["sh", "-c", &script], None);
    }

    /// A port for a chat host that no other worktree session has and nothing
    /// is listening on now.
    fn free_agent_port(&self) -> Result<u16> {
        let taken: Vec<u16> = store::Store::load()
            .map(|s| s.list().iter().filter_map(|s| s.agent_port).collect())
            .unwrap_or_default();
        AGENT_PORTS
            .clone()
            .filter(|p| !taken.contains(p))
            .find(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
            .ok_or_else(|| {
                Error::Local(format!(
                    "every port from {} to {} is in use; remove a worktree session",
                    AGENT_PORTS.start(),
                    AGENT_PORTS.end()
                ))
            })
    }

    /// The step that installs the Agent SDK for the chat host, when it is not
    /// there yet: the release paired with the server's Claude Code, as the
    /// image picks it, so the two speak the same control protocol.
    fn sdk_script(&self) -> String {
        format!(
            r#"if [ ! -d {dir}/node_modules/@anthropic-ai/claude-agent-sdk ]; then
  cd {dir}
  v=$({claude} --version 2>/dev/null | awk '{{print $1}}')
  sdk=$(npm view '@anthropic-ai/claude-agent-sdk@>=0.0.0' claudeCodeVersion 2>/dev/null \
    | awk -v want="'$v'" '$2 == want {{ sub(/.*@/, "", $1); print $1 }}' | tail -n 1)
  npm install --no-save --no-audit --no-fund --omit=optional --legacy-peer-deps \
    "@anthropic-ai/claude-agent-sdk@${{sdk:-latest}}" >&2
fi
"#,
            dir = sh_quote(&self.support.display().to_string()),
            claude = sh_quote(&tool("claude")),
        )
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The server's `PATH`, plus where the tools a worktree agent needs are put
/// when a service manager started `hurad` with less than a login shell has.
/// Inherited by the tmux server when an exec here is what starts it, and so
/// by every agent in it.
fn search_path() -> String {
    let mut dirs: Vec<String> = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .collect();
    let mut extra = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ];
    if let Some(h) = home() {
        extra.push(h.join(".local/bin"));
        extra.push(h.join(".local/share/mise/shims"));
        extra.push(h.join(".cargo/bin"));
    }
    for d in extra {
        let d = d.display().to_string();
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    dirs.join(":")
}

/// A tool on [`search_path`], as an absolute path, or its bare name when it
/// is not there and the failure is better left to the shell to report.
fn tool(name: &str) -> String {
    search_path()
        .split(':')
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
        .map_or_else(|| name.to_string(), |p| p.display().to_string())
}

/// The settings a terminal agent here is started with, on top of the server
/// user's own: the hooks and status line the image's settings carry, pointed
/// at this directory's copies. Nothing else of the image's -- the model, the
/// permission mode -- is the server user's to choose.
fn settings(support: &Path) -> String {
    let status = |state: &str| {
        serde_json::json!({ "hooks": [{ "type": "command",
            "command": format!("{} {state}", sh_quote(&support.join("hura-status").display().to_string())) }] })
    };
    let any = |state: &str| {
        let mut v = status(state);
        v["matcher"] = "*".into();
        v
    };
    let value = serde_json::json!({
        "statusLine": {
            "type": "command",
            "command": sh_quote(&support.join("hura-usage").display().to_string()),
        },
        "hooks": {
            "SessionStart": [status("idle")],
            "UserPromptSubmit": [status("running")],
            "PreToolUse": [any("running")],
            "PostToolUse": [any("running")],
            "Notification": [status("waiting")],
            "Stop": [status("idle")],
            "SessionEnd": [status("idle")],
        }
    });
    serde_json::to_string_pretty(&value).expect("plain JSON") + "\n"
}

/// The tmux session name for a worktree session's agent, `hura-` prefixed so
/// it is recognisable in a `tmux ls` beside the server user's own.
pub fn tmux_name(name: &str) -> String {
    format!("hura-{name}")
}

impl Backend for Worktree {
    fn kind(&self) -> Kind {
        Kind::Worktree
    }

    fn paths(&self, session: &Session) -> Paths {
        let hura = self.record_dir(&session.name);
        Paths {
            repo: self.dir(session).display().to_string(),
            // In the temporary directory, which a restart of the machine
            // empties: what the sandbox's tmpfs marker is for.
            boot: std::env::temp_dir()
                .join(format!("hura-agent-up-{}", session.name))
                .display()
                .to_string(),
            hura: hura.display().to_string(),
        }
    }

    /// The record's agent, told where its state goes and given the hooks
    /// that write it.
    fn agent_command(&self, session: &Session) -> String {
        format!(
            "HURA_DIR={dir} {agent} --settings {settings}",
            dir = sh_quote(&self.record_dir(&session.name).display().to_string()),
            agent = session.agent,
            settings = sh_quote(&self.support.join("settings.json").display().to_string()),
        )
    }

    fn hura_agent(&self, session: &Session) -> String {
        format!(
            "HURA_DIR={dir} HURA_AGENT_PORT={port} HURA_CLAUDE={claude} {node} \
             --disable-warning=UNDICI-EHPA {script}",
            dir = sh_quote(&self.record_dir(&session.name).display().to_string()),
            port = crate::chat::port(session),
            claude = sh_quote(&tool("claude")),
            node = sh_quote(&tool("node")),
            script = sh_quote(&self.support.join("hura-agent.mjs").display().to_string()),
        )
    }

    fn exec(&self, _session: &Session, argv: &[&str]) -> Result<ExecOutput> {
        self.install_support()?;
        self.run(argv, None)
    }

    fn exec_stdin(&self, _session: &Session, argv: &[&str], input: &[u8]) -> Result<ExecOutput> {
        self.install_support()?;
        self.run(argv, Some(input))
    }

    /// Nothing to wrap it in: what a terminal attaches to here is the command
    /// itself, under a pty on this machine.
    fn interactive_argv(&self, _session: &Session, argv: &[&str]) -> Result<Vec<String>> {
        let mut out = vec!["env".to_string(), format!("PATH={}", search_path())];
        out.extend(argv.iter().map(|a| (*a).to_string()));
        Ok(out)
    }

    /// The port is already on this machine's loopback, so the forward has
    /// nothing to carry: it says where to connect, as a relay does, and waits
    /// to be stopped.
    fn forward_argv(
        &self,
        _session: &Session,
        port: u16,
        host: crate::ports::Loopback,
    ) -> Result<Vec<String>> {
        if host != crate::ports::Loopback::V4 {
            return Err(Error::Local(format!(
                "port {port} is only on [::1] here; bind it to 127.0.0.1 to preview it"
            )));
        }
        Ok(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo \"Forwarding 127.0.0.1:$1 -> 127.0.0.1:$1 on this machine\"; exec tail -f /dev/null"
                .to_string(),
            "sh".to_string(),
            port.to_string(),
        ])
    }

    /// No `-f`: this is the server user's own tmux, and their config is
    /// theirs. The locale is still said outright, because a service manager's
    /// environment is not a login shell's and tmux with no UTF-8 locale draws
    /// an agent's glyphs as `_`.
    fn tmux(&self) -> &'static str {
        "LANG=C.UTF-8 LC_ALL=C.UTF-8 COLORTERM=truecolor tmux -u"
    }

    fn shell_prefix(&self, session: &Session) -> String {
        format!("{}-shell-", session.tmux)
    }

    fn place(&self, session: &mut Session, _draft: &Draft) -> Result<()> {
        // Named before anything else, because `agent` -- what a sandboxed
        // session calls its tmux session -- would be one name shared by every
        // worktree on the machine.
        session.tmux = tmux_name(&session.name);
        // None of these is anything here, and a record saying otherwise would
        // be claiming a guarantee that is not there: no policy, no proxy to
        // swap a credential in, no image to carry a toolchain. Skills and MCP
        // servers would be installed into the server user's own `~/.claude`,
        // for every agent they run, so they are left to that file.
        session.policy = None;
        session.providers.clear();
        session.toolchains.clear();
        session.skills.clear();
        session.mcp.clear();

        let checkout = self.source_checkout(session)?;
        if !checkout.join(".git").exists() {
            return Err(Error::Local(format!(
                "`{}` is not a git checkout",
                checkout.display()
            )));
        }
        let dir = self.root.join(&session.name);
        if dir.exists() {
            return Err(Error::Local(format!(
                "`{}` already exists; remove it or pick another name",
                dir.display()
            )));
        }
        std::fs::create_dir_all(&self.root).map_err(Error::local)?;
        std::fs::create_dir_all(self.record_dir(&session.name)).map_err(Error::local)?;
        self.install_support()?;
        session.workdir = Some(dir.display().to_string());
        if session.interface == crate::chat::Interface::Chat {
            session.agent_port = Some(self.free_agent_port()?);
        }

        // The branch the checkout is on, when nothing else said. A clone
        // records the remote's default and the diff resolves it later; a
        // checkout may have no remote at all, so the base is decided here, or
        // there is nothing for the diff to measure against.
        if session.base_branch.is_none() {
            let out = self.run(
                &[
                    "git",
                    "-C",
                    &checkout.display().to_string(),
                    "rev-parse",
                    "--abbrev-ref",
                    "HEAD",
                ],
                None,
            )?;
            let branch = out.trimmed().trim().to_string();
            if out.ok() && !branch.is_empty() && branch != "HEAD" && branch != session.work_branch {
                session.base_branch = Some(branch);
            }
        }
        Ok(())
    }

    /// Nothing to impose: the endpoint lists, the MCP grants and the toolchain
    /// registries are all rules for a proxy, and there is no proxy here.
    fn configure(&self, _s: &Session, _d: &Draft, _w: &mut Vec<String>) -> Result<()> {
        Ok(())
    }

    /// Add the worktree, on the branch this session works on.
    ///
    /// Idempotent in both directions, because re-seeding is how a half-created
    /// session is repaired: an existing worktree is reused, and an existing
    /// branch is checked out rather than re-created.
    ///
    /// The fetch is best-effort and deliberate: without it the base is
    /// whatever the checkout last pulled. It touches the person's own checkout,
    /// which is why it is a fetch and never anything that moves a ref they
    /// have.
    fn fetch_script(&self, session: &Session) -> String {
        let checkout = match self.source_checkout(session) {
            Ok(p) => p.display().to_string(),
            // The seeder has nowhere to return an error to, so the failure is
            // the script's, with the reason, for its handler to report.
            Err(e) => {
                return format!("printf '%s\\n' {} >&2\nexit 1\n", sh_quote(&e.to_string()));
            }
        };
        let base = match &session.base_branch {
            Some(b) => b.clone(),
            None => String::new(),
        };
        let sdk = if session.interface == crate::chat::Interface::Chat {
            self.sdk_script()
        } else {
            String::new()
        };
        format!(
            r#"if [ -e {dir}/.git ]; then
  cd {dir}
  git checkout {branch} 2>/dev/null || git checkout -b {branch}
else
  cd {checkout}
  git fetch --prune origin >/dev/null 2>&1 || true
  base={base}
  if [ -n "$base" ] && git rev-parse --verify --quiet "origin/$base" >/dev/null 2>&1; then
    base="origin/$base"
  fi
  if [ -z "$base" ]; then
    # `|| true`: the seeder runs under `set -e`, and a checkout with no remote
    # has no default branch to name. The fallback below is HEAD.
    base=$(git symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null || true)
  fi
  if [ -n "$base" ]; then
    git rev-parse --verify --quiet "$base" >/dev/null 2>&1 || base=''
  fi
  if [ -z "$base" ]; then base=HEAD; fi
  if git show-ref --verify --quiet refs/heads/{branch}; then
    git worktree add {dir} {branch}
  else
    git worktree add -b {branch} {dir} "$base"
  fi
fi
{sdk}"#,
            dir = sh_quote(&self.dir(session).display().to_string()),
            checkout = sh_quote(&checkout),
            branch = sh_quote(&session.work_branch),
            base = sh_quote(&base),
        )
    }

    /// Take the worktree away, and the agent with it.
    ///
    /// `--force`, because a worktree with modifications is what every session
    /// that did any work is. **The branch goes too, but only when there is
    /// nothing on it**: `git branch -d` deletes one merged into its upstream or
    /// its base and refuses one carrying work of its own, which may be the only
    /// copy, and is left where it is.
    fn tear_down(&self, name: &str, session: Option<&Session>) -> Result<Torn> {
        if let Some(session) = session {
            self.kill_tmux(session);
        }

        let dir = session.map_or_else(|| self.root.join(name), |s| self.dir(s));
        let record = self.record_dir(name);
        let existed = dir.exists();
        if existed {
            // Asked of the repository the worktree belongs to, which git
            // resolves from the worktree itself: `worktree remove .` from
            // inside it is refused.
            let branch = match session {
                Some(s) => format!(
                    "git -C \"$repo\" branch -d {} >/dev/null 2>&1 || true",
                    sh_quote(&s.work_branch)
                ),
                None => String::new(),
            };
            let script = format!(
                "main=$(git -C {dir} rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 1
repo=\"$main/..\"
git -C \"$repo\" worktree remove --force {dir} || rm -rf {dir}
git -C \"$repo\" worktree prune >/dev/null 2>&1 || true
{branch}",
                dir = sh_quote(&dir.display().to_string()),
            );
            let out = self.run(&["sh", "-c", &script], None)?;
            if !out.ok() {
                // No longer a worktree of anything -- the checkout was deleted,
                // or someone pruned it -- and still this session's to remove.
                std::fs::remove_dir_all(&dir).map_err(Error::local)?;
            }
        }
        let _ = std::fs::remove_dir_all(&record);
        if let Some(s) = session {
            let _ = std::fs::remove_file(self.paths(s).boot);
        }
        Ok(if existed {
            Torn::Removed
        } else {
            Torn::RecordOnly
        })
    }

    /// Alive is "the directory is still there". Weaker than a runtime's state
    /// and honestly so: the ways a worktree ends are someone deleting it, a
    /// `git worktree remove` elsewhere, or a disk that is not mounted, and all
    /// three mean the session is gone.
    fn live(&self, cached: Vec<Session>) -> Result<store::Reconciliation> {
        let mut out = store::Reconciliation::default();
        for mut session in cached {
            if self.dir(&session).exists() {
                if session.state == State::Dead {
                    session.state = State::Ready;
                }
            } else if !matches!(
                session.state,
                State::Creating | State::Seeding | State::Failed
            ) {
                if session.state != State::Dead {
                    out.dead.push(session.name.clone());
                }
                session.state = State::Dead;
            }
            out.sessions.push(session);
        }

        let known: Vec<&str> = out.sessions.iter().map(|s| s.name.as_str()).collect();
        let tombstoned = removed::names();
        // The records are what is scanned, not the worktrees: a directory
        // under the root says nothing about who made it.
        if let Ok(entries) = std::fs::read_dir(&self.state) {
            for entry in entries.flatten() {
                let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                if !entry.path().join("meta.json").is_file() {
                    continue;
                }
                if tombstoned.contains(&name) {
                    out.lingering.push(name);
                } else if !known.contains(&name.as_str()) {
                    out.orphans.push(name);
                }
            }
        }
        for name in &tombstoned {
            if !out.lingering.contains(name) && self.root.join(name).exists() {
                out.lingering.push(name.clone());
            }
        }
        Ok(out)
    }

    fn read_meta(&self, name: &str) -> Result<Session> {
        let path = self.record_dir(name).join("meta.json");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| Error::Local(format!("could not read {}: {e}", path.display())))?;
        serde_json::from_str(&text)
            .map_err(|e| Error::Local(format!("{} is not a session record: {e}", path.display())))
    }

    fn rules(&self, _session: &Session) -> Result<Vec<policy::Rule>> {
        Ok(Vec::new())
    }

    fn add_rules(&self, _session: &Session, _rules: &[sbx_client::RuleSpec]) -> Result<()> {
        Err(Error::Local(NOT_ISOLATED.to_string()))
    }

    fn remove_rule(&self, _session: &Session, _id: &str) -> Result<()> {
        Err(Error::Local(NOT_ISOLATED.to_string()))
    }

    fn withdraw(&self, _session: &Session, _endpoint: &str, _allow: bool) -> Result<()> {
        Err(Error::Local(NOT_ISOLATED.to_string()))
    }

    /// Nothing decides anything about this session's requests, so there is
    /// nothing to report.
    fn events(&self, _session: &Session) -> Result<Vec<Event>> {
        Ok(Vec::new())
    }

    /// None: a credential is something the runtime's proxy swaps in, and the
    /// server user's own are what a worktree session uses.
    fn providers(&self) -> Result<Vec<ProviderChoice>> {
        Ok(Vec::new())
    }

    fn health(&self) -> Check {
        let missing: Vec<&str> = ["git", "tmux", "claude", "node", "npm"]
            .into_iter()
            .filter(|t| tool(t) == *t)
            .collect();
        if missing.is_empty() {
            Check::ok("worktrees", format!("on, under {}", self.root.display()))
        } else {
            Check::fail(
                "worktrees",
                format!("on, but {} not found", missing.join(", ")),
                "install them on this machine; a worktree session's agent runs here",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str, dir: &Path) -> Session {
        let mut s = Session::new(name.to_string(), "/nowhere".into(), String::new());
        s.backend = Kind::Worktree;
        s.tmux = tmux_name(name);
        s.workdir = Some(dir.display().to_string());
        s
    }

    fn backend(tmp: &Path) -> Worktree {
        Worktree::at(
            tmp.join("worktrees"),
            tmp.join("state"),
            tmp.join("support"),
        )
    }

    fn scratch(tag: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("hura-wt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        tmp
    }

    /// One tmux server holds every worktree session on the machine, so a
    /// shared name would be two sessions attaching to one agent.
    #[test]
    fn shells_are_named_per_session() {
        let tmp = std::env::temp_dir();
        let b = backend(&tmp);
        let one = session("alpha", &tmp);
        let two = session("beta", &tmp);
        assert_eq!(b.shell_prefix(&one), "hura-alpha-shell-");
        assert_eq!(b.shell_prefix(&two), "hura-beta-shell-");
        assert_ne!(one.tmux, two.tmux);
    }

    /// The record would show up in every `git status` the agent runs, and go
    /// with the first `git clean`.
    #[test]
    fn the_record_lives_outside_the_worktree() {
        let tmp = std::env::temp_dir().join("hura-test-record");
        let b = backend(&tmp);
        let s = session("alpha", &tmp.join("worktrees").join("alpha"));
        let paths = b.paths(&s);
        assert!(!paths.meta().starts_with(&paths.repo), "{paths:?}");
        assert!(paths.meta().ends_with("state/alpha/meta.json"), "{paths:?}");
    }

    /// The hooks are what tell the list a terminal agent is waiting on you;
    /// the server user's own settings do not have them, so they are handed
    /// to the agent, with where to write.
    #[test]
    fn a_terminal_agent_is_given_the_hooks_and_its_directory() {
        let tmp = scratch("agent");
        let b = backend(&tmp);
        let s = session("alpha", &tmp);
        let cmd = b.agent_command(&s);
        assert!(cmd.starts_with("HURA_DIR="), "{cmd}");
        assert!(cmd.contains("state/alpha"), "{cmd}");
        assert!(cmd.contains("claude --settings"), "{cmd}");

        let written: serde_json::Value =
            serde_json::from_str(&settings(&tmp.join("support"))).unwrap();
        let hook = written["hooks"]["Notification"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(hook.ends_with("hura-status' waiting"), "{hook}");
    }

    /// Several hosts share this machine's loopback, so each listens where its
    /// record says, and is told so.
    #[test]
    fn a_chat_host_listens_on_its_own_port() {
        let tmp = scratch("host");
        let b = backend(&tmp);
        let mut s = session("alpha", &tmp);
        s.agent_port = Some(47_801);
        let cmd = b.hura_agent(&s);
        assert!(cmd.contains("HURA_AGENT_PORT=47801"), "{cmd}");
        let start = crate::chat::start_script(&b, &s);
        assert!(start.contains("serve --port 47801"), "{start}");
    }

    /// The fallback that makes `--repo ~/dev/thing` work, and the error that
    /// has to be readable when there is nothing to add a worktree to.
    #[test]
    fn a_local_checkout_can_be_named_directly() {
        let tmp = scratch("checkout");
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        let b = backend(&tmp);
        let mut s = session("alpha", &tmp);
        s.repo = tmp.display().to_string();
        assert_eq!(b.source_checkout(&s).unwrap(), tmp);

        s.repo = "https://github.com/example/thing.git".into();
        let err = b.source_checkout(&s).unwrap_err().to_string();
        assert!(err.contains("needs a checkout on the server"), "{err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Re-seeding is how a half-created session is repaired, so neither the
    /// worktree nor the branch may be a second create's problem.
    #[test]
    fn the_fetch_script_is_idempotent() {
        let tmp = scratch("fetch");
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        let b = backend(&tmp);
        let mut s = session("alpha", &tmp.join("wt"));
        s.repo = tmp.display().to_string();

        let script = b.fetch_script(&s);
        assert!(script.contains("git checkout"), "{script}");
        assert!(script.contains("worktree add -b"), "{script}");
        assert!(
            script.contains("show-ref --verify --quiet refs/heads/"),
            "{script}"
        );
        assert!(
            !script.contains("claude-agent-sdk"),
            "a terminal session needs no SDK"
        );

        s.interface = crate::chat::Interface::Chat;
        assert!(b.fetch_script(&s).contains("claude-agent-sdk"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A checkout with one commit on `main`, and a worktree session added to
    /// it by the seeder's own script. Real git, because what is tested is what
    /// git decides.
    fn checkout_with_a_session(tag: &str) -> (PathBuf, Worktree, Session) {
        let tmp = scratch(tag);
        let repo = tmp.join("checkout");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .expect("git");
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        git(&["init", "--quiet", "--initial-branch=main"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("README"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "--quiet", "-m", "first"]);

        let b = backend(&tmp);
        let dir = tmp.join("worktrees").join("alpha");
        let mut s = session("alpha", &dir);
        s.repo = repo.display().to_string();
        s.work_branch = "hura/alpha".into();
        s.base_branch = Some("main".into());
        let out = b
            .run(
                &["sh", "-c", &format!("set -eu\n{}", b.fetch_script(&s))],
                None,
            )
            .unwrap();
        assert!(out.ok(), "{out:?}");
        std::fs::create_dir_all(b.record_dir("alpha")).unwrap();
        (tmp, b, s)
    }

    fn branches(repo: &Path) -> String {
        let out = Command::new("git")
            .current_dir(repo)
            .args(["branch", "--format=%(refname:short)"])
            .output()
            .expect("git");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn the_seeder_adds_a_worktree_on_the_work_branch() {
        let (tmp, b, s) = checkout_with_a_session("add");
        assert!(b.dir(&s).join("README").is_file());
        assert!(branches(&tmp.join("checkout")).contains("hura/alpha"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A branch with nothing on it is part of the session, or a session made
    /// again under the name would resume the old one.
    #[test]
    fn removing_a_session_that_did_no_work_takes_its_branch_with_it() {
        let (tmp, b, s) = checkout_with_a_session("rm-empty");
        let repo = tmp.join("checkout");
        assert_eq!(b.tear_down("alpha", Some(&s)).unwrap(), Torn::Removed);
        assert!(!b.dir(&s).exists());
        assert!(!b.record_dir("alpha").exists());
        assert!(
            !branches(&repo).contains("hura/alpha"),
            "{}",
            branches(&repo)
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Commits that were never pushed are the one thing removing a session
    /// must not destroy.
    #[test]
    fn a_branch_carrying_commits_is_left_where_it_is() {
        let (tmp, b, s) = checkout_with_a_session("rm-work");
        let repo = tmp.join("checkout");
        let dir = b.dir(&s);
        std::fs::write(dir.join("README"), "two\n").unwrap();
        for args in [vec!["add", "."], vec!["commit", "--quiet", "-m", "work"]] {
            let out = Command::new("git")
                .current_dir(&dir)
                .args(&args)
                .output()
                .expect("git");
            assert!(out.status.success(), "{out:?}");
        }
        assert_eq!(b.tear_down("alpha", Some(&s)).unwrap(), Torn::Removed);
        assert!(!dir.exists());
        assert!(
            branches(&repo).contains("hura/alpha"),
            "{}",
            branches(&repo)
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_worktree_is_removable_with_no_record_at_all() {
        let (tmp, b, s) = checkout_with_a_session("rm-no-record");
        assert_eq!(b.tear_down("alpha", None).unwrap(), Torn::Removed);
        assert!(!b.dir(&s).exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A worktree whose directory went is dead; one still being made is not,
    /// since its directory is the seeder's to add.
    #[test]
    fn a_missing_directory_is_a_dead_session_unless_it_is_being_made() {
        let tmp = scratch("live");
        let b = backend(&tmp);
        let mut gone = session("gone", &tmp.join("nope"));
        gone.state = State::Ready;
        let mut young = session("young", &tmp.join("nope2"));
        young.state = State::Creating;
        let rec = b.live(vec![gone, young]).unwrap();
        assert_eq!(rec.dead, vec!["gone".to_string()]);
        assert_eq!(rec.sessions[1].state, State::Creating);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn its_policy_cannot_be_changed() {
        let tmp = std::env::temp_dir();
        let b = backend(&tmp);
        let s = session("alpha", &tmp);
        assert!(b.rules(&s).unwrap().is_empty());
        assert!(b.events(&s).unwrap().is_empty());
        let err = b.remove_rule(&s, "x").unwrap_err().to_string();
        assert!(err.contains("no network policy"), "{err}");
    }
}
