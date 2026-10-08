//! Where a session's work actually happens.
//!
//! [`crate::ops`], [`crate::git`], [`crate::files`] and [`crate::seed`] all talk
//! to a [`Backend`] rather than to the gateway directly. The scripts -- the
//! diff, the poll, the status scrape, the file tree, the review, the shells --
//! are shared and pure; where they run, where the files are and how tmux is
//! invoked is this trait's business rather than theirs. There is one
//! implementation, [`Sandboxed`]: a session inside an OpenShell sandbox, with
//! the gateway's policy on everything that leaves it.
//!
//! There used to be a second, a plain `git worktree` on the server with no
//! isolation at all. It was removed: two backends meant two of everything to
//! keep working, and the isolation is the product.

use openshell_client::{PolicyRevision, PolicyUpdate};

use crate::doctor::Check;
use crate::events::Event;
use crate::ops::ProviderChoice;
use crate::session::{self, Session};

mod sandboxed;

pub use sandboxed::{Sandboxed, Settings};

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
    fn paths(&self, session: &Session) -> Paths;

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
    /// `port` inside this session. See [`OpenShell::forward_argv`].
    fn forward_argv(
        &self,
        session: &Session,
        port: u16,
        host: crate::ports::Loopback,
    ) -> Result<Vec<String>>;

    /// How to invoke tmux where this session's agent runs.
    ///
    /// The image ships a config and a sandbox exec inherits no locale, so this
    /// carries both. `-u` says "this terminal is UTF-8" outright rather than
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
    /// adopt. Imposing MCP endpoints is a `policy update --wait`, which made
    /// that window seconds wide.
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

    /// The effective policy.
    fn policy(&self, session: &Session) -> Result<PolicyRevision>;

    fn policy_update(&self, session: &Session, update: &PolicyUpdate) -> Result<()>;

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

/// The backend every session runs on, as configured.
///
/// One of them since the worktree backend went; kept as the thing callers are
/// handed so that a session-shaped call site reads the same as it always has.
pub struct Backends {
    sandboxed: Sandboxed,
}

impl Backends {
    pub fn new(sandboxed: Sandboxed) -> Self {
        Backends { sandboxed }
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
        Backends::from_client(Box::new(client), Settings::from_config(cfg))
    }

    pub fn for_session(&self, _session: &Session) -> &dyn Backend {
        &self.sandboxed
    }

    /// The backend itself, for the operations that are not about one session:
    /// listing what exists, creating something new, and `hurad doctor`.
    pub fn sandboxed(&self) -> &dyn Backend {
        &self.sandboxed
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
