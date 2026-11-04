//! Defaults read from a file, so they stop being flags on every command.
//!
//! `$XDG_CONFIG_HOME/hura/config.toml`, beside the session cache and the events
//! history. Everything in it is optional and everything in it is a *default*:
//! a flag on the command line always wins, and so does an explicit choice in the
//! TUI's create form. The file is what stops `--policy feature-work --provider
//! claude-oauth --provider azure-pat` from being typed out again every time.
//!
//! **A file that cannot be read is an error, not a shrug.** A typo'd key or a
//! misspelled policy name that silently did nothing would be the same failure as
//! a sandbox reporting a policy it is not enforcing: the tool would say one thing
//! and do another. So unknown keys are rejected, a policy name that is not a
//! template is rejected, and every command except `hura doctor` refuses to run
//! until the file is fixed. `doctor` is the command you reach for when something
//! is wrong, so it reports the error as a failed check and carries on with the
//! built-in defaults.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::chat;
use crate::mcp;
use crate::policy;
use crate::session;
use crate::skills;
use crate::store::Store;

/// A starter file, written by `hura config --init`.
///
/// Every key commented out and set to what the tool already does, so the file
/// documents the defaults rather than changing them the moment it is created.
pub const EXAMPLE: &str = include_str!("config.example.toml");

/// Bounds on `refresh`. Below the floor the TUI's 100ms input tick becomes the
/// limit -- the polls would land faster than the frames that draw them, which
/// buys nothing and costs a `git status` inside every sandbox; above the ceiling
/// the list has stopped being live and the tool would be better closed.
const REFRESH_MIN: Duration = Duration::from_millis(250);
const REFRESH_MAX: Duration = Duration::from_secs(60);

/// The defaults, and where they came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// The file this was read from. Present even when the file does not exist,
    /// because `hura config` has to be able to say where to create it.
    pub path: PathBuf,
    /// Whether the file existed. The difference between "these are the defaults"
    /// and "this is what you asked for".
    pub present: bool,

    /// The OpenShell gateway sessions used to run on. Nothing reads it since
    /// they moved to Docker Sandboxes; still parsed so existing files load.
    pub gateway: Option<String>,
    /// Repository a `hura new` without `--repo` clones, and the row the TUI's
    /// picker opens on.
    pub repo: Option<String>,
    /// Branch to clone from. `None` means the remote's default.
    pub base: Option<String>,
    /// Policy template name, or a path to a YAML file.
    pub policy: Option<String>,
    /// Credentials a new session is given unless the create form says
    /// otherwise, by name from [`Self::credentials`].
    pub providers: Option<Vec<String>>,
    /// Every credential a session may be given, from `[credentials.NAME]`.
    pub credentials: Vec<crate::credentials::Credential>,
    /// Whole CPUs per session sandbox. Unset is the runtime's default, which
    /// is every host CPU for every sandbox.
    pub sandbox_cpus: Option<u32>,
    /// Memory per session sandbox, as `8g` or `512m`. Unset is the runtime's
    /// default, half the host's memory for every sandbox.
    pub sandbox_memory: Option<String>,
    /// The `sbx` binary, when it is neither on `PATH` nor where its installer
    /// puts it.
    pub sbx: Option<PathBuf>,
    /// Where the TUI's picker looks for repositories. Replaces the built-in
    /// roots rather than adding to them, like `HURA_REPO_ROOTS`, which still wins.
    pub repo_roots: Option<Vec<PathBuf>>,
    /// Whether a new session may be a plain `git worktree` on this machine
    /// instead of a sandbox. Off unless the file says so: a worktree session
    /// runs with the server's own rights, and a server shared with a phone is
    /// not one where that should be a tap away by default.
    pub worktrees: bool,
    /// Where worktree sessions put their working copies, one directory each.
    /// `None` is [`crate::backend::Worktree::default_root`].
    pub worktree_root: Option<PathBuf>,
    /// How often the terminal interface read the sandboxes. Nothing reads it
    /// since that went in v0.4.0; it is still parsed so existing config files
    /// keep loading. See:
    /// this is one number scaling a set of measured ones, because they are
    /// related to each other and a single absolute interval would break the
    /// relationships.
    pub refresh: Option<Duration>,

    /// Whether `hurad serve` may fetch a newer release in the background.
    ///
    /// It never replaces a running binary. What it does is download and verify
    /// ahead of time and leave the result beside the current one; the swap
    /// happens at the next process start. `false` turns the check off
    /// altogether, for a machine that would rather not reach github at all.
    pub auto_update: Option<bool>,
    /// Where a missing sandbox image is pulled from. `None` is
    /// [`crate::image::REGISTRY`], and an empty string is nowhere: every image
    /// is built on this machine, as before images were published.
    pub image_registry: Option<String>,
    /// Skills copied into every new session, already resolved to host paths.
    ///
    /// Global for the same reason as [`Self::mcp`]: this is what an agent of
    /// yours knows how to do, not a per-session choice.
    pub skills: Vec<skills::Skill>,
    /// MCP servers every new session's agent is given, already validated.
    ///
    /// Not a per-session choice and so not on [`crate::ops::Draft`]: like the
    /// global endpoint lists, this is one decision about what an agent of yours
    /// can reach, made once. A session records the servers it was created with,
    /// so changing the file changes the next session rather than this one.
    pub mcp: Vec<mcp::Entry>,
    /// What a work branch is named under: `<prefix>/<name>`.
    ///
    /// `hura` by default, which is what every session has been called until now.
    /// Set it to your own name and a session started from a ticket is
    /// `tobias/PROJ-123-add-the-changelog` -- the convention a tracker's own
    /// commit hooks and every reviewer already look for.
    pub branch_prefix: Option<String>,
    /// Whether a new session's agent is a chat or a terminal. `None` is a
    /// chat; see [`Self::interface`].
    pub interface: Option<chat::Interface>,
}

impl Config {
    /// `$XDG_CONFIG_HOME/hura/config.toml`, beside the session cache.
    pub fn default_path() -> PathBuf {
        Store::default_path().with_file_name("config.toml")
    }

    pub fn load() -> Result<Self, Error> {
        Self::load_from(Self::default_path())
    }

    pub fn load_from(path: impl Into<PathBuf>) -> Result<Self, Error> {
        let path = path.into();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            // No file is the normal case: the defaults are meant to be good.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Config {
                    path,
                    ..Config::default()
                });
            }
            Err(source) => return Err(Error::Read { path, source }),
        };
        Self::parse(&path, &text)
    }

    /// Parse and validate. Separate from the read so the tests need no files.
    pub fn parse(path: &Path, text: &str) -> Result<Self, Error> {
        let mut raw: Raw = toml::from_str(text).map_err(|e| Error::Parse {
            path: path.to_path_buf(),
            // toml's own message already carries the line, the column and a
            // caret; reprinting it is better than summarising it.
            message: e.to_string().trim_end().to_string(),
        })?;

        // Blanks become unset before anything is validated, so `policy = ""`
        // means "not decided" rather than "a template with no name".
        raw.gateway = blank_to_none(raw.gateway);
        raw.repo = blank_to_none(raw.repo);
        raw.base = blank_to_none(raw.base);
        raw.policy = blank_to_none(raw.policy);

        let invalid = |key: &'static str, message: String| Error::Invalid {
            path: path.to_path_buf(),
            key,
            message,
        };

        // A policy that is neither a template nor a path is a typo, and finding
        // out at create time -- after a sandbox exists -- is finding out late.
        if let Some(spec) = &raw.policy
            && !policy::looks_like_path(spec)
            && !policy::is_template(spec)
        {
            return Err(invalid(
                "policy",
                format!(
                    "`{spec}` is not a template; expected one of {}, or a path to a template file",
                    names()
                ),
            ));
        }

        let interface = match blank_to_none(raw.interface.take()) {
            None => None,
            Some(text) => Some(chat::Interface::parse(&text).ok_or_else(|| {
                invalid(
                    "interface",
                    format!("`{text}` is not an interface; expected \"chat\" or \"terminal\""),
                )
            })?),
        };

        let refresh = match &raw.refresh {
            None => None,
            Some(raw) => {
                let d = parse_duration(raw).map_err(|m| invalid("refresh", m))?;
                if d < REFRESH_MIN || d > REFRESH_MAX {
                    return Err(invalid("refresh", format!("`{raw}` is outside 250ms..60s")));
                }
                Some(d)
            }
        };

        // An empty list is a mistake worth naming: `providers = []` reads as
        // "no credentials", which produces a sandbox whose agent cannot log in
        // and whose clone cannot authenticate, three steps later.
        if raw.providers.as_ref().is_some_and(Vec::is_empty) {
            return Err(invalid(
                "providers",
                "is empty; remove the key to accept the defaults".to_string(),
            ));
        }
        if raw.repo_roots.as_ref().is_some_and(Vec::is_empty) {
            return Err(invalid(
                "repo_roots",
                "is empty; remove the key to scan the usual places".to_string(),
            ));
        }

        // Resolved but not checked against the filesystem: a skill directory
        // that is temporarily gone -- a repository not cloned yet, an external
        // drive not mounted -- should not stop every command from running. The
        // shape is checked here, existence is `hura doctor`'s to report and a
        // create-time warning otherwise.
        let mut resolved_skills = Vec::new();
        for entry in raw.skills.iter().flatten() {
            let skill = skills::Skill::parse(entry)
                .map_err(|e| invalid("skills", format!("`{entry}` {e}")))?;
            if resolved_skills
                .iter()
                .any(|s: &skills::Skill| s.name == skill.name)
            {
                return Err(invalid(
                    "skills",
                    format!(
                        "`{}` is the name of two skills; the agent keys them by \
                         directory name, so one would hide the other",
                        skill.name
                    ),
                ));
            }
            resolved_skills.push(skill);
        }
        if raw.skills.as_ref().is_some_and(Vec::is_empty) {
            return Err(invalid(
                "skills",
                "is empty; remove the key to copy none".to_string(),
            ));
        }

        // Validated here, where the error can name the file and the entry,
        // rather than at create time: a URL the sandbox cannot reach is worth
        // finding out about before a sandbox exists, and a loopback URL -- the
        // one mistake everybody makes -- looks perfectly fine until the agent
        // is running.
        let mut mcp: Vec<mcp::Entry> = Vec::new();
        for entry in raw.mcp.into_iter().flatten() {
            let named = |e: String| invalid("mcp", format!("`{}`: {e}", entry.name));
            let transport = match &entry.transport {
                Some(t) => mcp::Transport::parse(t).map_err(|e| named(e.to_string()))?,
                None => mcp::Transport::default(),
            };
            // Two shapes, and exactly one of them per table. A `url` is a
            // server somebody else runs; an `image` is one this server runs and
            // whose url it derives -- and an entry with both would be a url
            // pointing somewhere other than the container beside it, which is
            // the sort of thing nobody notices until the agent reports a dead
            // tool.
            let resolved = match (&entry.url, &entry.image) {
                (Some(_), Some(_)) => {
                    return Err(named(
                        "has both a url and an image; a url is a server you run                          elsewhere, an image is one this server runs"
                            .into(),
                    ));
                }
                (None, None) => {
                    return Err(named("has neither a url nor an image".into()));
                }
                (Some(url), None) => {
                    for (key, present) in [
                        ("port", entry.port.is_some()),
                        ("args", entry.args.is_some()),
                        ("env", entry.env.is_some()),
                        ("secrets", entry.secrets.is_some()),
                    ] {
                        if present {
                            return Err(named(format!(
                                "has a url, so `{key}` does nothing: this server                                  is not started here"
                            )));
                        }
                    }
                    mcp::Server::parse(&entry.name, url, transport)
                        .map(mcp::Entry::external)
                        .map_err(|e| named(e.to_string()))?
                }
                (None, Some(image)) => {
                    let managed = mcp::Managed {
                        image: image.clone(),
                        port: entry.port.ok_or_else(|| {
                            named("is managed but names no port to reach it on".into())
                        })?,
                        args: entry.args.clone().unwrap_or_default(),
                        env: entry.env.clone().unwrap_or_default(),
                        secrets: entry.secrets.clone().unwrap_or_default(),
                    };
                    mcp::Entry::managed(&entry.name, transport, managed)
                        .map_err(|e| named(e.to_string()))?
                }
            };
            // The agent keys its servers by name, so two entries sharing one
            // means the second silently replaces the first.
            if mcp.iter().any(|e| e.name() == resolved.name()) {
                return Err(invalid(
                    "mcp",
                    mcp::Error::DuplicateName(resolved.name().to_string()).to_string(),
                ));
            }
            // A managed server is published on its own port here, so two on one
            // port would leave the second unable to start.
            if let Some(port) = resolved.managed.as_ref().map(|m| m.port)
                && let Some(other) = mcp
                    .iter()
                    .find(|e| e.managed.as_ref().is_some_and(|m| m.port == port))
            {
                return Err(invalid(
                    "mcp",
                    format!(
                        "`{}` and `{}` both run on port {port}, and each is published on \
                         its own port on this machine; give one another port",
                        other.name(),
                        resolved.name()
                    ),
                ));
            }
            mcp.push(resolved);
        }

        let mut credentials = Vec::new();
        for (name, c) in raw.credentials.unwrap_or_default() {
            let named = |message: String| invalid("credentials", format!("`{name}`: {message}"));
            credentials.push(credential(&name, c).map_err(named)?);
        }

        if raw.sandbox_cpus == Some(0) {
            return Err(invalid(
                "sandbox_cpus",
                "a sandbox needs at least one CPU".into(),
            ));
        }
        let sandbox_memory = blank_to_none(raw.sandbox_memory);
        if let Some(m) = &sandbox_memory {
            let (digits, unit) = m.split_at(m.trim_end_matches(['m', 'g', 'M', 'G']).len());
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) || unit.len() > 1 {
                return Err(invalid(
                    "sandbox_memory",
                    format!("`{m}` is not an amount of memory; write it as `8g` or `512m`"),
                ));
            }
        }

        Ok(Config {
            path: path.to_path_buf(),
            present: true,
            gateway: raw.gateway,
            repo: raw.repo,
            base: raw.base,
            policy: raw.policy,
            providers: raw.providers,
            credentials,
            sandbox_cpus: raw.sandbox_cpus,
            sandbox_memory,
            sbx: raw.sbx.map(|p| expand_tilde(&p)),
            repo_roots: raw
                .repo_roots
                .map(|list| list.iter().map(|p| expand_tilde(p)).collect()),
            worktrees: raw.worktrees.unwrap_or(false),
            worktree_root: raw.worktree_root.as_deref().map(expand_tilde),
            refresh,
            auto_update: raw.auto_update,
            image_registry: raw.image_registry,
            skills: resolved_skills,
            mcp,
            branch_prefix: raw.branch_prefix,
            interface,
        })
    }

    /// What a new session's agent is when nothing else says: a chat. The
    /// terminal is still there for anyone who would rather have it, and is
    /// what every session made before the choice existed stays.
    pub fn interface(&self) -> chat::Interface {
        self.interface.unwrap_or(chat::Interface::Chat)
    }

    /// The policy a new session gets when nothing else says.
    pub fn policy(&self) -> &str {
        self.policy.as_deref().unwrap_or(policy::DEFAULT_TEMPLATE)
    }

    /// The providers a new session gets when nothing else says.
    pub fn providers(&self) -> &[String] {
        self.providers.as_deref().unwrap_or(&[])
    }

    /// Every credential a session may be given.
    pub fn credentials(&self) -> &[crate::credentials::Credential] {
        &self.credentials
    }

    pub fn credential(&self, name: &str) -> Option<&crate::credentials::Credential> {
        self.credentials.iter().find(|c| c.name == name)
    }

    /// Where a missing sandbox image is pulled from, or `None` to build it here.
    pub fn image_registry(&self) -> Option<&str> {
        match self.image_registry.as_deref().map(str::trim) {
            None => Some(crate::image::REGISTRY),
            Some("") => None,
            Some(registry) => Some(registry),
        }
    }

    /// What a work branch is named under.
    pub fn branch_prefix(&self) -> &str {
        self.branch_prefix
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or(session::DEFAULT_BRANCH_PREFIX)
    }

    /// The MCP servers a new session's agent is given.
    pub fn mcp(&self) -> &[mcp::Entry] {
        &self.mcp
    }

    /// Just the servers, which is what a session is given and what its record
    /// keeps. A managed entry's *definition* -- image, environment, the names
    /// of its secrets -- is the server's business and not a session's.
    pub fn mcp_servers(&self) -> Vec<mcp::Server> {
        self.mcp.iter().map(|e| e.server.clone()).collect()
    }

    /// The skills copied into a new session.
    pub fn skills(&self) -> &[skills::Skill] {
        &self.skills
    }
}

/// Exactly the file's shape, so serde rejects anything else.
///
/// Separate from [`Config`] because the two are not the same type: the file
/// holds `refresh = "2s"` and a config holds a [`Duration`], and putting the
/// validation in between is what lets the errors name the key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    gateway: Option<String>,
    repo: Option<String>,
    base: Option<String>,
    policy: Option<String>,
    providers: Option<Vec<String>>,
    repo_roots: Option<Vec<PathBuf>>,
    worktrees: Option<bool>,
    worktree_root: Option<PathBuf>,
    refresh: Option<String>,
    auto_update: Option<bool>,
    image_registry: Option<String>,
    skills: Option<Vec<String>>,
    /// `[[mcp]]` tables. An `Option` so `deny_unknown_fields` still rejects a
    /// misspelled `[[mcps]]` rather than reading it as none configured.
    mcp: Option<Vec<RawMcp>>,
    /// `[[tracker]]` tables, from when the server read the trackers. They are
    /// the desktop application's now, kept on the machine it runs on; still
    /// accepted here so an existing file keeps loading.
    #[allow(dead_code)]
    tracker: Option<serde::de::IgnoredAny>,
    branch_prefix: Option<String>,
    interface: Option<String>,
    /// `[credentials.NAME]` tables. Ordered by name so the create form lists
    /// them the same way every time.
    credentials: Option<std::collections::BTreeMap<String, RawCredential>>,
    sandbox_cpus: Option<u32>,
    sandbox_memory: Option<String>,
    sbx: Option<PathBuf>,
}

/// One `[credentials.NAME]` table. Every key any kind takes; [`credential`]
/// refuses the ones a kind does not, rather than ignoring them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCredential {
    kind: String,
    command: Option<String>,
    #[serde(rename = "ref")]
    reference: Option<String>,
    refresh: Option<String>,
    /// `nuget-feed`: the source's key in `NuGet.config`.
    feed: Option<String>,
    /// `nuget-feed`: where the feed is served from. `docker-registry`: where
    /// the registry's token service is, when it is not the registry itself.
    hosts: Option<Vec<String>>,
    /// `docker-registry`: the registry, as an image reference names it.
    registry: Option<String>,
    /// `docker-registry`: the user the registry takes the password with.
    username: Option<String>,
    /// `azure-cli`: the Azure CLI on this machine, when `az` is not enough.
    az: Option<String>,
    /// `azure-cli`: each Entra resource, and the hosts its token is sent to.
    resources: Option<std::collections::BTreeMap<String, Vec<String>>>,
}

/// Check one `[credentials.NAME]` table against what its kind takes.
fn credential(name: &str, c: RawCredential) -> Result<crate::credentials::Credential, String> {
    use crate::credentials::{Credential, Delivery, Kind, Resource, Source};

    let kind = Kind::parse(&c.kind).ok_or_else(|| {
        format!(
            "`{}` is not a kind of credential; expected one of {}",
            c.kind,
            Kind::ALL.map(|k| k.name()).join(", ")
        )
    })?;
    let refuse = |key: &str, present: bool| -> Result<(), String> {
        if present {
            Err(format!(
                "`{key}` is not something a {} credential takes",
                kind.name()
            ))
        } else {
            Ok(())
        }
    };
    let refresh = match blank_to_none(c.refresh) {
        None => None,
        Some(r) if runtime_duration(&r) => Some(r),
        Some(r) => {
            return Err(format!(
                "`refresh = \"{r}\"` is not a duration the sandbox runtime reads; write it \
                 like \"30m\" or \"1h\", or \"on-demand\""
            ));
        }
    };
    let source = || match (
        blank_to_none(c.command.clone()),
        blank_to_none(c.reference.clone()),
    ) {
        (Some(cmd), None) => Ok(Source::Command(cmd)),
        (None, Some(r)) => Ok(Source::Ref(r)),
        (Some(_), Some(_)) => Err("has both `command` and `ref`; say which".to_string()),
        (None, None) => {
            Err("says nowhere to get the value from; give it a `command` or a `ref`".to_string())
        }
    };

    if kind != Kind::DockerRegistry {
        refuse("registry", c.registry.is_some())?;
        refuse("username", c.username.is_some())?;
    }
    let delivery = match kind {
        Kind::ClaudeOauth | Kind::AzureDevOpsPat | Kind::GitHub => {
            refuse("feed", c.feed.is_some())?;
            refuse("hosts", c.hosts.is_some())?;
            refuse("az", c.az.is_some())?;
            refuse("resources", c.resources.is_some())?;
            Delivery::Value(source()?)
        }
        Kind::DockerRegistry => {
            refuse("feed", c.feed.is_some())?;
            refuse("az", c.az.is_some())?;
            refuse("resources", c.resources.is_some())?;
            let registry = blank_to_none(c.registry.clone()).ok_or(
                "needs `registry`, the registry as an image reference names it, such as \
                 \"myregistry.azurecr.io\"",
            )?;
            // What `docker pull` writes before the image's name: a host and
            // maybe a port, never a scheme or a path.
            let host = hosts_of(vec![registry.clone()])
                .map_err(|_| {
                    format!(
                        "`registry = \"{registry}\"` is not a registry; give it as an image \
                         reference starts, such as myregistry.azurecr.io"
                    )
                })?
                .remove(0);
            let username = blank_to_none(c.username.clone()).ok_or(
                "needs `username`, the user the registry takes the password with; for a \
                 token from `az acr login --expose-token` that is \
                 \"00000000-0000-0000-0000-000000000000\"",
            )?;
            if username.contains(':') {
                return Err(format!(
                    "`username = \"{username}\"` has a `:`, which ends a user in a Basic \
                     login, so the registry would read it as a shorter one"
                ));
            }
            let hosts = match c.hosts.clone() {
                Some(h) => hosts_of(h).map_err(|m| format!("`hosts`: {m}"))?,
                // The secret names hosts without a port.
                None => vec![
                    host.rsplit_once(':')
                        .map_or(host.as_str(), |(h, _)| h)
                        .to_string(),
                ],
            };
            if hosts.is_empty() {
                return Err(
                    "`hosts` is empty, so the login would be sent nowhere; leave it \
                     out to send it to the registry itself"
                        .into(),
                );
            }
            Delivery::Registry {
                source: source()?,
                registry: host,
                username,
                hosts,
            }
        }
        Kind::NugetFeed => {
            refuse("az", c.az.is_some())?;
            refuse("resources", c.resources.is_some())?;
            let feed = blank_to_none(c.feed.clone()).ok_or(
                "needs `feed`, the source's key in the repository's NuGet.config \
                 (`<add key=\"...\"`)",
            )?;
            // NuGet reads the credentials from a variable named after the key,
            // and a variable name cannot hold a dot, a dash or a space.
            if !feed
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            {
                return Err(format!(
                    "`feed = \"{feed}\"`: NuGet reads a feed's credentials from a variable \
                     named after its key, so the key can only be letters, digits and `_`"
                ));
            }
            let hosts = hosts_of(c.hosts.clone().unwrap_or_default())
                .map_err(|m| format!("`hosts`: {m}"))?;
            if hosts.is_empty() {
                return Err("needs `hosts`, where the feed is served from, such as \
                     [\"pkgs.dev.azure.com\"]"
                    .into());
            }
            Delivery::Feed {
                source: source()?,
                feed,
                hosts,
            }
        }
        Kind::AzureCli => {
            refuse("feed", c.feed.is_some())?;
            refuse("hosts", c.hosts.is_some())?;
            if c.command.is_some() || c.reference.is_some() {
                return Err(
                    "takes no `command` or `ref`: its tokens come from `az` on this \
                     machine, and `az = \"/path/to/az\"` says where that is"
                        .into(),
                );
            }
            let mut resources = Vec::new();
            for (resource, hosts) in c.resources.unwrap_or_default() {
                let resource = resource.trim().to_string();
                if resource.is_empty() {
                    return Err("`resources` has an empty resource name".into());
                }
                let hosts =
                    hosts_of(hosts).map_err(|m| format!("`resources.\"{resource}\"`: {m}"))?;
                if hosts.is_empty() {
                    return Err(format!(
                        "`resources.\"{resource}\"` names no hosts, so its token would be sent \
                         nowhere; list the hosts that take it"
                    ));
                }
                resources.push(Resource { resource, hosts });
            }
            if resources.is_empty() {
                return Err(
                    "needs `resources`: each Entra resource a session may get a token \
                     for, and the hosts it is sent to, such as \
                     \"https://vault.azure.net\" = [\"my-vault.vault.azure.net\"]"
                        .into(),
                );
            }
            Delivery::Azure {
                az: blank_to_none(c.az).unwrap_or_else(|| "az".to_string()),
                resources,
            }
        }
    };
    Ok(Credential {
        name: name.to_string(),
        kind,
        delivery,
        refresh,
    })
}

/// Hosts as a secret names them: a host or a glob, with no scheme or path.
fn hosts_of(hosts: Vec<String>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for h in hosts {
        let h = h.trim().to_string();
        if h.is_empty() {
            return Err("has an empty host".into());
        }
        if h.contains("://") || h.contains('/') || h.contains(char::is_whitespace) {
            return Err(format!(
                "`{h}` is not a host; give the name alone, such as example.com"
            ));
        }
        out.push(h);
    }
    Ok(out)
}

/// Whether the sandbox runtime reads `s` as a refresh: `on-demand`, or a
/// duration in its own notation, whole numbers each with `h`, `m` or `s`
/// (`30m`, `1h30m`).
fn runtime_duration(s: &str) -> bool {
    if s == "on-demand" {
        return true;
    }
    let mut digits = false;
    let mut any = false;
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            digits = true;
        } else if matches!(ch, 'h' | 'm' | 's') && digits {
            digits = false;
            any = true;
        } else {
            return false;
        }
    }
    any && !digits
}

/// One `[[mcp]]` table, before it is checked. Its own struct so a misspelled key
/// inside one is an error too, and so the message can name the entry.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcp {
    name: String,
    /// A server somebody else runs. Exclusive with `image`.
    url: Option<String>,
    transport: Option<String>,
    /// A server this one runs. The rest of the keys below belong to it, and are
    /// refused beside a `url` rather than ignored.
    image: Option<String>,
    port: Option<u16>,
    args: Option<Vec<String>>,
    env: Option<std::collections::BTreeMap<String, String>>,
    secrets: Option<Vec<String>>,
}

#[derive(Debug)]
pub enum Error {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        message: String,
    },
    Invalid {
        path: PathBuf,
        key: &'static str,
        message: String,
    },
    /// The file could not be written. Only [`crate::settings`] produces this
    /// one: reading a missing config is the normal case and not an error at
    /// all, but a settings screen that says it saved and did not is a lie.
    Write {
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Read { path, source } => {
                write!(f, "could not read {}: {source}", path.display())
            }
            Error::Parse { path, message } => {
                write!(f, "{}: {message}", path.display())
            }
            Error::Invalid { path, key, message } => {
                write!(f, "{}: `{key}` {message}", path.display())
            }
            Error::Write { path, source } => {
                write!(f, "could not write {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Read { source, .. } | Error::Write { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn names() -> String {
    policy::TEMPLATES
        .iter()
        .map(|t| t.name.to_string())
        .chain(policy::user_templates().into_iter().map(|t| t.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `""` means unset, not "a value that happens to be empty".
///
/// A key left as `policy = ""` while editing is far more likely to mean "I have
/// not decided" than "use a policy with no name", and the latter is only ever
/// an error two steps later.
fn blank_to_none(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}

/// `~` and `~/...`, which is how anyone writes a home-relative path in a config
/// file and which the shell is not around to expand here.
fn expand_tilde(path: &Path) -> PathBuf {
    let Some(rest) = path.to_str().and_then(|s| s.strip_prefix('~')) else {
        return path.to_path_buf();
    };
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return path.to_path_buf();
    };
    match rest.strip_prefix('/') {
        Some(tail) => home.join(tail),
        None if rest.is_empty() => home,
        // `~other/...` is another user's home, which is not ours to guess at.
        None => path.to_path_buf(),
    }
}

/// A whole number of milliseconds or seconds.
///
/// The unit is required. `refresh = 2` could plausibly mean either, and a config
/// that polls two thousand times too often is worse than one that refuses to
/// load.
fn parse_duration(raw: &str) -> Result<Duration, String> {
    let t = raw.trim();
    // `ms` first: every `ms` also ends in `s`.
    let (digits, per) = if let Some(n) = t.strip_suffix("ms") {
        (n, 1u64)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1000)
    } else {
        return Err(format!("`{raw}` has no unit; write it as `500ms` or `2s`"));
    };
    let n: u64 = digits
        .trim()
        .parse()
        .map_err(|_| format!("`{raw}` is not a whole number of milliseconds or seconds"))?;
    n.checked_mul(per)
        .map(Duration::from_millis)
        .ok_or_else(|| format!("`{raw}` is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Config, Error> {
        Config::parse(Path::new("/tmp/config.toml"), text)
    }

    #[test]
    fn an_empty_file_is_all_defaults() {
        let c = parse("").unwrap();
        assert_eq!(c.policy(), policy::DEFAULT_TEMPLATE);
        assert!(c.providers().is_empty());
        assert_eq!(c.refresh, None);
        assert!(c.present, "the file existed, it was just empty");
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let c = Config::load_from("/nonexistent/hura/config.toml").unwrap();
        assert!(!c.present);
        assert_eq!(c.policy(), policy::DEFAULT_TEMPLATE);
    }

    #[test]
    fn reads_every_key() {
        let c = parse(
            r#"
            gateway = "work"
            repo = "https://github.com/o/r"
            base = "develop"
            policy = "readonly-explore"
            providers = ["claude-oauth", "azure-pat"]
            repo_roots = ["/srv/code"]
            refresh = "2s"
            "#,
        )
        .unwrap();
        assert_eq!(c.gateway.as_deref(), Some("work"));
        assert_eq!(c.repo.as_deref(), Some("https://github.com/o/r"));
        assert_eq!(c.base.as_deref(), Some("develop"));
        assert_eq!(c.policy(), "readonly-explore");
        assert_eq!(c.providers(), ["claude-oauth", "azure-pat"]);
        assert_eq!(c.repo_roots.unwrap(), [PathBuf::from("/srv/code")]);
        assert_eq!(c.refresh, Some(Duration::from_secs(2)));
    }

    /// The whole point of `deny_unknown_fields`: a key that does nothing is
    /// indistinguishable from a key that is not working.
    #[test]
    fn an_unknown_key_is_an_error() {
        let e = parse("polciy = \"feature-work\"").unwrap_err();
        assert!(
            e.to_string().contains("polciy"),
            "the message should name the key: {e}"
        );
    }

    #[test]
    fn a_policy_that_is_not_a_template_is_an_error() {
        let e = parse(r#"policy = "feature-wrok""#).unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("feature-wrok"), "{msg}");
        assert!(
            msg.contains(policy::DEFAULT_TEMPLATE),
            "lists the real ones: {msg}"
        );
    }

    /// A path is not checked for existence here: it may be relative to wherever
    /// the command is run, and `policy::resolve` gives the better error.
    #[test]
    fn a_policy_path_is_taken_on_trust() {
        assert_eq!(
            parse(r#"policy = "./my.yaml""#).unwrap().policy(),
            "./my.yaml"
        );
        assert_eq!(
            parse(r#"policy = "/etc/hura/strict.yaml""#)
                .unwrap()
                .policy(),
            "/etc/hura/strict.yaml"
        );
    }

    #[test]
    fn refresh_needs_a_unit() {
        let e = parse(r#"refresh = "2""#).unwrap_err();
        assert!(e.to_string().contains("no unit"), "{e}");
        assert_eq!(
            parse(r#"refresh = "750ms""#).unwrap().refresh,
            Some(Duration::from_millis(750))
        );
    }

    #[test]
    fn refresh_is_bounded() {
        assert!(parse(r#"refresh = "1ms""#).is_err());
        assert!(
            parse(r#"refresh = "100ms""#).is_err(),
            "below the input tick"
        );
        assert!(
            parse(r#"refresh = "5m""#).is_err(),
            "minutes are not a unit here"
        );
        assert!(parse(r#"refresh = "120s""#).is_err());
        assert!(
            parse(r#"refresh = "250ms""#).is_ok(),
            "the floor is inclusive"
        );
        assert!(parse(r#"refresh = "60s""#).is_ok(), "so is the ceiling");
    }

    #[test]
    fn an_empty_list_is_an_error_rather_than_a_silent_nothing() {
        assert!(
            parse("providers = []")
                .unwrap_err()
                .to_string()
                .contains("providers")
        );
        assert!(
            parse("repo_roots = []")
                .unwrap_err()
                .to_string()
                .contains("repo_roots")
        );
    }

    #[test]
    fn a_blank_string_is_unset() {
        let c = parse(
            r#"policy = ""
                         gateway = "  ""#,
        )
        .unwrap();
        assert_eq!(c.policy(), policy::DEFAULT_TEMPLATE);
        assert_eq!(c.gateway, None);
    }

    #[test]
    fn tilde_is_expanded_against_home() {
        // SAFETY: single-threaded test process for this variable; the rest of
        // the suite reads HOME only through this helper.
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let Some(home) = home else { return };
        let c = parse(r#"repo_roots = ["~/dev", "~", "/abs", "~other/x"]"#).unwrap();
        assert_eq!(
            c.repo_roots.unwrap(),
            [
                home.join("dev"),
                home,
                PathBuf::from("/abs"),
                PathBuf::from("~other/x"),
            ]
        );
    }

    /// The shipped example has to parse, or `--init` writes a file that stops
    /// every later command from running.
    #[test]
    fn the_example_file_parses() {
        let c = parse(EXAMPLE).expect("the example must be valid");
        assert_eq!(
            c.policy(),
            policy::DEFAULT_TEMPLATE,
            "every key in the example is commented out, so it changes nothing"
        );
        assert_eq!(c.refresh, None);
    }

    /// Worktree sessions are off unless asked for, and the root is a path
    /// like the others, `~` and all.
    #[test]
    fn worktrees_are_off_until_the_file_turns_them_on() {
        assert!(!parse("").unwrap().worktrees);
        let c = parse("worktrees = true\nworktree_root = \"~/wt\"\n").unwrap();
        assert!(c.worktrees);
        assert!(!c.worktree_root.unwrap().starts_with("~"));
    }

    /// Every key the example mentions has to be a key the parser accepts, or the
    /// documentation and the code drift apart silently.
    #[test]
    fn the_example_documents_every_key() {
        for key in [
            "sbx",
            "sandbox_cpus",
            "sandbox_memory",
            "repo",
            "base",
            "policy",
            "providers",
            "repo_roots",
            "worktrees",
            "worktree_root",
            "refresh",
            "skills",
            "image_registry",
        ] {
            assert!(
                EXAMPLE.contains(&format!("# {key} =")),
                "the example does not show `{key}`"
            );
        }
        // A table rather than a key, so it is shown as one.
        assert!(
            EXAMPLE.contains("# [[mcp]]"),
            "the example does not show `[[mcp]]`"
        );
        assert!(
            EXAMPLE.contains("# [credentials."),
            "the example does not show `[credentials.NAME]`"
        );
    }

    /// The example's own tables, uncommented, are a config file that loads.
    #[test]
    fn the_example_s_credentials_load() {
        let text: String = EXAMPLE
            .lines()
            .skip_while(|l| !l.starts_with("# [credentials.claude]"))
            .take_while(|l| l.starts_with('#'))
            .map(|l| l.trim_start_matches('#').trim_start())
            .collect::<Vec<_>>()
            .join("\n");
        let c = parse(&text).unwrap();
        assert_eq!(c.credentials().len(), 5);
        assert_eq!(
            c.credential("azure-devops").unwrap().kind,
            crate::credentials::Kind::AzureDevOpsPat
        );
        assert_eq!(
            c.credential("azure").unwrap().kind,
            crate::credentials::Kind::AzureCli
        );
        assert_eq!(
            c.credential("feed").unwrap().kind,
            crate::credentials::Kind::NugetFeed
        );
        assert_eq!(
            c.credential("registry").unwrap().kind,
            crate::credentials::Kind::DockerRegistry
        );
    }

    #[test]
    fn a_registry_names_itself_and_a_user_and_sends_the_login_to_itself() {
        use crate::credentials::Delivery;
        let ok = r#"
            [credentials.acr]
            kind = "docker-registry"
            registry = "myregistry.azurecr.io"
            username = "00000000-0000-0000-0000-000000000000"
            command = "az acr login --name myregistry --expose-token --query accessToken -o tsv"
            refresh = "1h"
        "#;
        let c = parse(ok).unwrap();
        let acr = c.credential("acr").unwrap();
        let Delivery::Registry {
            registry,
            username,
            hosts,
            ..
        } = &acr.delivery
        else {
            panic!("{acr:?}");
        };
        assert_eq!(registry, "myregistry.azurecr.io");
        assert_eq!(username, "00000000-0000-0000-0000-000000000000");
        assert_eq!(hosts, &["myregistry.azurecr.io"]);
        assert_eq!(acr.refresh.as_deref(), Some("1h"));

        // A registry on a port is still logged in to by host, and a token
        // service elsewhere is named outright.
        let c = parse(
            &ok.replace("myregistry.azurecr.io\"", "registry.example.com:5000\"")
                .replace("refresh = \"1h\"", "hosts = [\"auth.example.com\"]"),
        )
        .unwrap();
        let Delivery::Registry {
            registry, hosts, ..
        } = &c.credential("acr").unwrap().delivery
        else {
            panic!();
        };
        assert_eq!(registry, "registry.example.com:5000");
        assert_eq!(hosts, &["auth.example.com"]);
        let c =
            parse(&ok.replace("myregistry.azurecr.io\"", "registry.example.com:5000\"")).unwrap();
        let Delivery::Registry { hosts, .. } = &c.credential("acr").unwrap().delivery else {
            panic!();
        };
        assert_eq!(hosts, &["registry.example.com"]);

        for (from, to, says) in [
            (
                "registry = \"myregistry.azurecr.io\"",
                "",
                "needs `registry`",
            ),
            (
                "myregistry.azurecr.io\"",
                "https://myregistry.azurecr.io\"",
                "is not a registry",
            ),
            (
                "username = \"00000000-0000-0000-0000-000000000000\"",
                "",
                "needs `username`",
            ),
            ("00000000-0000", "a:b", "has a `:`"),
            ("refresh = \"1h\"", "hosts = []", "sent nowhere"),
            (
                "refresh = \"1h\"",
                "feed = \"x\"",
                "`feed` is not something",
            ),
        ] {
            let e = parse(&ok.replace(from, to)).unwrap_err().to_string();
            assert!(e.contains(says), "{from} -> {to}: {e}");
        }

        let e = parse(
            "[credentials.gh]\nkind = \"github\"\ncommand = \"gh auth token\"\nusername = \"me\"\n",
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("`username` is not something a github credential takes"),
            "{e}"
        );
    }

    #[test]
    fn an_azure_cli_credential_names_its_resources_and_their_hosts() {
        use crate::credentials::{Delivery, Resource};
        let c = parse(
            r#"
            [credentials.azure]
            kind = "azure-cli"
            [credentials.azure.resources]
            "https://vault.azure.net" = ["kv.vault.azure.net", "other.vault.azure.net"]
            "https://storage.azure.com" = ["acct.blob.core.windows.net"]
            "#,
        )
        .unwrap();
        let azure = c.credential("azure").unwrap();
        assert_eq!(azure.refresh, None, "left to the grant's own default");
        let Delivery::Azure { az, resources } = &azure.delivery else {
            panic!("{azure:?}");
        };
        assert_eq!(az, "az");
        assert_eq!(
            resources[1],
            Resource {
                resource: "https://vault.azure.net".into(),
                hosts: vec!["kv.vault.azure.net".into(), "other.vault.azure.net".into()],
            },
            "ordered by resource, like every table here"
        );
    }

    #[test]
    fn an_azure_cli_credential_refuses_what_it_cannot_use() {
        let e = parse("[credentials.a]\nkind = \"azure-cli\"\n")
            .unwrap_err()
            .to_string();
        assert!(e.contains("needs `resources`"), "{e}");

        let e = parse(
            "[credentials.a]\nkind = \"azure-cli\"\ncommand = \"az\"\n\
             [credentials.a.resources]\n\"https://vault.azure.net\" = [\"kv.vault.azure.net\"]\n",
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("takes no `command`"), "{e}");

        let e = parse(
            "[credentials.a]\nkind = \"azure-cli\"\n\
             [credentials.a.resources]\n\"https://vault.azure.net\" = []\n",
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("names no hosts"), "{e}");

        let e = parse(
            "[credentials.a]\nkind = \"azure-cli\"\n\
             [credentials.a.resources]\n\"https://vault.azure.net\" = [\"https://kv.vault.azure.net/\"]\n",
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("is not a host"), "{e}");
    }

    #[test]
    fn a_nuget_feed_needs_a_key_nuget_can_name_and_its_hosts() {
        let ok = r#"
            [credentials.feed]
            kind = "nuget-feed"
            feed = "MyPackages"
            hosts = ["pkgs.dev.azure.com"]
            command = "cat /tokens/feed"
            refresh = "30m"
        "#;
        let c = parse(ok).unwrap();
        assert_eq!(
            c.credential("feed").unwrap().refresh.as_deref(),
            Some("30m")
        );

        let e = parse(&ok.replace("MyPackages", "my-packages"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("letters, digits and `_`"), "{e}");
        let e = parse(&ok.replace("hosts = [\"pkgs.dev.azure.com\"]", ""))
            .unwrap_err()
            .to_string();
        assert!(e.contains("needs `hosts`"), "{e}");
        let e = parse(&ok.replace("command = \"cat /tokens/feed\"", ""))
            .unwrap_err()
            .to_string();
        assert!(e.contains("give it a `command` or a `ref`"), "{e}");
    }

    /// A key that belongs to another kind is named back, not ignored: a
    /// `hosts` on a GitHub token would otherwise look like it did something.
    #[test]
    fn a_credential_refuses_keys_its_kind_does_not_take() {
        let e = parse(
            "[credentials.gh]\nkind = \"github\"\ncommand = \"gh auth token\"\nhosts = [\"x.com\"]\n",
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("`hosts` is not something a github credential takes"),
            "{e}"
        );
    }

    #[test]
    fn refresh_is_a_duration_the_runtime_reads() {
        for good in ["30m", "1h", "1h30m", "90s", "on-demand"] {
            assert!(runtime_duration(good), "{good}");
        }
        for bad in ["", "30", "m", "30x", "1.5h", "-1h", "h30"] {
            assert!(!runtime_duration(bad), "{bad}");
        }
        let e = parse("[credentials.c]\nkind = \"github\"\ncommand = \"x\"\nrefresh = \"soon\"\n")
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("not a duration the sandbox runtime reads"),
            "{e}"
        );
    }

    /// A file written before sessions moved to Docker Sandboxes still loads.
    #[test]
    fn a_retired_gateway_still_parses() {
        parse("gateway = \"default\"\n").expect("a retired key is accepted and ignored");
    }

    #[test]
    fn skills_take_a_name_or_a_path() {
        let c = parse(r#"skills = ["ship-pr", "~/dev/hura/.claude/skills/audit"]"#).unwrap();
        assert_eq!(c.skills().len(), 2);
        assert_eq!(c.skills()[0].name, "ship-pr");
        assert_eq!(
            c.skills()[0].source,
            crate::skills::host_skills_dir().join("ship-pr"),
            "a bare name is one of your own"
        );
        assert_eq!(c.skills()[1].name, "audit");
    }

    /// A skill directory that is not there is not a config error: it may be a
    /// repository that is not cloned yet, and every command refusing to run over
    /// it would be worse than the session missing the skill.
    #[test]
    fn a_skill_that_does_not_exist_still_loads() {
        let c = parse(r#"skills = ["/nope/not/here"]"#).unwrap();
        assert_eq!(c.skills()[0].name, "here");
        assert!(c.skills()[0].problem().is_some(), "doctor's to report");
    }

    #[test]
    fn two_skills_cannot_share_a_directory_name() {
        let e = parse(r#"skills = ["/a/ship-pr", "/b/ship-pr"]"#).unwrap_err();
        assert!(e.to_string().contains("two skills"), "{e}");
    }

    #[test]
    fn an_empty_skills_list_is_an_error() {
        assert!(
            parse("skills = []")
                .unwrap_err()
                .to_string()
                .contains("skills")
        );
    }

    #[test]
    fn mcp_servers_are_read_and_defaulted() {
        let c = parse(
            r#"
            [[mcp]]
            name = "jira"
            url = "http://mcp-atlassian:9000/mcp"

            [[mcp]]
            name = "azure-devops"
            url = "http://mcp-azure:9001/sse"
            transport = "sse"
            "#,
        )
        .unwrap();
        assert_eq!(c.mcp().len(), 2);
        assert_eq!(c.mcp()[0].server.endpoint, "mcp-atlassian:9000");
        assert_eq!(
            c.mcp()[0].server.transport,
            mcp::Transport::Http,
            "http is the transport a server has unless it says otherwise"
        );
        assert_eq!(c.mcp()[1].server.transport, mcp::Transport::Sse);
        assert!(
            c.mcp().iter().all(|e| !e.is_managed()),
            "a url is a server somebody else runs"
        );
    }

    /// The managed shape: an image instead of a url, and the url derived from
    /// the container this server will start. Deriving it is what removes the two
    /// mistakes a hand-written url invites -- a name no sandbox resolves, and a
    /// `localhost` that means the sandbox itself.
    #[test]
    fn a_managed_entry_is_described_rather_than_started_by_hand() {
        let c = parse(
            r#"
            [[mcp]]
            name = "sentry"
            image = "ghcr.io/example/mcp-sentry:1.4"
            port = 9000
            args = ["--transport", "http"]
            secrets = ["SENTRY_TOKEN"]
            env = { SENTRY_ORG = "contoso" }
            "#,
        )
        .unwrap();
        let e = &c.mcp()[0];
        assert!(e.is_managed());
        assert_eq!(e.server.url, "http://host.docker.internal:9000/mcp");
        assert_eq!(e.server.endpoint, "host.docker.internal:9000");
        let m = e.managed.as_ref().unwrap();
        assert_eq!(m.image, "ghcr.io/example/mcp-sentry:1.4");
        assert_eq!(m.secrets, ["SENTRY_TOKEN"]);
        assert_eq!(m.env.get("SENTRY_ORG").unwrap(), "contoso");
        // What a session records is the server, not the recipe: the image and
        // the secret names are the server's business.
        assert_eq!(c.mcp_servers()[0].name, "sentry");
    }

    /// Each of these is a table that would half-work: a key that does nothing,
    /// or a url pointing somewhere other than the container beside it.
    #[test]
    fn an_mcp_table_names_one_shape_or_the_other() {
        let cases = [
            (
                r#"[[mcp]]
                   name = "x"
                   url = "http://a:9000/mcp"
                   image = "an/image""#,
                "both a url and an image",
            ),
            (
                r#"[[mcp]]
                   name = "x""#,
                "neither a url nor an image",
            ),
            (
                r#"[[mcp]]
                   name = "x"
                   image = "an/image""#,
                "names no port",
            ),
            (
                r#"[[mcp]]
                   name = "x"
                   url = "http://a:9000/mcp"
                   secrets = ["TOKEN"]"#,
                "`secrets` does nothing",
            ),
        ];
        for (text, expected) in cases {
            let err = parse(text).expect_err(text).to_string();
            assert!(err.contains(expected), "expected `{expected}` in: {err}");
        }
    }

    #[test]
    fn no_mcp_table_is_no_servers() {
        assert!(parse("").unwrap().mcp().is_empty());
    }

    /// The mistake this validation exists for: correct on the host, wrong in the
    /// sandbox, and invisible until the agent is running.
    #[test]
    fn a_loopback_mcp_url_is_refused_by_name() {
        let e = parse(
            r#"
            [[mcp]]
            name = "jira"
            url = "http://localhost:9000/mcp"
            "#,
        )
        .unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("jira"), "names the entry: {msg}");
        assert!(
            msg.contains("host.docker.internal"),
            "says what to use: {msg}"
        );
    }

    #[test]
    fn a_misspelled_mcp_key_is_an_error() {
        let e = parse(
            r#"
            [[mcp]]
            name = "jira"
            urls = "http://mcp:9000/mcp"
            "#,
        )
        .unwrap_err();
        assert!(e.to_string().contains("urls"), "{e}");
    }

    /// The agent keys its servers by name, so a duplicate is one server
    /// silently replacing another.
    #[test]
    fn two_mcp_servers_cannot_share_a_name() {
        let e = parse(
            r#"
            [[mcp]]
            name = "jira"
            url = "http://a:9000/mcp"

            [[mcp]]
            name = "jira"
            url = "http://b:9000/mcp"
            "#,
        )
        .unwrap_err();
        assert!(e.to_string().contains("already the name"), "{e}");
    }

    #[test]
    fn two_managed_servers_cannot_share_a_port() {
        let e = parse(
            r#"
            [[mcp]]
            name = "jira"
            image = "ghcr.io/example/mcp-jira:1"
            port = 9000

            [[mcp]]
            name = "sentry"
            image = "ghcr.io/example/mcp-sentry:1"
            port = 9000
            "#,
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("jira") && e.contains("sentry") && e.contains("9000"),
            "{e}"
        );
    }
}
