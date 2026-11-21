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
//! Where the server starts is where every agent in it lives, and under a
//! systemd service that was the service's own cgroup: stopping the service,
//! which a restart or an update does, killed the tmux server and every agent
//! with it, and the boot marker then kept them from being brought back. So
//! under a service a `new-session` starts in a scope of its own, which the
//! server stays in after the command has gone. See [`Worktree::tmux_new`].
//!
//! ## What the image would have carried
//!
//! The sandbox image brings the chat host, the hooks that report a terminal
//! agent's state, and the status line that records its spend. Here they are
//! written out of this binary into one directory under the state directory --
//! the same files, which read where to write from `HURA_DIR` -- and the chat
//! host's Agent SDK is installed beside them the first time a chat session
//! needs it. Claude Code itself, `node` and `tmux` are the server's own.
//!
//! ## On Windows
//!
//! The same scripts, run by Git Bash, with psmux as the tmux. Git for Windows
//! is what Claude Code on Windows needs anyway, and psmux is a tmux for
//! Windows that speaks tmux's commands over the console Windows has. Where the
//! two differ from a Unix server it is said here and nowhere else:
//!
//! * `sh` is Git Bash's `bash.exe`, found by [`host::posix_shell`], and every
//!   other program is run through it, since a script's tools (`cat`, `awk`,
//!   `true`) are Git Bash's and not on Windows' own `PATH`.
//! * A script longer than [`ARG_MAX`] goes to Git Bash as a file: measured,
//!   it cuts any one argument off at about 8 KiB.
//! * A pane runs Git Bash ([`Backend::tmux_shell`]), not psmux's default,
//!   PowerShell, which cannot run the lines typed into it.
//! * Paths written into scripts and settings use `/`, which Git Bash and
//!   Windows both read; a hook is run by naming Git Bash, so it works whichever
//!   shell Claude Code hands it to.
//! * The temporary directory survives a restart of the machine, so the boot
//!   marker's name carries when the machine started.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use super::{Backend, Error, ExecOutput, Paths, Result, Torn};
use crate::config::Config;
use crate::doctor::Check;
use crate::events::Event;
use crate::host;
use crate::ops::{Draft, ProviderChoice};
use crate::policy;
use crate::projects;
use crate::removed;
use crate::seed::sh_quote;
use crate::session::{Kind, Session, State};
use crate::store;

/// The files the sandbox image carries for an agent, built into this binary.
const HURA_AGENT: &str = include_str!("../../../../hurad/images/hura-base/hura-agent.mjs");
const HURA_STATUS: &str = include_str!("../../../../hurad/images/hura-base/hura-status");
const HURA_USAGE: &str = include_str!("../../../../hurad/images/hura-base/hura-usage");

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
    /// Whether a `new-session` starts in a scope of its own. Unset is
    /// [`scoped`]'s answer, which is this process's; a test says outright.
    scope: Option<bool>,
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
            scope: None,
        }
    }

    /// Say outright whether a `new-session` starts in a scope of its own,
    /// rather than leaving it to what this process is running under.
    #[cfg(test)]
    pub(crate) fn with_scope(mut self, scope: bool) -> Self {
        self.scope = Some(scope);
        self
    }

    pub fn from_config(cfg: &Config) -> Self {
        Worktree::new(
            cfg.worktree_root
                .clone()
                .unwrap_or_else(Worktree::default_root),
        )
    }

    /// `worktrees` in hura's data directory: `$XDG_DATA_HOME/hura/worktrees`,
    /// `~/.local/share/hura/worktrees`, or under `%LOCALAPPDATA%\hura\data` on
    /// Windows. See [`crate::dirs`].
    ///
    /// Data rather than state: a worktree is work, with commits in it that may
    /// not have been pushed anywhere. State is what a machine can regenerate,
    /// and this is the one directory here that it cannot.
    pub fn default_root() -> PathBuf {
        crate::dirs::data().join("worktrees")
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
        let script = posix_argv(argv, None)?;
        let (program, args): (&std::ffi::OsStr, Vec<String>) = match &script {
            Some(posix) => (posix.program.as_os_str(), posix.args.clone()),
            None => (
                std::ffi::OsStr::new(*bin),
                args.iter().map(|a| (*a).to_string()).collect(),
            ),
        };
        let mut cmd = Command::new(program);
        // An explicit directory rather than whatever the server was started
        // in, which may have been deleted under a long-lived process. Every
        // script says where it wants to be anyway.
        cmd.current_dir(neutral_dir())
            .args(&args)
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
        // The same on Windows, with no console window for it either. And a
        // login Git Bash, which is what a psmux pane runs, stays in the
        // directory it was started in rather than going home.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(host::CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
                .env("CHERE_INVOKING", "1");
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Local(crate::plain::spawn(bin, &e)))?;
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
            dir = sh_quote(&slash(&self.support)),
            claude = sh_quote(&tool("claude")),
        )
    }
}

fn home() -> Option<PathBuf> {
    crate::dirs::home()
}

/// The server's `PATH`, plus where the tools a worktree agent needs are put
/// when a service manager started `hurad` with less than a login shell has.
/// Inherited by the tmux server when an exec here is what starts it, and so
/// by every agent in it.
fn search_path() -> String {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.retain(|d| !d.as_os_str().is_empty());
    // Where tools go that a service manager's `PATH` may leave out. Windows'
    // Task Scheduler gives a task the account's own `PATH`, so there it is
    // only the two user directories Claude Code and cargo install into.
    let mut extra: Vec<PathBuf> = if cfg!(windows) {
        Vec::new()
    } else {
        vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
        ]
    };
    if let Some(h) = home() {
        extra.push(h.join(".local").join("bin"));
        if !cfg!(windows) {
            extra.push(h.join(".local").join("share").join("mise").join("shims"));
        }
        extra.push(h.join(".cargo").join("bin"));
    }
    for d in extra {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    std::env::join_paths(dirs)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A tool on [`search_path`], as an absolute path, or its bare name when it
/// is not there and the failure is better left to the shell to report. On
/// Windows a program is a `.exe` or, installed by npm, a `.cmd`, and the path
/// is written with `/` for the scripts it goes into.
fn tool(name: &str) -> String {
    let names: Vec<String> = if cfg!(windows) {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            name.to_string(),
        ]
    } else {
        vec![name.to_string()]
    };
    std::env::split_paths(&search_path())
        .flat_map(|d| names.iter().map(move |n| d.join(n)).collect::<Vec<_>>())
        .find(|p| p.is_file())
        .map_or_else(|| name.to_string(), |p| slash(&p))
}

/// A path as a script here writes it: see [`host::script_path`].
fn slash(path: &Path) -> String {
    host::script_path(path)
}

/// The longest script given to Git Bash as an argument. Measured: it cuts
/// any one argument off at about 8 KiB, and the script then ends inside a
/// quote. Longer ones go as a file.
const ARG_MAX: usize = 7_000;

/// Where a command here runs when it has not said: `/` on Unix, rather than
/// whatever the server was started in, and the temporary directory on
/// Windows, which has no `/`.
fn neutral_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::temp_dir()
    } else {
        PathBuf::from("/")
    }
}

/// A command for Git Bash: the program, its arguments, and the script file
/// to delete once it has run, if one was written.
struct PosixArgv {
    program: PathBuf,
    args: Vec<String>,
    /// Held for its `Drop`, which deletes the file once the command has run.
    _script: Option<ScriptFile>,
}

/// A script written out for Git Bash to read, deleted with this.
struct ScriptFile(PathBuf);

impl Drop for ScriptFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `argv` as Git Bash runs it on Windows, and `None` elsewhere, where it runs
/// as it is.
///
/// `sh` is Git Bash itself. Anything else is run by it, `exec "$@"`, because
/// the programs scripts name are Git Bash's and are not on Windows' `PATH`.
/// A `-c` script past [`ARG_MAX`] is written to a file: in `keep` when the
/// caller reads it later, where it is named for its contents and left;
/// otherwise in the temporary directory, deleted when the result is dropped.
fn posix_argv(argv: &[&str], keep: Option<&Path>) -> Result<Option<PosixArgv>> {
    if !cfg!(windows) {
        return Ok(None);
    }
    let bash = host::posix_shell().ok_or_else(|| {
        Error::Local(
            "Git for Windows is not installed: a worktree session's scripts run in its bash".into(),
        )
    })?;
    let (first, rest) = argv
        .split_first()
        .ok_or_else(|| Error::Local("nothing to run".into()))?;
    let mut args: Vec<String> = if *first == "sh" {
        rest.iter().map(|a| (*a).to_string()).collect()
    } else {
        ["-c", "exec \"$@\"", "sh"]
            .into_iter()
            .chain(argv.iter().copied())
            .map(str::to_string)
            .collect()
    };
    let mut script = None;
    if args.first().map(String::as_str) == Some("-c")
        && args.get(1).is_some_and(|s| s.len() > ARG_MAX)
    {
        let text = &args[1];
        let path = match keep {
            Some(dir) => {
                use std::hash::{Hash as _, Hasher as _};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                text.hash(&mut h);
                dir.join(format!("hura-script-{:016x}.sh", h.finish()))
            }
            None => {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                std::env::temp_dir().join(format!("hura-script-{}-{n}.sh", std::process::id()))
            }
        };
        std::fs::write(&path, text)
            .map_err(|e| Error::Local(format!("could not write {}: {e}", path.display())))?;
        if keep.is_none() {
            script = Some(ScriptFile(path.clone()));
        }
        // `sh -c SCRIPT NAME ARGS` is `sh FILE ARGS`: the file is `$0`.
        let tail: Vec<String> = args.iter().skip(3).cloned().collect();
        args = std::iter::once(slash(&path)).chain(tail).collect();
    }
    Ok(Some(PosixArgv {
        program: bash,
        args,
        _script: script,
    }))
}

/// Part of the boot marker's name that changes when the machine restarts:
/// nothing on Unix, where the temporary directory is emptied then, and on
/// Windows the minute it started, which it is not.
fn boot_suffix() -> String {
    #[cfg(windows)]
    {
        // SAFETY: no arguments, no state.
        let up = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        format!("-{}", now.saturating_sub(up) / 60_000)
    }
    #[cfg(not(windows))]
    String::new()
}

/// The settings a terminal agent here is started with, on top of the server
/// user's own: the hooks and status line the image's settings carry, pointed
/// at this directory's copies. Nothing else of the image's -- the model, the
/// permission mode -- is the server user's to choose.
fn settings(support: &Path) -> String {
    let status = |state: &str| {
        serde_json::json!({ "hooks": [{ "type": "command",
            "command": format!("{} {state}", hook(&support.join("hura-status"))) }] })
    };
    let any = |state: &str| {
        let mut v = status(state);
        v["matcher"] = "*".into();
        v
    };
    let value = serde_json::json!({
        "statusLine": {
            "type": "command",
            "command": hook(&support.join("hura-usage")),
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

/// The command that runs one of the hook scripts. The script itself on Unix.
/// On Windows Git Bash is named to run it, in double quotes, which read the
/// same in Git Bash and in `cmd`, so it works whichever Claude Code hands it
/// to.
fn hook(script: &Path) -> String {
    match host::posix_shell() {
        Some(bash) if cfg!(windows) => format!("\"{}\" \"{}\"", slash(&bash), slash(script)),
        _ => sh_quote(&script.display().to_string()),
    }
}

/// The tmux session name for a worktree session's agent, `hura-` prefixed so
/// it is recognisable in a `tmux ls` beside the server user's own.
pub fn tmux_name(name: &str) -> String {
    format!("hura-{name}")
}

/// No `-f`: this is the server user's own tmux, and their config is theirs.
/// The locale is still said outright, because a service manager's environment
/// is not a login shell's and tmux with no UTF-8 locale draws an agent's
/// glyphs as `_`.
#[cfg(not(windows))]
const TMUX: &str = "LANG=C.UTF-8 LC_ALL=C.UTF-8 COLORTERM=truecolor tmux -u";

/// psmux, which has no `-u` and needs none: it is UTF-8 throughout. Without
/// its warm server, a spare it starts beside each one: that spare starts in
/// the session's directory and keeps it, so a removed worktree's directory
/// could not be deleted until it went. Measured.
#[cfg(windows)]
const TMUX: &str = "COLORTERM=truecolor PSMUX_NO_WARM=1 psmux";

/// [`TMUX`] in a transient systemd scope of its own. `systemd-run --scope`
/// runs the command itself, in the caller's environment, so a server it
/// starts inherits what the exec gave it, as it would without. `--collect`
/// drops the scope once it is empty, however it ended: at once when the
/// server was already running, and when the server stops otherwise.
const TMUX_IN_A_SCOPE: &str = "LANG=C.UTF-8 LC_ALL=C.UTF-8 COLORTERM=truecolor \
     systemd-run --user --scope --quiet --collect tmux -u";

/// Whether a `new-session` starts in a scope of its own: this process is in a
/// systemd service's cgroup, and the user's systemd makes scopes. Asked once,
/// the first time a session needs it.
///
/// Only under a service, because that is what kills the whole cgroup when it
/// stops. A `hurad serve` in a terminal, or on a machine with no systemd,
/// starts tmux as it always has. Under a service where a scope cannot be had,
/// it says so once and starts tmux as before, which works until the service
/// stops.
fn scoped() -> bool {
    static SCOPED: OnceLock<bool> = OnceLock::new();
    *SCOPED.get_or_init(|| {
        let in_service =
            std::fs::read_to_string("/proc/self/cgroup").is_ok_and(|c| in_a_service(&c));
        if !in_service {
            return false;
        }
        let works = Command::new("systemd-run")
            .args(["--user", "--scope", "--quiet", "--collect", "true"])
            .current_dir("/")
            .env("PATH", search_path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !works {
            eprintln!(
                "hurad: `systemd-run --user --scope` did not work, so worktree sessions' \
                 agents run in this service and stop when it does"
            );
        }
        works
    })
}

/// Whether `/proc/self/cgroup` puts this process in a service's own cgroup:
/// the unified hierarchy's line, or systemd's own on a legacy one.
fn in_a_service(cgroup: &str) -> bool {
    cgroup
        .lines()
        .filter_map(|line| {
            line.strip_prefix("0::")
                .or_else(|| line.split_once(":name=systemd:").map(|(_, path)| path))
        })
        .any(|path| {
            path.rsplit('/')
                .next()
                .is_some_and(|unit| unit.ends_with(".service"))
        })
}

impl Backend for Worktree {
    fn kind(&self) -> Kind {
        Kind::Worktree
    }

    fn paths(&self, session: &Session) -> Paths {
        let hura = self.record_dir(&session.name);
        Paths {
            repo: slash(&self.dir(session)),
            // In the temporary directory, which a restart of the machine
            // empties: what the sandbox's tmpfs marker is for. Windows keeps
            // it, so there the name says which start of the machine it is.
            boot: slash(&std::env::temp_dir().join(format!(
                "hura-agent-up-{}{}",
                session.name,
                boot_suffix()
            ))),
            hura: slash(&hura),
        }
    }

    /// The record's agent, told where its state goes and given the hooks
    /// that write it.
    fn agent_command(&self, session: &Session) -> String {
        format!(
            "HURA_DIR={dir} {agent} --settings {settings}",
            dir = sh_quote(&slash(&self.record_dir(&session.name))),
            agent = session.agent,
            settings = sh_quote(&slash(&self.support.join("settings.json"))),
        )
    }

    fn hura_agent(&self, session: &Session) -> String {
        format!(
            "HURA_DIR={dir} HURA_AGENT_PORT={port} HURA_CLAUDE={claude}{bash} {node} \
             --disable-warning=UNDICI-EHPA {script}",
            dir = sh_quote(&slash(&self.record_dir(&session.name))),
            port = crate::chat::port(session),
            claude = sh_quote(&tool("claude")),
            // The shell a `!` line runs in. Unset on Unix, where `bash` on the
            // `PATH` is it; on Windows that one is WSL's.
            bash = host::posix_shell()
                .filter(|_| cfg!(windows))
                .map(|b| format!(" HURA_BASH={}", sh_quote(&slash(&b))))
                .unwrap_or_default(),
            node = sh_quote(&tool("node")),
            script = sh_quote(&slash(&self.support.join("hura-agent.mjs"))),
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
        // On Windows the pty is given this process's environment, `PATH`
        // included, and Git Bash finds its own tools; a long script is kept in
        // the temporary directory, since the terminal reads it after this
        // returns.
        if let Some(posix) = posix_argv(argv, Some(&std::env::temp_dir()))? {
            let mut out = vec![posix.program.display().to_string()];
            out.extend(posix.args);
            return Ok(out);
        }
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
        // Waiting is `tail` on Unix, where the forward's guard stops it, and
        // reading stdin to its end on Windows, where closing it is the stop.
        let wait = if cfg!(windows) {
            "exec cat > /dev/null"
        } else {
            "exec tail -f /dev/null"
        };
        let argv = [
            "sh".to_string(),
            "-c".to_string(),
            format!("echo \"Forwarding 127.0.0.1:$1 -> 127.0.0.1:$1 on this machine\"; {wait}"),
            "sh".to_string(),
            port.to_string(),
        ];
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        Ok(match posix_argv(&refs, None)? {
            Some(posix) => std::iter::once(posix.program.display().to_string())
                .chain(posix.args)
                .collect(),
            None => argv.to_vec(),
        })
    }

    fn tmux(&self) -> &'static str {
        TMUX
    }

    /// Git Bash in every pane on Windows, as a login shell so it sets up its
    /// own `PATH`, in the directory `-c` gave it (`CHERE_INVOKING`, set by
    /// [`Self::run`]). Nothing elsewhere: tmux's default shell is a POSIX one.
    fn tmux_shell(&self) -> String {
        match host::posix_shell() {
            Some(bash) if cfg!(windows) => format!(" -- {} --login -i", sh_quote(&slash(&bash))),
            _ => String::new(),
        }
    }

    /// psmux refuses `attach -d`, and its `=` prefix is not read by every
    /// command, so on Windows the name is given plain. A shell's name is the
    /// agent's plus a suffix, and an exact match is preferred over a prefix.
    fn tmux_attach(&self, name: &str) -> String {
        if cfg!(windows) {
            format!("attach -t {}", sh_quote(name))
        } else {
            format!("attach -d -t ={}", sh_quote(name))
        }
    }

    /// In a scope of its own under a systemd service, so the server it may
    /// start, and every agent in it, outlives the service. See the module
    /// comment.
    fn tmux_new(&self) -> &'static str {
        if self.scope.unwrap_or_else(scoped) {
            TMUX_IN_A_SCOPE
        } else {
            TMUX
        }
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
            // Windows will not delete a directory that is some process's
            // current one, and the panes just killed take a moment to go: git
            // empties the worktree and leaves the directory. Measured. Only an
            // empty one is removed here, and on Unix it has gone already.
            for _ in 0..30 {
                if !dir.exists() || std::fs::remove_dir(&dir).is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
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
        // On Windows psmux is the tmux, jq is what the hooks need and Git
        // Bash does not carry, and Git Bash itself runs every script.
        let tools: &[&str] = if cfg!(windows) {
            &["git", "psmux", "claude", "node", "npm", "jq"]
        } else {
            &["git", "tmux", "claude", "node", "npm"]
        };
        let mut missing: Vec<&str> = tools.iter().copied().filter(|t| tool(t) == *t).collect();
        if host::posix_shell().is_none() {
            missing.insert(0, "Git Bash");
        }
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
        .with_scope(false)
    }

    fn scratch(tag: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("hura-wt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        tmp
    }

    /// A download runs the same script a sandbox would, through a real shell:
    /// a file by either kind of path comes back whole, and each refusal comes
    /// back as its own sentence rather than as the shell's.
    #[cfg(unix)]
    #[test]
    fn a_file_is_fetched_whole_by_either_path() {
        let tmp = scratch("download");
        let b = backend(&tmp);
        let repo = tmp.join("repo");
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        let raw: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
        std::fs::write(repo.join("docs/shot.png"), &raw).unwrap();
        std::fs::write(tmp.join("report.md"), "# done\n").unwrap();
        let s = session("alpha", &repo);

        let got = crate::files::fetch(&b, &s, "docs/shot.png").unwrap();
        assert_eq!(got.name, "shot.png");
        assert_eq!(got.bytes, raw.len() as u64);
        assert_eq!(crate::files::decode_base64(&got.data).unwrap(), raw);

        let abs = format!("{}/report.md", tmp.display());
        let got = crate::files::fetch(&b, &s, &abs).unwrap();
        assert_eq!(crate::files::decode_base64(&got.data).unwrap(), b"# done\n");

        let err = crate::files::fetch(&b, &s, "docs/gone.png").unwrap_err();
        assert!(err.contains("not there"), "{err}");
        let err = crate::files::fetch(&b, &s, "/dev/null").unwrap_err();
        assert!(err.contains("regular"), "{err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A folder comes back as a zip of it, `.git` left out, that the system's
    /// own `unzip` unpacks to the same files under the folder's name.
    #[cfg(unix)]
    #[test]
    fn a_folder_is_fetched_as_a_zip() {
        let tmp = scratch("download-folder");
        let b = backend(&tmp);
        let shots = tmp.join("repo/shots");
        std::fs::create_dir_all(shots.join("deep/.git")).unwrap();
        let png: Vec<u8> = (0..=255u8).cycle().take(40_000).collect();
        std::fs::write(shots.join("a.png"), &png).unwrap();
        std::fs::write(shots.join("deep/notes with space.md"), "# hej\n").unwrap();
        std::fs::write(shots.join("deep/.git/HEAD"), "ref\n").unwrap();
        std::fs::write(shots.join("zz-empty"), "").unwrap();
        let s = session("alpha", &tmp.join("repo"));

        let got = crate::files::fetch(&b, &s, "shots").unwrap();
        assert_eq!(got.name, "shots.zip");
        let zip = crate::files::decode_base64(&got.data).unwrap();
        assert_eq!(zip.len() as u64, got.bytes);
        let out = tmp.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("x.zip"), &zip).unwrap();
        let unzip = std::process::Command::new("unzip")
            .args(["-q", "x.zip"])
            .current_dir(&out)
            .output();
        if let Ok(unzip) = unzip {
            assert!(
                unzip.status.success(),
                "{}",
                String::from_utf8_lossy(&unzip.stderr)
            );
            assert_eq!(std::fs::read(out.join("shots/a.png")).unwrap(), png);
            assert_eq!(
                std::fs::read(out.join("shots/deep/notes with space.md")).unwrap(),
                b"# hej\n"
            );
            assert_eq!(std::fs::read(out.join("shots/zz-empty")).unwrap(), b"");
            assert!(!out.join("shots/deep/.git").exists());
        }

        std::fs::create_dir_all(tmp.join("repo/nothing")).unwrap();
        let err = crate::files::fetch(&b, &s, "nothing").unwrap_err();
        assert!(err.contains("no files"), "{err}");
        let _ = std::fs::remove_dir_all(&tmp);
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
        if cfg!(windows) {
            // Git Bash named to run it, in quotes `cmd` reads too.
            assert!(hook.starts_with('"'), "{hook}");
            assert!(hook.contains("bash.exe\" \""), "{hook}");
            assert!(hook.ends_with("hura-status\" waiting"), "{hook}");
        } else {
            assert!(hook.ends_with("hura-status' waiting"), "{hook}");
        }
    }

    /// Git Bash cuts an argument off at about 8 KiB, so a script longer than
    /// that has to reach it some other way. Run for real, on every platform:
    /// the same call is a plain `sh -c` on Unix.
    #[test]
    fn a_script_past_git_bash_s_limit_still_runs() {
        let tmp = scratch("long");
        let b = backend(&tmp);
        let long = "a".repeat(9_000);
        let script = format!("x='{long}'; printf '%s %s' \"${{#x}}\" \"$1\"");
        let out = b.run(&["sh", "-c", &script, "sh", "second"], None).unwrap();
        assert!(out.ok(), "{}", out.stderr);
        assert_eq!(out.stdout, "9000 second");
    }

    /// A program a script would name, run as itself on Unix and through Git
    /// Bash on Windows, where `true` is one of its tools and not Windows'.
    #[test]
    fn a_plain_program_runs_where_scripts_do() {
        let tmp = scratch("plain");
        let b = backend(&tmp);
        let out = b.run(&["true"], None).unwrap();
        assert!(out.ok(), "{}", out.stderr);
    }

    #[test]
    #[cfg(windows)]
    fn on_windows_panes_are_git_bash_and_attach_is_plain() {
        let tmp = scratch("panes");
        let b = backend(&tmp);
        let shell = b.tmux_shell();
        assert!(shell.starts_with(" -- '"), "{shell}");
        assert!(shell.ends_with("bash.exe' --login -i"), "{shell}");
        assert_eq!(b.tmux_attach("hura-alpha"), "attach -t 'hura-alpha'");
        assert!(b.tmux().contains("psmux"), "{}", b.tmux());
        let s = session("alpha", &tmp);
        for path in [b.paths(&s).repo, b.paths(&s).hura, b.paths(&s).boot] {
            assert!(!path.contains('\\'), "a path in a script uses `/`: {path}");
        }
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

    /// Under a service, every script that can start the tmux server starts it
    /// in a scope of its own, and nothing else in them goes through systemd:
    /// the status poll runs these often, and a scope per tmux call would be a
    /// line in the journal each time.
    #[test]
    #[cfg(unix)]
    fn under_a_service_every_new_session_starts_in_a_scope() {
        let tmp = scratch("scope");
        let scoped = backend(&tmp).with_scope(true);
        let mut s = session("alpha", &tmp);
        s.interface = crate::chat::Interface::Chat;
        let mut scripts = vec![
            crate::chat::start_script(&scoped, &s),
            crate::seed::resume_check(&scoped, &s),
            crate::ops::attach_script(&scoped, &s, &s.tmux),
        ];
        s.interface = crate::chat::Interface::Terminal;
        scripts.push(crate::seed::start_agent_script(&scoped, &s));
        scripts.push(crate::seed::launch_agent_script(&scoped, &s, true));
        scripts.push(crate::ops::attach_script(&scoped, &s, &s.tmux));
        for script in &scripts {
            let new = script.matches("new-session").count();
            assert!(new > 0, "{script}");
            let in_scope = script
                .matches("systemd-run --user --scope --quiet --collect tmux -u new-session")
                .count();
            assert_eq!(in_scope, new, "{script}");
            assert_eq!(script.matches("systemd-run").count(), new, "{script}");
        }

        let plain = backend(&tmp);
        s.interface = crate::chat::Interface::Chat;
        let script = crate::chat::start_script(&plain, &s);
        assert!(script.contains("tmux -u new-session"), "{script}");
        assert!(!script.contains("systemd-run"), "{script}");
    }

    /// What `/proc/self/cgroup` says under a user service, in a login
    /// session, and on a legacy hierarchy.
    #[test]
    fn a_service_is_told_from_its_cgroup() {
        let service = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/hurad.service\n";
        assert!(in_a_service(service));
        let system = "0::/system.slice/hurad.service\n";
        assert!(in_a_service(system));

        // `user@1000.service` is the user's manager, not this process's unit.
        let terminal = "0::/user.slice/user-1000.slice/session-c3.scope\n";
        assert!(!in_a_service(terminal));
        let app = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-term.scope\n";
        assert!(!in_a_service(app));
        assert!(!in_a_service("0::/\n"));
        assert!(!in_a_service(""));

        let legacy = "12:cpu,cpuacct:/user.slice\n1:name=systemd:/user.slice/user-1000.slice/user@1000.service/hurad.service\n";
        assert!(in_a_service(legacy));
        let legacy_terminal = "12:cpu,cpuacct:/user.slice/user-1000.slice/user@1000.service\n1:name=systemd:/user.slice/user-1000.slice/session-2.scope\n";
        assert!(!in_a_service(legacy_terminal));
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
