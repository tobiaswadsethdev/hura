//! Where a session's work actually happens.
//!
//! [`crate::ops`], [`crate::git`], [`crate::files`] and [`crate::seed`] all talk
//! to a [`Backend`] rather than to the sandbox runtime directly. The scripts,
//! the diff, the poll, the status scrape, the file tree, the review and the
//! shells, are shared and pure; where they run, where the files are and how tmux is
//! invoked is this trait's business rather than theirs. There is one
//! implementation, [`Sandboxed`]: a session inside a sandbox of its own, with
//! the sandbox's rules on everything that leaves it.
//!
//! The other, [`Worktree`], is a plain `git worktree` on the server with no
//! isolation at all, for the work a Linux VM cannot do: an iOS app wants a Mac
//! and Xcode. It was removed once, because two backends meant two of
//! everything to keep working and the isolation is the product, and came back
//! behind `worktrees = true` in the config file, off by default. What the two
//! differ in is only *where*:
//!
//! | | Sandboxed | Worktree |
//! | --- | --- | --- |
//! | `exec` | `sbx exec` | a child process on the server |
//! | [`Paths::repo`] | `/sandbox/repo` | the worktree's own directory |
//! | [`Paths::hura`] | `/sandbox/.hura` | under the server's state directory |
//! | tmux | its own server, on the image's config | the server user's, names prefixed |
//! | policy, events | the runtime's | none |
//! | credentials | swapped in by the runtime's proxy | the server user's own |

use sbx_client::RuleSpec;

use crate::doctor::Check;
use crate::events::Event;
use crate::ops::ProviderChoice;
use crate::session::{self, Session};

mod sandboxed;
mod worktree;

pub use sandboxed::{Sandboxed, Settings};
pub use worktree::Worktree;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No sandbox by that name where sessions run.
    #[error("{0}")]
    Missing(String),
    /// Where sessions run said no, or could not be reached.
    #[error("{0}")]
    Refused(String),
    /// Something on the server itself: a command that would not spawn, a
    /// directory that is not there.
    #[error("{0}")]
    Local(String),
}

impl Error {
    /// Whether this is "there is nothing there", which several callers treat as
    /// the state they were trying to reach rather than as a failure.
    pub fn is_missing(&self) -> bool {
        matches!(self, Error::Missing(_))
    }

    fn local(e: impl std::fmt::Display) -> Self {
        Error::Local(e.to_string())
    }
}

/// What a command run inside a session left behind.
///
/// The backend's own type rather than whichever client produced it, so the
/// scripts above the trait (the diff, the poll, the seeder) never name what
/// runs them.
#[derive(Debug, Clone, Default)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl ExecOutput {
    pub fn ok(&self) -> bool {
        self.exit_code == 0
    }

    /// Stdout without its trailing newlines, which is what callers almost
    /// always want.
    pub fn trimmed(&self) -> &str {
        self.stdout.trim_end_matches('\n')
    }
}

/// Where a session's things are, from the point of view of its own `exec`.
///
/// Two absolute paths and everything else derived from them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The repository's working copy.
    pub repo: String,
    /// The directory holding everything hura itself writes about the session.
    pub hura: String,
    /// A file that is there for one boot of whatever the agent runs in: its
    /// absence beside an agent that once ran says it was stopped and started
    /// since. See [`crate::seed::resume_check`].
    pub boot: String,
}

impl Paths {
    /// The sandbox's paths, which the image bakes in and this cannot change.
    ///
    /// `hura-status` writes [`session::STATUS_PATH`] from inside the image and
    /// the seeder's own script names the rest, so these are a published
    /// interface rather than a choice. There is a test below that they still
    /// agree with the constants.
    pub fn in_sandbox() -> Self {
        Paths {
            repo: session::REPO_PATH.to_string(),
            hura: SANDBOX_HURA_DIR.to_string(),
            boot: crate::seed::BOOT_MARKER.to_string(),
        }
    }

    pub fn meta(&self) -> String {
        format!("{}/meta.json", self.hura)
    }
    pub fn task(&self) -> String {
        format!("{}/task.txt", self.hura)
    }
    pub fn status(&self) -> String {
        format!("{}/status.json", self.hura)
    }
    pub fn usage(&self) -> String {
        format!("{}/usage.json", self.hura)
    }
    pub fn seed_state(&self) -> String {
        format!("{}/seed.state", self.hura)
    }
    pub fn seed_log(&self) -> String {
        format!("{}/seed.log", self.hura)
    }
    pub fn seed_script(&self) -> String {
        format!("{}/seed.sh", self.hura)
    }
    /// Left by the first start of the session's agent, and kept: a session
    /// that once had an agent gets it back after its sandbox restarts. See
    /// [`crate::seed::resume_check`].
    pub fn agent_started(&self) -> String {
        format!("{}/agent-started", self.hura)
    }
}

/// The `.hura` directory inside a sandbox. Not public: [`Paths::in_sandbox`] is
/// the way to it, so nothing outside grows a second opinion about the layout.
const SANDBOX_HURA_DIR: &str = "/sandbox/.hura";

/// What starting a session left behind, per backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Torn {
    /// The backend removed the thing the session ran in.
    Removed,
    /// There was nothing left to remove; only the record went.
    RecordOnly,
}

/// The place a session runs.
///
/// Deliberately small, and every method on it is a *where* rather than a what.
/// The scripts are shared: one definition of the diff, the poll, the status
/// scrape and the review lives above this and is handed the paths and the tmux
/// invocation it should use. A backend that grew its own copy of the diff
/// script would be a second answer to what a diff is.
pub trait Backend {
    /// Which kind of place this is, for a record and for a badge.
    fn kind(&self) -> session::Kind;

    fn paths(&self, session: &Session) -> Paths;

    /// The command a terminal agent is started with, before its task.
    ///
    /// The record's own `agent` in a sandbox, whose image carries the settings
    /// that report its state. On the server the same hooks have to be handed
    /// to it, since the server user's settings do not have them.
    fn agent_command(&self, session: &Session) -> String {
        session.agent.clone()
    }

    /// How to run `hura-agent`, the chat host, as a shell command.
    fn hura_agent(&self, _session: &Session) -> String {
        "hura-agent".to_string()
    }

    fn exec(&self, session: &Session, argv: &[&str]) -> Result<ExecOutput>;

    /// Run a command with `input` on its stdin.
    ///
    /// For anything that does not belong in an argument: a script carrying a
    /// task and the skills, a review, a message. Linux caps a single argument
    /// at 128 KiB, and none of those has a size of its own choosing.
    fn exec_stdin(&self, session: &Session, argv: &[&str], input: &[u8]) -> Result<ExecOutput>;

    /// The argv a terminal emulator spawns to attach to this session.
    fn interactive_argv(&self, session: &Session, argv: &[&str]) -> Result<Vec<String>>;

    /// The long-running argv that forwards a loopback port on the server to
    /// `port` inside this session. See [`Sandboxed::forward_argv`].
    fn forward_argv(
        &self,
        session: &Session,
        port: u16,
        host: crate::ports::Loopback,
    ) -> Result<Vec<String>>;

    /// How to invoke tmux where this session's agent runs.
    ///
    /// The image ships a config, and the locale is safer said than inherited,
    /// so this carries both. `-u` says "this terminal is UTF-8" outright rather than
    /// inferring it from an environment.
    fn tmux(&self) -> &'static str;

    /// The prefix the shells beside the agent are named with.
    fn shell_prefix(&self, session: &Session) -> String;

    /// Make the thing the session runs in exist: a sandbox with its policy.
    ///
    /// Takes the session `&mut` because placing it decides facts that belong on
    /// the record, such as which policy revision it got.
    fn place(&self, session: &mut Session, draft: &crate::ops::Draft) -> Result<()>;

    /// Everything imposed on a session that already exists: the global endpoint
    /// lists, the MCP grants, the toolchain registries.
    ///
    /// Apart from [`Backend::place`] because the record is written between the
    /// two, and it has to be: between the sandbox existing and its record being
    /// saved it is an orphan that a refresh in another process will try to
    /// adopt. Imposing the policy and the MCP endpoints is a runtime call per
    /// rule, which makes that window far wider than a moment.
    fn configure(
        &self,
        session: &Session,
        draft: &crate::ops::Draft,
        warnings: &mut Vec<String>,
    ) -> Result<()>;

    /// The seeder's first step: put the repository where [`Paths::repo`] says
    /// it is, on the branch the session works on.
    ///
    /// A script rather than an action, because it runs inside the detached
    /// seeder along with everything else -- which is what lets a clone survive
    /// the tool that asked for it going away.
    fn fetch_script(&self, session: &Session) -> String;

    /// Remove what this session ran in. The record is the caller's to drop.
    fn tear_down(&self, name: &str, session: Option<&Session>) -> Result<Torn>;

    /// Reconcile the cache against what the backend can see.
    fn live(&self, cached: Vec<Session>) -> Result<crate::store::Reconciliation>;

    /// Read a session's own record, from wherever this backend keeps it.
    fn read_meta(&self, name: &str) -> Result<Session>;

    /// Every network rule that applies to the session, its own and the ones
    /// every sandbox has.
    fn rules(&self, session: &Session) -> Result<Vec<crate::policy::Rule>>;

    /// Add rules to the session. They apply to the next request.
    fn add_rules(&self, session: &Session, rules: &[RuleSpec]) -> Result<()>;

    /// Remove one of the session's own rules.
    fn remove_rule(&self, session: &Session, id: &str) -> Result<()>;

    /// Take an endpoint out of the session's own allows (or denies, when
    /// `allow` is false) that name it, keeping what else they named.
    fn withdraw(&self, session: &Session, endpoint: &str, allow: bool) -> Result<()>;

    /// The session's recent allow and deny decisions, as read from wherever
    /// this backend keeps them. Not merged with what was kept: that is
    /// [`crate::ops::events`].
    fn events(&self, session: &Session) -> Result<Vec<Event>>;

    /// Credentials a new session may be given.
    fn providers(&self) -> Result<Vec<ProviderChoice>>;

    /// Whether the place sessions run is there and answering, as `hurad doctor`
    /// reports it.
    fn health(&self) -> Check;
}

/// Both backends, and which of them a session belongs to.
///
/// The one thing above the trait that knows there are two. Everything that
/// works on a session is handed the right one by [`Backends::for_session`].
/// The worktree backend is always here, whatever the config file says: what
/// `worktrees = true` gates is making a new one, and a session made before it
/// was turned off still has to be shown and removed.
pub struct Backends {
    sandboxed: Sandboxed,
    worktree: Worktree,
}

impl Backends {
    pub fn new(sandboxed: Sandboxed) -> Self {
        Backends::with_worktree(sandboxed, Worktree::new(Worktree::default_root()))
    }

    pub fn with_worktree(sandboxed: Sandboxed, worktree: Worktree) -> Self {
        Backends {
            sandboxed,
            worktree,
        }
    }

    pub fn from_client(client: Box<dyn sbx_client::Sbx>, settings: Settings) -> Self {
        Backends::new(Sandboxed::new(client, settings))
    }

    /// The backend as the config file describes it: the `sbx` it names, or the
    /// one found on `PATH` or where its installer puts it, and the sizes and
    /// credentials sessions are made with.
    pub fn from_config(cfg: &crate::config::Config) -> Self {
        let mut client = sbx_client::CliClient::new();
        if let Some(bin) = &cfg.sbx {
            client = client.with_bin(bin);
        }
        Backends::with_worktree(
            Sandboxed::new(Box::new(client), Settings::from_config(cfg)),
            Worktree::from_config(cfg),
        )
    }

    pub fn for_session(&self, session: &Session) -> &dyn Backend {
        self.of_kind(session.backend)
    }

    pub fn of_kind(&self, kind: session::Kind) -> &dyn Backend {
        match kind {
            session::Kind::Sandbox => &self.sandboxed,
            session::Kind::Worktree => &self.worktree,
        }
    }

    /// The sandboxed backend, for what is about the runtime rather than about
    /// one session: the credentials it can hand out, and `hurad doctor`.
    pub fn sandboxed(&self) -> &dyn Backend {
        &self.sandboxed
    }

    pub fn each(&self) -> [&dyn Backend; 2] {
        [&self.sandboxed, &self.worktree]
    }
}

/// Backends for the tests that assert on the *shape* of a script.
///
/// Almost every test in this crate about a backend is about the script it
/// produces -- the diff, the poll, the seeder, the publish -- and a script is
/// pure. What was missing was a way to get a [`Backend`] without a sandbox
/// runtime to talk to, which is why this exists and why its `Sbx` panics: a test
/// that reaches the network through one of these is a test that meant to be a
/// live test.
#[cfg(test)]
pub(crate) mod testing {
    use std::path::Path;

    use sbx_client::{
        CreateOpts, CustomSecret, ExecOutput, PolicyLog, Port, Result as SbxResult, Rule, RuleSpec,
        Sandbox, Sbx, SecretSpec, Template, Version,
    };

    use super::{Sandboxed, Settings};

    pub(crate) fn sandboxed() -> Sandboxed {
        Sandboxed::new(Box::new(NoSandboxes), Settings::default())
    }

    struct NoSandboxes;

    /// Every method unreachable, on purpose. See the module comment.
    impl Sbx for NoSandboxes {
        fn version(&self) -> SbxResult<Version> {
            unreachable!("no sandboxes in a script test")
        }
        fn create(&self, _: &CreateOpts) -> SbxResult<()> {
            unreachable!("no sandboxes in a script test")
        }
        fn detach(&self, _: &str) -> SbxResult<()> {
            unreachable!("no sandboxes in a script test")
        }
        fn list(&self) -> SbxResult<Vec<Sandbox>> {
            unreachable!("no sandboxes in a script test")
        }
        fn exec(&self, _: &str, _: &[&str]) -> SbxResult<ExecOutput> {
            unreachable!("no sandboxes in a script test")
        }
        fn exec_stdin(&self, _: &str, _: &[&str], _: &[u8]) -> SbxResult<ExecOutput> {
            unreachable!("no sandboxes in a script test")
        }
        fn interactive_argv(&self, name: &str, argv: &[&str]) -> Vec<String> {
            // The one method that is pure: it builds a command line and talks
            // to nothing, and a test about attaching wants to read it.
            sbx_client::CliClient::new()
                .with_bin("sbx")
                .interactive_argv(name, argv)
        }
        fn remove(&self, _: &str) -> SbxResult<()> {
            unreachable!("no sandboxes in a script test")
        }
        fn rules(&self, _: &str) -> SbxResult<Vec<Rule>> {
            unreachable!("no sandboxes in a script test")
        }
        fn add_rule(&self, _: &str, _: &RuleSpec) -> SbxResult<String> {
            unreachable!("no sandboxes in a script test")
        }
        fn remove_rule(&self, _: &str, _: &str) -> SbxResult<()> {
            unreachable!("no sandboxes in a script test")
        }
        fn log(&self, _: &str) -> SbxResult<PolicyLog> {
            unreachable!("no sandboxes in a script test")
        }
        fn ports(&self, _: &str) -> SbxResult<Vec<Port>> {
            unreachable!("no sandboxes in a script test")
        }
        fn publish(&self, _: &str, _: u16) -> SbxResult<Port> {
            unreachable!("no sandboxes in a script test")
        }
        fn unpublish(&self, _: &str, _: &Port) -> SbxResult<()> {
            unreachable!("no sandboxes in a script test")
        }
        fn secrets(&self) -> SbxResult<Vec<CustomSecret>> {
            unreachable!("no sandboxes in a script test")
        }
        fn add_secret(&self, _: &SecretSpec) -> SbxResult<String> {
            unreachable!("no sandboxes in a script test")
        }
        fn remove_secret(&self, _: Option<&str>, _: &str) -> SbxResult<()> {
            unreachable!("no sandboxes in a script test")
        }
        fn templates(&self) -> SbxResult<Vec<Template>> {
            unreachable!("no sandboxes in a script test")
        }
        fn load_template(&self, _: &Path) -> SbxResult<()> {
            unreachable!("no sandboxes in a script test")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The image bakes these paths in: `hura-status` writes `status.json` from a
    /// hook inside the sandbox, and the seeder's script is written to a fixed
    /// place so a second `hura` can find it. If [`Paths`] drifted from the
    /// constants, the host would be reading files nothing writes.
    #[test]
    fn the_sandbox_paths_are_the_ones_the_image_uses() {
        let p = Paths::in_sandbox();
        assert_eq!(p.repo, session::REPO_PATH);
        assert_eq!(p.meta(), session::META_PATH);
        assert_eq!(p.task(), session::TASK_PATH);
        assert_eq!(p.status(), session::STATUS_PATH);
        assert_eq!(p.seed_state(), session::SEED_STATE_PATH);
        assert_eq!(p.seed_log(), session::SEED_LOG_PATH);
        assert_eq!(p.seed_script(), session::SEED_SCRIPT_PATH);
    }
}
