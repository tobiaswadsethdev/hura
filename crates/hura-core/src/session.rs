//! Session identity and the metadata record.
//!
//! A session is a task plus the sandbox running it. The **sandbox is the
//! source of truth**: seeding writes `/sandbox/.hura/meta.json` inside it, so a
//! session survives losing the local cache entirely. Its name says which
//! session it is (`hura-<session>`), since the sandbox runtime has no labels.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Where the metadata record lives inside the sandbox.
pub const META_PATH: &str = "/sandbox/.hura/meta.json";
/// Where the repository is cloned inside the sandbox.
pub const REPO_PATH: &str = "/sandbox/repo";
/// The task prompt, written as a plain file so the shell can read it without
/// any nested quoting.
pub const TASK_PATH: &str = "/sandbox/.hura/task.txt";
/// Where the agent's hooks record what it is doing. Written by `hura-status`,
/// which the image bakes in; see `images/hura-base/hura-status`.
pub const STATUS_PATH: &str = "/sandbox/.hura/status.json";
/// Where the seeder reports how far it has got. Written inside the sandbox by a
/// process that outlives the command that started it, which is what lets a clone
/// survive the tool quitting; see [`crate::seed`].
pub const SEED_STATE_PATH: &str = "/sandbox/.hura/seed.state";
/// The seeder's own output, kept for when it fails.
pub const SEED_LOG_PATH: &str = "/sandbox/.hura/seed.log";
/// Where the seeder script is written before being run detached.
pub const SEED_SCRIPT_PATH: &str = "/sandbox/.hura/seed.sh";
/// Name of the tmux session **inside** the sandbox that the agent runs in.
///
/// tmux runs in the sandbox rather than on the host so the agent survives
/// losing its connection, and so its output can be scraped with capture-pane
/// without depending on anything host-side.
pub const TMUX_SESSION: &str = "agent";
/// What a work branch is named under unless the config file says otherwise.
///
/// Every session's branch has been `hura/<name>` until now, and this keeps that
/// the default. The reason it is configurable at all is tickets: a
/// session started from a ticket wants the branch its tracker's commit hooks
/// and its reviewers already look for, which is `<you>/PROJ-123-description`.
pub const DEFAULT_BRANCH_PREFIX: &str = "hura";

/// Container image hura runs sandboxes from: Docker's `shell-docker` sandbox
/// template plus tmux, Claude Code and hura's own scripts.
pub const IMAGE: &str = "hura-sandbox:latest";
/// The repository half of [`IMAGE`], which the toolchain variants share.
///
/// Its own constant because a variant tag is built from it -- `hura-sandbox:dotnet`
/// beside `hura-sandbox:latest` -- and two `format!`s spelling the name out would be
/// two places to rename it. See [`crate::toolchain::tag`].
pub const IMAGE_REPO: &str = "hura-sandbox";

/// Prefix applied to sandbox and tmux names, so ours are recognisable in
/// `sbx ls` beside sandboxes created by hand, and so a sandbox says which
/// session it is: everything after the prefix.
const PREFIX: &str = "hura-";

/// How long a session name may be.
///
/// The name is also a branch (`hura/<name>`), a column in the list, and
/// something you type after `hura attach`. The sandbox, `hura-` and the name,
/// stays well inside the 63 characters the runtime accepts.
const MAX_NAME: usize = 40;

/// The size to leave the agent's tmux window at when nothing is attached.
///
/// The status scraper reads that window, and Claude Code's footer -- where the
/// running marker lives -- is truncated to the pane, so the width decides
/// whether the state column can tell working from idle. Matches the image's
/// `default-size`; `image.rs` has a test that it does.
pub const SCRAPE_SIZE: (u16, u16) = (200, 50);

/// The sandbox a session of this name owns.
///
/// The convention, in one place. Deleting and adopting both have to name a
/// sandbox without a record to read it from, which is the whole point of
/// having a convention, and two copies of this `format!` would be two things
/// to keep in step with [`Session::new`].
pub fn sandbox_name(name: &str) -> String {
    format!("{PREFIX}{name}")
}

/// The session a sandbox belongs to, read off its name. `None` for a sandbox
/// that is not one of hura's, which includes one that only starts with the
/// prefix but could not have been made from a session name.
pub fn session_of(sandbox: &str) -> Option<&str> {
    let name = sandbox.strip_prefix(PREFIX)?;
    validate_name(name).is_ok().then_some(name)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NameError {
    #[error("name is empty")]
    Empty,
    #[error("name is longer than {MAX_NAME} characters")]
    TooLong,
    #[error("name must be lowercase letters, digits and dashes; `{0}` is not allowed")]
    BadChar(char),
    #[error("name must start and end with a letter or digit")]
    BadEdge,
}

/// Turn arbitrary text into a session name: lowercase, dashes, trimmed.
///
/// Used to derive a name from a task description when the user does not supply
/// one. Returns `None` if nothing usable survives.
/// Words a task wraps its subject in, dropped when deriving a name.
///
/// A task is written as a sentence -- "I want to add the MaxGaming Scala
/// customer id" -- and the first words of a sentence are almost never what it
/// is about. Keeping them spent the whole budget on `i-want-to-add`, which
/// names nothing. Verbs are *not* here: `add`, `fix`, `remove` and `update`
/// distinguish two tasks about the same subject.
const FILLER: &[&str] = &[
    "a", "also", "an", "and", "any", "are", "at", "be", "can", "could", "do", "does", "for",
    "from", "i", "in", "into", "is", "it", "its", "just", "let", "lets", "me", "my", "need",
    "needs", "of", "on", "or", "our", "please", "shall", "should", "some", "that", "the", "there",
    "these", "this", "to", "us", "want", "we", "will", "would", "you", "your",
];

pub fn slugify(text: &str) -> Option<String> {
    // Twice: once keeping only the words that carry meaning, and -- if that
    // leaves nothing, as "can you do it for me" would -- once taking the text as
    // written. A name is better than no name.
    slug(text, true).or_else(|| slug(text, false))
}

fn slug(text: &str, drop_filler: bool) -> Option<String> {
    // Split on anything that is not alphanumeric, then re-join with dashes,
    // dropping whole words once the budget is spent. Truncating mid-word would
    // turn "readme" into "read" and read as a different task.
    let mut out = String::new();
    for word in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        let word = word.to_ascii_lowercase();
        if drop_filler && FILLER.contains(&word.as_str()) {
            continue;
        }
        let extra = if out.is_empty() {
            word.len()
        } else {
            word.len() + 1
        };
        if out.len() + extra > MAX_NAME {
            // A single first word longer than the budget still has to yield
            // something, so hard-truncate only in that case.
            if out.is_empty() {
                out = word.chars().take(MAX_NAME).collect();
            }
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&word);
    }
    let trimmed = out.trim_matches('-').to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// A session name from the task, falling back to the repository's last path
/// segment.
///
/// Shared by `hura new` and the TUI's create form, so the name a session gets is
/// the same however it was started -- and so the form can show the name it is
/// about to use while the task is still being typed.
pub fn derive_name(task: &str, repo: &str) -> Option<String> {
    slugify(task).or_else(|| {
        repo.trim_end_matches('/')
            .rsplit('/')
            .next()
            .map(|s| s.trim_end_matches(".git"))
            .and_then(slugify)
    })
}

/// A derived name that is not already taken.
///
/// Two sessions in the same repository is the normal case -- try something, try
/// something else -- and with no task typed yet both derive the repository's own
/// name. Refusing the second one until the name is edited by hand makes the
/// common case the one that needs work, so a counter is appended instead:
/// `api-server`, `api-server-2`, `api-server-3`.
///
/// The base is shortened to make room for the suffix rather than the suffix being
/// dropped, because the gateway's name budget is the hard part and a name that no
/// longer fits it would be refused three steps later. Unlike [`slugify`], which
/// drops whole words, this cuts mid-word if it has to: `fix-the-readm-2` still
/// reads as a variant of the same thing, where `fix-the-2` would not.
pub fn unique_name(base: &str, taken: &[String]) -> String {
    let is_free = |candidate: &str| !taken.iter().any(|t| t == candidate);
    if is_free(base) {
        return base.to_string();
    }
    for n in 2..=99u32 {
        let suffix = format!("-{n}");
        let room = MAX_NAME.saturating_sub(suffix.len());
        let stem: String = base.chars().take(room).collect();
        // Trimming can leave a trailing dash, which `validate_name` rejects.
        let candidate = format!("{}{suffix}", stem.trim_end_matches('-'));
        if is_free(&candidate) {
            return candidate;
        }
    }
    // A hundred sessions of one name is not a case worth handling; hand back the
    // base and let the collision be reported as it was before.
    base.to_string()
}

/// The provider profile type carrying an agent's credential.
///
/// Used to preselect a provider in the create form: a session started without
/// the agent's credential comes up to a login prompt, which is a poor way to
/// find out. `None` for an agent whose credentials hura knows nothing about.
pub fn agent_provider_type(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" => Some("claude-code-oauth"),
        _ => None,
    }
}

/// Validate a session name against both our rules and the gateway's.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.len() > MAX_NAME {
        return Err(NameError::TooLong);
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
    {
        return Err(NameError::BadChar(bad));
    }
    let edges_ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    if !name.starts_with(edges_ok) || !name.ends_with(edges_ok) {
        return Err(NameError::BadEdge);
    }
    Ok(())
}

/// Whether a string is usable as a git branch name.
///
/// A subset of what `git check-ref-format` allows, and deliberately: the branch
/// arrives from a client, is interpolated into shell scripts inside the sandbox
/// and is pushed to a remote, so what it may contain is decided by shape rather
/// than by escaping. `#` is in the set because Azure DevOps work item keys are
/// `AB#1234` and git is perfectly happy with one.
pub fn validate_branch(branch: &str) -> Result<(), BranchError> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Err(BranchError::Empty);
    }
    if branch.len() > MAX_BRANCH {
        return Err(BranchError::TooLong);
    }
    if let Some(bad) = branch
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-' | '#')))
    {
        return Err(BranchError::BadChar(bad));
    }
    // Each of these is a name git itself refuses, and refusing them here means
    // the failure lands on the request rather than on a push three minutes
    // later.
    if branch.contains("..")
        || branch.contains("//")
        || branch.starts_with(['/', '-', '.'])
        || branch.ends_with(['/', '.'])
        || branch.ends_with(".lock")
    {
        return Err(BranchError::BadShape);
    }
    Ok(())
}

/// Long enough for `name/PROJ-1234-a-description-of-some-length`, short enough
/// that it is still a branch name rather than a paragraph.
const MAX_BRANCH: usize = 120;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BranchError {
    #[error("branch name is empty")]
    Empty,
    #[error("branch name is longer than {MAX_BRANCH} characters")]
    TooLong,
    #[error("branch names may hold letters, digits and `._/-#`; `{0}` is not allowed")]
    BadChar(char),
    #[error(
        "that is not a branch name git would accept: no `..` or `//`, and it may not start with \
         `/`, `-` or `.` or end with `/`, `.` or `.lock`"
    )]
    BadShape,
}

/// Lifecycle state.
///
/// The agent-derived states (`Running`, `Waiting`, `Idle`) are not set yet;
/// status detection arrives in a later increment. Until then a healthy session
/// sits in `Ready`.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Creating,
    Seeding,
    /// `published` too: that state went with `hurad publish`, and a record
    /// written while it existed is a session whose sandbox is still ready.
    #[serde(alias = "published")]
    Ready,
    Running,
    Waiting,
    Idle,
    Failed,
    /// The sandbox backing this session is gone.
    Dead,
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            State::Creating => "creating",
            State::Seeding => "seeding",
            State::Ready => "ready",
            State::Running => "running",
            State::Waiting => "waiting",
            State::Idle => "idle",
            State::Failed => "failed",
            State::Dead => "dead",
        };
        // `pad`, not `write_str`: a Display impl that writes directly ignores
        // the formatter's width, so `{:<9}` silently does nothing and the list
        // columns run into each other.
        f.pad(s)
    }
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub name: String,
    /// The sandbox this session owns, derived from its name.
    pub sandbox: String,
    /// Name of the tmux session inside the sandbox. Stored rather than assumed
    /// so an older session keeps working if the default ever changes.
    pub tmux: String,
    pub repo: String,
    /// The project this worktree belongs to, by name. `None` for a session
    /// created before projects existed, or from the command line, which does
    /// not have them: the tree shows those grouped by their clone URL instead
    /// of pretending they belong somewhere.
    ///
    /// Recorded rather than matched back by `repo`, because two projects may
    /// share a clone URL -- two checkouts of one repository is a normal thing
    /// to have -- and a worktree would then belong to both.
    #[serde(default)]
    pub project: Option<String>,
    /// Branch cloned from; `None` means the remote's default.
    #[serde(default)]
    pub base_branch: Option<String>,
    pub work_branch: String,
    #[serde(default)]
    pub task: String,
    /// The ticket this session was started from, if it was started from one.
    ///
    /// A note about where the work came from; nothing writes back through it.
    /// See [`crate::tracker::Ticket`].
    #[serde(default)]
    pub ticket: Option<crate::tracker::Ticket>,
    #[serde(default)]
    pub policy: Option<String>,
    #[serde(default)]
    pub providers: Vec<String>,
    /// Skills copied into this session when it was created, and where each came
    /// from on the host. A copy, not a link -- see [`crate::skills`] -- so this
    /// says what the agent has, whatever the host's copy says now.
    #[serde(default)]
    pub skills: Vec<crate::skills::Skill>,
    /// MCP servers this session's agent was given, as the config file named
    /// them when it was created.
    ///
    /// Recorded rather than re-read, for the reason the whole record exists: the
    /// sandbox is the source of truth about itself. The config file may have
    /// changed since, and what matters for reading -- or re-seeding -- this
    /// session is what it was actually created with.
    #[serde(default)]
    pub mcp: Vec<crate::mcp::Server>,
    /// Toolchains this session's image carries, by name.
    ///
    /// Recorded for the reason the whole record exists: the sandbox is the source
    /// of truth about itself. The image variant it was created from may have been
    /// rebuilt or deleted since, and what matters for reading this session is
    /// what it was actually given -- which is also what says why its policy holds
    /// a registry endpoint no template grants.
    #[serde(default)]
    pub toolchains: Vec<String>,
    #[serde(default = "default_agent")]
    pub agent: String,
    /// Whether the agent is a terminal or a chat. A record from before there
    /// was a choice is a terminal, which is what it was.
    #[serde(default)]
    pub interface: crate::chat::Interface,
    /// Epoch seconds. Deliberately not a formatted timestamp: the display wants
    /// a relative age, and storing epoch avoids a date-library dependency.
    // `number`, not the `bigint` ts-rs assumes for a u64: serde_json writes it
    // as a JSON number and `JSON.parse` reads one back, so `bigint` would be a
    // type the runtime never produces. Epoch seconds are exact in a double
    // until the year 285000000.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_at: u64,
    pub state: State,
    /// Why the session failed, in the words of whatever refused it: the
    /// gateway turning down a create, a clone the seeder gave up on.
    ///
    /// On the record because the record is the only thing a window reads. A
    /// create on `hurad` runs on a thread whose one way to speak is its own
    /// terminal, so without this a sandbox the gateway refused showed up as a
    /// `dead` row whose panes each said `sandbox not found`, and the reason
    /// was on a screen nobody was looking at.
    ///
    /// Kept when a failed session goes `dead`, which is what one whose sandbox
    /// was never made does on the next refresh, and dropped when a session
    /// comes back to life. `None` on a `dead` session means its sandbox went
    /// away later, not that it never had one.
    #[serde(default)]
    pub failure: Option<String>,
}

fn default_agent() -> String {
    "claude".to_string()
}

pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Render an age like `3m`, `2h`, `4d` for the list view.
pub fn humanize_age(created_at: u64, now: u64) -> String {
    let secs = now.saturating_sub(created_at);
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

impl Session {
    pub fn new(name: String, repo: String, task: String) -> Self {
        Session {
            sandbox: sandbox_name(&name),
            tmux: TMUX_SESSION.to_string(),
            work_branch: format!("{DEFAULT_BRANCH_PREFIX}/{name}"),
            name,
            repo,
            project: None,
            base_branch: None,
            task,
            ticket: None,
            policy: None,
            providers: Vec::new(),
            skills: Vec::new(),
            mcp: Vec::new(),
            toolchains: Vec::new(),
            agent: default_agent(),
            interface: crate::chat::Interface::Terminal,
            created_at: now_epoch(),
            state: State::Creating,
            failure: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record written while there were worktree sessions still reads: the
    /// `backend` and `workdir` fields it carries are ignored rather than being
    /// a deserialization error that reads as a corrupt cache. A worktree
    /// record is then one whose sandbox the gateway has never heard of, which
    /// a refresh marks dead and `hurad rm` removes.
    #[test]
    fn a_record_from_a_worktree_session_still_reads() {
        let old = r#"{
            "name": "readme-fix",
            "backend": "worktree",
            "sandbox": "hura-readme-fix",
            "workdir": "/home/you/.local/share/hura/worktrees/readme-fix",
            "tmux": "hura-readme-fix",
            "repo": "https://github.com/you/hura.git",
            "work_branch": "hura/readme-fix",
            "created_at": 1788000000,
            "state": "ready"
        }"#;
        let s: Session = serde_json::from_str(old).expect("an older record still reads");
        assert_eq!(s.sandbox, "hura-readme-fix");
    }

    /// A variant tag is `IMAGE_REPO` plus the toolchains, and the base image is
    /// the same repository with `latest`. If those two drifted apart, every
    /// variant would be built under a name nothing looks for.
    #[test]
    fn the_base_image_and_the_variants_share_a_repository() {
        assert_eq!(IMAGE, format!("{IMAGE_REPO}:latest"));
    }

    /// Two sessions in one repository is the normal case, and with no task typed
    /// both derive the repository's name. The second must not need hand-editing.
    #[test]
    fn a_taken_name_gets_a_counter() {
        let taken = vec!["api-server".to_string()];
        assert_eq!(unique_name("api-server", &taken), "api-server-2");
        assert_eq!(
            unique_name("other", &taken),
            "other",
            "free names are left be"
        );

        let taken = vec!["api-server".into(), "api-server-2".into()];
        assert_eq!(unique_name("api-server", &taken), "api-server-3");
    }

    /// The suffix has to fit inside the gateway's budget, or the name it produces
    /// is refused three steps later.
    #[test]
    fn the_counter_fits_the_name_limit() {
        let base = "a".repeat(MAX_NAME);
        let taken = vec![base.clone()];
        let next = unique_name(&base, &taken);
        assert!(next.len() <= MAX_NAME, "`{next}` is {} long", next.len());
        assert!(
            validate_name(&next).is_ok(),
            "{next}: {:?}",
            validate_name(&next)
        );
        assert!(next.ends_with("-2"), "{next}");
    }

    /// Shortening the stem can leave it ending in a dash, which the gateway's
    /// name rules reject.
    #[test]
    fn the_shortened_stem_never_ends_in_a_dash() {
        // 13 characters with a dash where the truncation lands.
        let base = "aaaaaaaaaaaa-b";
        let taken = vec![base.to_string()];
        let next = unique_name(base, &taken);
        assert!(validate_name(&next).is_ok(), "{next}");
        assert!(!next.contains("--"), "{next}");
    }

    #[test]
    fn slugifies_task_text() {
        assert_eq!(
            slugify("Add OAuth login!").as_deref(),
            Some("add-oauth-login")
        );
        assert_eq!(slugify("  fix   the BUG  ").as_deref(), Some("fix-bug"));
        assert_eq!(slugify("!!!"), None);
        assert_eq!(slugify(""), None);
    }

    /// The name that started this: fifteen characters of "I want to add the
    /// MaxGaming Scala customer id" was `i-want-to-add`, which says nothing
    /// about the task at all.
    #[test]
    fn filler_words_do_not_get_to_spend_the_budget() {
        assert_eq!(
            slugify("I want to add the MaxGaming Scala customer id").as_deref(),
            Some("add-maxgaming-scala-customer-id")
        );
        assert_eq!(
            slugify("Can you please update the changelog for me").as_deref(),
            Some("update-changelog")
        );
        // Verbs stay: they are what tells two tasks about one subject apart.
        assert_eq!(slugify("remove the flag").as_deref(), Some("remove-flag"));
        assert_eq!(slugify("add the flag").as_deref(), Some("add-flag"));
    }

    /// A task made entirely of filler still has to produce a name.
    #[test]
    fn a_task_of_nothing_but_filler_falls_back_to_the_words_it_has() {
        assert_eq!(
            slugify("can you do it for me").as_deref(),
            Some("can-you-do-it-for-me")
        );
    }

    /// Long enough to say what the session is, and still a legal branch and
    /// list column.
    #[test]
    fn a_long_name_is_still_a_session() {
        let long = "add-maxgaming-scala-customer-id-to-prod";
        assert!(long.len() <= MAX_NAME);
        assert_eq!(validate_name(long), Ok(()));

        let s = Session::new(long.into(), "https://example.com/r.git".into(), "t".into());
        assert_eq!(s.work_branch, format!("hura/{long}"));
        assert_eq!(s.sandbox, format!("hura-{long}"));
    }

    /// The sandbox is a pure function of the session name and the session is
    /// read back off it, which is what lets `hura rm` and adoption name a
    /// sandbox with no record to read.
    #[test]
    fn a_sandbox_says_which_session_it_is() {
        for name in ["add-auth", "data-239-all-campaign-costs-from-google"] {
            assert_eq!(session_of(&sandbox_name(name)), Some(name));
        }
        // The longest name fits what the runtime accepts, measured at 63.
        assert!(sandbox_name(&"a".repeat(MAX_NAME)).len() <= 63);
        // Not ours: no prefix, or something after it no session could be.
        assert_eq!(session_of("claude-workspace"), None);
        assert_eq!(session_of("hura-"), None);
        assert_eq!(session_of("hura-Spike_A"), None);
    }

    #[test]
    fn derives_a_name_from_the_task_then_the_repo() {
        assert_eq!(
            derive_name("Fix the README typo", "https://github.com/o/r.git").as_deref(),
            Some("fix-readme-typo")
        );
        // No task: the repository's own name is the next best thing.
        assert_eq!(
            derive_name("", "https://github.com/o/hello-world.git").as_deref(),
            Some("hello-world")
        );
        assert_eq!(
            derive_name("", "https://dev.azure.com/org/proj/_git/repo").as_deref(),
            Some("repo")
        );
        assert_eq!(derive_name("", "!!!"), None);
    }

    #[test]
    fn knows_which_provider_type_carries_the_agent_credential() {
        assert_eq!(agent_provider_type("claude"), Some("claude-code-oauth"));
        assert_eq!(agent_provider_type("codex"), None);
    }

    #[test]
    fn rejects_names_the_gateway_would_reject() {
        assert_eq!(validate_name(""), Err(NameError::Empty));
        assert_eq!(validate_name("Add-Auth"), Err(NameError::BadChar('A')));
        assert_eq!(validate_name("add/auth"), Err(NameError::BadChar('/')));
        assert_eq!(validate_name("-auth"), Err(NameError::BadEdge));
        assert_eq!(validate_name("auth-"), Err(NameError::BadEdge));
        assert_eq!(
            validate_name(&"a".repeat(MAX_NAME + 1)),
            Err(NameError::TooLong)
        );
        assert!(validate_name("add-auth-2").is_ok());
    }

    #[test]
    fn derives_consistent_identifiers() {
        let s = Session::new(
            "add-auth".into(),
            "https://example.com/r.git".into(),
            "task".into(),
        );
        assert_eq!(s.sandbox, "hura-add-auth");
        assert_eq!(s.tmux, TMUX_SESSION);
        assert_eq!(s.work_branch, "hura/add-auth");
    }

    /// `published` went with `hurad publish`. A record still carrying it is a
    /// session whose sandbox is ready, not a cache that fails to parse.
    #[test]
    fn a_published_record_reads_as_ready() {
        let s: State = serde_json::from_str("\"published\"").unwrap();
        assert_eq!(s, State::Ready);
    }

    #[test]
    fn state_display_honours_a_width() {
        assert_eq!(format!("{:<9}|", State::Ready), "ready    |");
        assert_eq!(format!("{:<9}|", State::Failed), "failed   |");
        assert_eq!(format!("{}", State::Waiting), "waiting");
    }

    #[test]
    fn humanizes_age() {
        assert_eq!(humanize_age(0, 30), "30s");
        assert_eq!(humanize_age(0, 300), "5m");
        assert_eq!(humanize_age(0, 7200), "2h");
        assert_eq!(humanize_age(0, 172_800), "2d");
        // Clock skew must not panic.
        assert_eq!(humanize_age(100, 0), "0s");
    }

    #[test]
    fn session_json_roundtrips() {
        let s = Session::new("x".into(), "r".into(), "t".into());
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Session>(&json).unwrap(), s);
    }
}
