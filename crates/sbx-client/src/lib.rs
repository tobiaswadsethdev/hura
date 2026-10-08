//! A typed client for `sbx`, the Docker Sandboxes CLI.
//!
//! Everything the rest of hura knows about Docker Sandboxes goes through the
//! [`Sbx`] trait, so a change in the CLI's surface lands in this one file.
//!
//! Measured against v0.47.0 rather than read off its documentation:
//!
//! * every `--json` listing prints one JSON document on stdout and exits 0
//! * a missing sandbox exits 1 with `error: sandbox 'NAME' not found` on
//!   stderr, except `policy ls NAME`, which answers with the global rules
//!   instead, so nothing here relies on that one to say a sandbox is gone
//! * `exec` passes the remote exit code through and keeps the two streams
//!   apart, and starts the sandbox first when it is stopped
//! * a sandbox stops itself shortly after its last session disconnects
//!   unless it is switched to detached mode with `run -d`
//! * `policy allow` and `policy deny` have no JSON output, and say which rule
//!   they made as `Rule added to policy local (scope: sandbox:NAME): ID (...)`

use std::ffi::OsStr;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not run `{bin}`: {source}")]
    Spawn {
        bin: String,
        #[source]
        source: std::io::Error,
    },

    #[error("sandbox `{0}` not found")]
    NotFound(String),

    #[error("`sbx {args}` failed (exit {code}): {stderr}")]
    Cli {
        args: String,
        code: i32,
        stderr: String,
    },

    #[error("could not parse `sbx {args}` output as JSON: {source}")]
    Parse {
        args: String,
        #[source]
        source: serde_json::Error,
    },

    /// The CLI succeeded and said something this build cannot read, such as a
    /// rule id missing from the line that should carry one.
    #[error("`sbx {args}` answered in a shape this build does not know: {said}")]
    Unexpected { args: String, said: String },
}

/// Where a sandbox is in its life, as `sbx ls` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Running,
    Stopped,
    /// Anything this build does not name. Kept rather than refused, so a new
    /// status in a newer `sbx` shows up as itself instead of failing a listing.
    Other(String),
}

impl From<&str> for Status {
    fn from(s: &str) -> Self {
        match s {
            "running" => Status::Running,
            "stopped" => Status::Stopped,
            other => Status::Other(other.to_string()),
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::Running => f.write_str("running"),
            Status::Stopped => f.write_str("stopped"),
            Status::Other(s) => f.write_str(s),
        }
    }
}

/// One sandbox, from `sbx ls --json`.
#[derive(Debug, Clone)]
pub struct Sandbox {
    pub name: String,
    pub id: String,
    pub status: Status,
    pub created_at: Option<String>,
}

#[derive(Deserialize)]
struct RawSandbox {
    name: String,
    #[serde(default)]
    id: String,
    status: String,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Deserialize)]
struct Listing {
    #[serde(default)]
    sandboxes: Vec<RawSandbox>,
}

impl From<RawSandbox> for Sandbox {
    fn from(r: RawSandbox) -> Self {
        Sandbox {
            status: Status::from(r.status.as_str()),
            name: r.name,
            id: r.id,
            created_at: r.created_at,
        }
    }
}

/// What a command run inside a sandbox left behind.
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
}

/// How to make a sandbox.
#[derive(Debug, Clone, Default)]
pub struct CreateOpts {
    pub name: String,
    /// The image, by the tag it was loaded into the sandbox runtime under.
    pub template: String,
    /// Whole CPUs. `None` leaves the CLI's default, which is every host CPU.
    pub cpus: Option<u32>,
    /// In the CLI's own units, such as `8g`. `None` leaves its default, which
    /// is half the host's memory.
    pub memory: Option<String>,
}

impl CreateOpts {
    fn args(&self) -> Vec<String> {
        // `shell` is the agent that starts nothing of its own, which is what a
        // sandbox hura runs its own agent in wants. `--skills off` keeps the
        // runtime's shared skills store away from the agent's skills
        // directory, which hura fills itself. `--pull never` because the
        // template is one hura loaded, and a registry has never heard of it.
        let mut out: Vec<String> = [
            "create",
            "shell",
            "--name",
            &self.name,
            "--template",
            &self.template,
            "--pull",
            "never",
            "--skills",
            "off",
            "--quiet",
        ]
        .map(String::from)
        .to_vec();
        if let Some(c) = self.cpus {
            out.push("--cpus".into());
            out.push(c.to_string());
        }
        if let Some(m) = &self.memory {
            out.push("--memory".into());
            out.push(m.clone());
        }
        out
    }
}

/// Allow or deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Deny,
}

impl Decision {
    fn verb(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Deny => "deny",
        }
    }
}

/// A network rule to add to one sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSpec {
    pub decision: Decision,
    /// Hosts, globs, IPs or CIDRs, each with an optional `:port`, as the CLI
    /// takes them. Several share one rule.
    pub resources: Vec<String>,
    /// HTTP methods, for a rule that inspects requests. Empty for a rule about
    /// the connection alone.
    pub methods: Vec<String>,
    /// The path glob an HTTP rule is narrowed to. `None` is every path.
    pub path: Option<String>,
}

impl RuleSpec {
    fn args(&self, sandbox: &str) -> Vec<String> {
        let mut out: Vec<String> = [
            "policy",
            self.decision.verb(),
            "network",
            "--sandbox",
            sandbox,
        ]
        .map(String::from)
        .to_vec();
        if !self.methods.is_empty() {
            out.push("--method".into());
            out.push(self.methods.join(","));
        }
        if let Some(p) = &self.path {
            out.push("--path".into());
            out.push(p.clone());
        }
        out.push(self.resources.join(","));
        out
    }
}

/// One rule, from `sbx policy ls NAME --wide --json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub id: String,
    /// `global`, or `sandbox:NAME` for a rule that is about one sandbox.
    #[serde(default)]
    pub scope: String,
    /// `network`, `http`, `filesystem:read` and so on.
    #[serde(default)]
    pub resource_type: String,
    pub decision: Decision,
    #[serde(default)]
    pub resources: Vec<String>,
    /// `local` for a rule on this machine, `scoped` for one made for a single
    /// sandbox, `org` or `kit` for the other two sources.
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub editable: bool,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub http_targets: Vec<HttpTarget>,
}

impl Rule {
    /// A rule made for one sandbox, as opposed to one every sandbox has.
    pub fn is_scoped(&self) -> bool {
        self.scope.starts_with("sandbox:")
    }

    pub fn is_network(&self) -> bool {
        self.resource_type == "network" || self.resource_type == "http"
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct HttpTarget {
    pub host: String,
    pub path: String,
}

#[derive(Deserialize)]
struct RuleListing {
    #[serde(default)]
    rules: Vec<Rule>,
}

/// One line of `sbx policy log NAME --json`: every decision about one host
/// under one rule since the daemon started, counted rather than listed.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct LogEntry {
    pub host: String,
    #[serde(default)]
    pub vm_name: String,
    /// `forward` for a client that used the proxy, `transparent` for one that
    /// did not, `network` for a DNS lookup.
    #[serde(default)]
    pub proxy_type: String,
    /// The rule that decided, or for a refusal the operation nobody allowed,
    /// which for an HTTP request names its method and path.
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub since: String,
    #[serde(default)]
    pub last_seen: String,
    #[serde(default)]
    pub count_since: u64,
    #[serde(default)]
    pub reason: Option<String>,
}

/// The two halves of `sbx policy log NAME --json`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PolicyLog {
    #[serde(default)]
    pub blocked_hosts: Vec<LogEntry>,
    #[serde(default)]
    pub allowed_hosts: Vec<LogEntry>,
}

/// A published port, from `sbx ports NAME --json`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Port {
    pub host_ip: String,
    pub host_port: u16,
    pub sandbox_port: u16,
    pub protocol: String,
}

/// A custom secret, from `sbx secret ls --json`. The value is masked.
#[derive(Debug, Clone, Deserialize)]
pub struct CustomSecret {
    /// `global`, or the name of the one sandbox it is for.
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub env: String,
    pub placeholder: String,
}

#[derive(Deserialize)]
struct SecretListing {
    #[serde(default)]
    custom_secrets: Vec<CustomSecret>,
}

/// Where a secret's value comes from. Never the value itself: the runtime
/// resolves either of these on the host, so hura never holds a credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    /// A host command whose stdout is the value.
    Command(String),
    /// A 1Password `op://` reference or an AWS Secrets Manager ARN.
    Ref(String),
}

/// A custom secret for one sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretSpec {
    pub sandbox: String,
    /// The hosts whose request headers get the value in place of the
    /// placeholder.
    pub hosts: Vec<String>,
    /// The variable the placeholder is put in, for sandboxes made after it.
    pub env: String,
    pub source: SecretSource,
    /// The placeholder's shape, with `{rand}` for its random part. `None`
    /// leaves the runtime's own, `sbx-cs-` and twenty characters.
    pub placeholder: Option<String>,
}

impl SecretSpec {
    fn args(&self) -> Vec<String> {
        let mut out: Vec<String> = [
            "secret",
            "set-custom",
            "--sandbox",
            &self.sandbox,
            "--env",
            &self.env,
        ]
        .map(String::from)
        .to_vec();
        for h in &self.hosts {
            out.push("--host".into());
            out.push(h.clone());
        }
        match &self.source {
            SecretSource::Command(c) => {
                out.push("--command".into());
                out.push(c.clone());
            }
            SecretSource::Ref(r) => {
                out.push("--ref".into());
                out.push(r.clone());
            }
        }
        if let Some(p) = &self.placeholder {
            out.push("--placeholder".into());
            out.push(p.clone());
        }
        out
    }
}

/// An image in the sandbox runtime's own store, from `sbx template ls --json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Template {
    /// The image id, shortened to twelve characters, which is the prefix of
    /// the id Docker reports for the same image.
    pub id: String,
    pub repository: String,
    pub tag: String,
    #[serde(default)]
    pub flavor: Option<String>,
}

#[derive(Deserialize)]
struct TemplateListing {
    #[serde(default)]
    images: Vec<Template>,
}

/// `sbx version --json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Version {
    pub client: ClientVersion,
    #[serde(default)]
    pub server: Option<ServerVersion>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientVersion {
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerVersion {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub version: String,
}

/// Everything hura asks of Docker Sandboxes.
pub trait Sbx {
    fn version(&self) -> Result<Version>;

    /// Make a sandbox. It is running when this returns.
    fn create(&self, opts: &CreateOpts) -> Result<()>;

    /// Keep a sandbox running after its sessions end, permanently.
    ///
    /// Without this a sandbox stops itself thirty seconds after the last
    /// `exec` disconnects, which takes an agent running in tmux with it.
    fn detach(&self, name: &str) -> Result<()>;

    fn list(&self) -> Result<Vec<Sandbox>>;

    /// Run a command with no stdin. Starts the sandbox if it is stopped.
    fn exec(&self, name: &str, argv: &[&str]) -> Result<ExecOutput>;

    /// Run a command with `input` on its stdin, for payloads too big for an
    /// argument.
    fn exec_stdin(&self, name: &str, argv: &[&str], input: &[u8]) -> Result<ExecOutput>;

    /// The argv for an interactive exec that owns a terminal. Built, not run.
    fn interactive_argv(&self, name: &str, argv: &[&str]) -> Vec<String>;

    fn remove(&self, name: &str) -> Result<()>;

    /// Every rule that applies to the sandbox, global ones included.
    fn rules(&self, name: &str) -> Result<Vec<Rule>>;

    /// Add a rule to one sandbox, returning its id.
    fn add_rule(&self, sandbox: &str, rule: &RuleSpec) -> Result<String>;

    fn remove_rule(&self, sandbox: &str, id: &str) -> Result<()>;

    fn log(&self, name: &str) -> Result<PolicyLog>;

    fn ports(&self, name: &str) -> Result<Vec<Port>>;

    /// Publish a port inside the sandbox on an ephemeral loopback port here.
    fn publish(&self, name: &str, sandbox_port: u16) -> Result<Port>;

    fn unpublish(&self, name: &str, port: &Port) -> Result<()>;

    fn secrets(&self) -> Result<Vec<CustomSecret>>;

    /// Make a sandbox-scoped secret, returning its placeholder.
    fn add_secret(&self, spec: &SecretSpec) -> Result<String>;

    fn remove_secret(&self, placeholder: &str) -> Result<()>;

    fn templates(&self) -> Result<Vec<Template>>;

    /// Load a `docker save` archive into the runtime's image store.
    fn load_template(&self, archive: &Path) -> Result<()>;
}

/// [`Sbx`] backed by the CLI.
#[derive(Debug, Clone)]
pub struct CliClient {
    bin: PathBuf,
}

impl Default for CliClient {
    fn default() -> Self {
        CliClient { bin: locate() }
    }
}

/// Where the CLI is when nobody has said.
///
/// `PATH` first, then where its own installer puts it. The second matters more
/// than it looks: a systemd user service starts with a `PATH` of
/// `/usr/local/bin:/usr/bin`, and the tarball installs to `~/.docker/sbx/bin`.
pub fn locate() -> PathBuf {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("sbx");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = Path::new(&home).join(".docker/sbx/bin/sbx");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("sbx")
}

/// The variables the CLI reads its state directories from.
///
/// Unset on every call, so the CLI always finds the user's own store and
/// daemon. An `hurad` run with these pointed somewhere private, which is how a
/// development server is kept apart from the real one, would otherwise start a
/// second daemon with an empty store and no sign-in.
const STATE_VARS: [&str; 4] = [
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
];

impl CliClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_bin(mut self, bin: impl Into<PathBuf>) -> Self {
        self.bin = bin.into();
        self
    }

    pub fn bin(&self) -> &Path {
        &self.bin
    }

    fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new(&self.bin);
        for var in STATE_VARS {
            cmd.env_remove(var);
        }
        cmd.args(args);
        cmd
    }

    /// Run the CLI and capture both streams without judging the exit code,
    /// because `exec` legitimately returns non-zero.
    fn run<I, S>(&self, args: I, input: Option<&[u8]>) -> Result<ExecOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = self.command(args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        // Never the caller's stdin: an `exec` left holding a terminal or a pipe
        // waits on it, and a background caller then never sees it return.
        cmd.stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let spawn = |source| Error::Spawn {
            bin: self.bin.display().to_string(),
            source,
        };
        let mut child = cmd.spawn().map_err(spawn)?;
        if let Some(bytes) = input {
            let mut stdin = child.stdin.take().expect("stdin was piped");
            // A command that exits without reading its input closes the pipe,
            // and what it printed is the answer worth returning, so a broken
            // pipe here is not an error.
            let _ = stdin.write_all(bytes);
        }
        let out = child.wait_with_output().map_err(spawn)?;
        Ok(ExecOutput {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            // A signalled process reports no code; -1 keeps this infallible
            // and is never a real exit status.
            exit_code: out.status.code().unwrap_or(-1),
        })
    }

    /// Run the CLI and require success.
    ///
    /// `subject` is the sandbox the call is about, so a refusal that says it
    /// is missing becomes [`Error::NotFound`] rather than a generic failure.
    fn run_checked(&self, args: &[String], subject: Option<&str>) -> Result<ExecOutput> {
        let out = self.run(args, None)?;
        if out.ok() {
            return Ok(out);
        }
        Err(refusal(args, subject, &out))
    }

    fn json<T: serde::de::DeserializeOwned>(
        &self,
        args: &[String],
        subject: Option<&str>,
    ) -> Result<T> {
        let out = self.run_checked(args, subject)?;
        serde_json::from_str(&out.stdout).map_err(|source| Error::Parse {
            args: args.join(" "),
            source,
        })
    }
}

/// What a failed call means, from what it said.
fn refusal(args: &[String], subject: Option<&str>, out: &ExecOutput) -> Error {
    let said = out
        .stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("try:"))
        .map(|l| l.strip_prefix("error: ").unwrap_or(l))
        .collect::<Vec<_>>()
        .join(" ");
    // Matched on the sandbox's own name in the message, not on `not found`
    // alone, which is also what the CLI says about a template or a secret. A
    // missing template read as a missing sandbox would report a failed create
    // as a sandbox already gone.
    if let Some(name) = subject
        && said.contains(&format!("sandbox '{name}' not found"))
    {
        return Error::NotFound(name.to_string());
    }
    Error::Cli {
        args: args.join(" "),
        code: out.exit_code,
        stderr: said,
    }
}

/// The id out of `Rule added to policy local (scope: sandbox:NAME): ID (...)`.
fn rule_id(said: &str) -> Option<String> {
    let line = said.lines().find(|l| l.starts_with("Rule added"))?;
    let (_, rest) = line.split_once("): ")?;
    let id = rest.split_whitespace().next()?;
    Some(id.to_string())
}

/// The placeholder out of `Saved custom secret placeholder "PH" for ...`.
fn placeholder(said: &str) -> Option<String> {
    let line = said
        .lines()
        .find(|l| l.starts_with("Saved custom secret placeholder"))?;
    let start = line.find('"')? + 1;
    let len = line[start..].find('"')?;
    Some(line[start..start + len].to_string())
}

/// The host port out of `Published 127.0.0.1:44620 -> 8080/tcp4`.
fn published(said: &str, sandbox_port: u16) -> Option<Port> {
    let line = said.lines().find(|l| l.starts_with("Published "))?;
    let mut words = line.split_whitespace().skip(1);
    let (ip, host_port) = words.next()?.rsplit_once(':')?;
    words.next()?;
    let (inside, protocol) = words.next()?.split_once('/')?;
    if inside.parse::<u16>().ok()? != sandbox_port {
        return None;
    }
    Some(Port {
        host_ip: ip.trim_matches(['[', ']']).to_string(),
        host_port: host_port.parse().ok()?,
        sandbox_port,
        protocol: protocol.to_string(),
    })
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_string()).collect()
}

impl Sbx for CliClient {
    fn version(&self) -> Result<Version> {
        self.json(&strings(&["version", "--json"]), None)
    }

    fn create(&self, opts: &CreateOpts) -> Result<()> {
        self.run_checked(&opts.args(), None)?;
        Ok(())
    }

    fn detach(&self, name: &str) -> Result<()> {
        self.run_checked(&strings(&["run", "--detached", "--name", name]), Some(name))?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<Sandbox>> {
        let listing: Listing = self.json(&strings(&["ls", "--json"]), None)?;
        Ok(listing.sandboxes.into_iter().map(Sandbox::from).collect())
    }

    fn exec(&self, name: &str, argv: &[&str]) -> Result<ExecOutput> {
        let mut args = strings(&["exec", name]);
        args.extend(argv.iter().map(|a| (*a).to_string()));
        let out = self.run(&args, None)?;
        // The command's own failures come back as its exit code; only the
        // CLI's refusal to run it at all is an error here.
        if !out.ok()
            && out.stdout.is_empty()
            && out.stderr.contains(&format!("sandbox '{name}' not found"))
        {
            return Err(Error::NotFound(name.to_string()));
        }
        Ok(out)
    }

    fn exec_stdin(&self, name: &str, argv: &[&str], input: &[u8]) -> Result<ExecOutput> {
        let mut args = strings(&["exec", "--interactive", name]);
        args.extend(argv.iter().map(|a| (*a).to_string()));
        let out = self.run(&args, Some(input))?;
        if !out.ok()
            && out.stdout.is_empty()
            && out.stderr.contains(&format!("sandbox '{name}' not found"))
        {
            return Err(Error::NotFound(name.to_string()));
        }
        Ok(out)
    }

    fn interactive_argv(&self, name: &str, argv: &[&str]) -> Vec<String> {
        let mut out = vec![self.bin.display().to_string()];
        out.extend(["exec", "--interactive", "--tty", name].map(String::from));
        out.extend(argv.iter().map(|a| (*a).to_string()));
        out
    }

    fn remove(&self, name: &str) -> Result<()> {
        self.run_checked(&strings(&["rm", "--force", name]), Some(name))?;
        Ok(())
    }

    fn rules(&self, name: &str) -> Result<Vec<Rule>> {
        let listing: RuleListing = self.json(
            &strings(&["policy", "ls", name, "--wide", "--json"]),
            Some(name),
        )?;
        Ok(listing.rules)
    }

    fn add_rule(&self, sandbox: &str, rule: &RuleSpec) -> Result<String> {
        let args = rule.args(sandbox);
        let out = self.run_checked(&args, Some(sandbox))?;
        rule_id(&out.stdout).ok_or_else(|| Error::Unexpected {
            args: args.join(" "),
            said: out.stdout.trim().to_string(),
        })
    }

    fn remove_rule(&self, sandbox: &str, id: &str) -> Result<()> {
        self.run_checked(
            &strings(&[
                "policy",
                "rm",
                "network",
                "--sandbox",
                sandbox,
                "--id",
                id,
                "--force",
            ]),
            Some(sandbox),
        )?;
        Ok(())
    }

    fn log(&self, name: &str) -> Result<PolicyLog> {
        self.json(&strings(&["policy", "log", name, "--json"]), Some(name))
    }

    fn ports(&self, name: &str) -> Result<Vec<Port>> {
        self.json(&strings(&["ports", name, "--json"]), Some(name))
    }

    fn publish(&self, name: &str, sandbox_port: u16) -> Result<Port> {
        let args = strings(&["ports", name, "--publish", &sandbox_port.to_string()]);
        let out = self.run_checked(&args, Some(name))?;
        published(&out.stdout, sandbox_port).ok_or_else(|| Error::Unexpected {
            args: args.join(" "),
            said: out.stdout.trim().to_string(),
        })
    }

    fn unpublish(&self, name: &str, port: &Port) -> Result<()> {
        let spec = format!(
            "{}:{}:{}/{}",
            port.host_ip, port.host_port, port.sandbox_port, port.protocol
        );
        self.run_checked(&strings(&["ports", name, "--unpublish", &spec]), Some(name))?;
        Ok(())
    }

    fn secrets(&self) -> Result<Vec<CustomSecret>> {
        let listing: SecretListing = self.json(&strings(&["secret", "ls", "--json"]), None)?;
        Ok(listing.custom_secrets)
    }

    fn add_secret(&self, spec: &SecretSpec) -> Result<String> {
        let args = spec.args();
        let out = self.run_checked(&args, Some(&spec.sandbox))?;
        placeholder(&out.stdout).ok_or_else(|| Error::Unexpected {
            args: args.join(" "),
            said: out.stdout.trim().to_string(),
        })
    }

    fn remove_secret(&self, placeholder: &str) -> Result<()> {
        self.run_checked(
            &strings(&["secret", "rm", "--placeholder", placeholder, "--force"]),
            None,
        )?;
        Ok(())
    }

    fn templates(&self) -> Result<Vec<Template>> {
        let listing: TemplateListing = self.json(&strings(&["template", "ls", "--json"]), None)?;
        Ok(listing.images)
    }

    fn load_template(&self, archive: &Path) -> Result<()> {
        let path = archive.display().to_string();
        self.run_checked(&strings(&["template", "load", &path]), None)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    #[test]
    fn reads_the_sandbox_listing() {
        let listing: Listing = serde_json::from_str(&fixture("ls.json")).unwrap();
        let all: Vec<Sandbox> = listing.sandboxes.into_iter().map(Sandbox::from).collect();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|s| s.name.starts_with("hura-spike-")));
        assert!(all.iter().all(|s| !s.id.is_empty()));
    }

    #[test]
    fn statuses_it_does_not_know_are_kept_as_themselves() {
        assert_eq!(Status::from("running"), Status::Running);
        assert_eq!(Status::from("stopped"), Status::Stopped);
        assert_eq!(
            Status::from("hibernating"),
            Status::Other("hibernating".into())
        );
        assert_eq!(Status::from("hibernating").to_string(), "hibernating");
    }

    #[test]
    fn reads_rules_of_both_kinds() {
        let listing: RuleListing = serde_json::from_str(&fixture("policy-ls.json")).unwrap();
        let http = listing
            .rules
            .iter()
            .find(|r| r.resource_type == "http" && r.decision == Decision::Deny)
            .expect("the path-level deny");
        assert_eq!(http.methods, ["POST"]);
        assert_eq!(http.http_targets[0].path, "/**/git-receive-pack");
        assert!(http.is_scoped() && http.is_network());

        // A sandbox's listing carries the global filesystem rules but not the
        // global network fallback (`default-deny-all`), which only the
        // unscoped `policy ls` shows.
        let global = listing
            .rules
            .iter()
            .find(|r| r.id == "default-fs-read-allow-all")
            .expect("a global rule");
        assert!(!global.is_scoped() && !global.is_network());
        assert!(!listing.rules.iter().any(|r| r.id == "default-deny-all"));
    }

    #[test]
    fn reads_the_decision_log() {
        let log: PolicyLog = serde_json::from_str(&fixture("policy-log.json")).unwrap();
        assert!(!log.blocked_hosts.is_empty() && !log.allowed_hosts.is_empty());
        let refused = log
            .blocked_hosts
            .iter()
            .find(|e| e.host == "example.com:443")
            .unwrap();
        assert_eq!(refused.proxy_type, "forward");
        assert!(refused.count_since >= 1);
        assert!(refused.reason.as_deref().unwrap().contains("default deny"));
    }

    #[test]
    fn reads_ports_secrets_templates_and_the_version() {
        let ports: Vec<Port> = serde_json::from_str(&fixture("ports.json")).unwrap();
        assert!(ports.iter().all(|p| p.host_ip == "127.0.0.1"));

        let secrets: SecretListing = serde_json::from_str(&fixture("secret-ls.json")).unwrap();
        assert!(secrets.custom_secrets[0].placeholder.starts_with("sbx-cs-"));

        let templates: TemplateListing =
            serde_json::from_str(&fixture("template-ls.json")).unwrap();
        assert!(templates.images.iter().any(|t| t.tag == "shell-docker"));
        assert!(templates.images.iter().all(|t| t.id.len() == 12));

        let version: Version = serde_json::from_str(&fixture("version.json")).unwrap();
        assert_eq!(version.client.version, "v0.47.0");
        assert_eq!(version.server.unwrap().state, "running");
    }

    #[test]
    fn a_missing_sandbox_is_told_apart_from_a_missing_anything_else() {
        let gone = ExecOutput {
            stdout: String::new(),
            stderr: "error: sandbox 'hura-nope' not found\n  try: sbx ls\n".into(),
            exit_code: 1,
        };
        let args = strings(&["rm", "--force", "hura-nope"]);
        assert!(matches!(
            refusal(&args, Some("hura-nope"), &gone),
            Error::NotFound(n) if n == "hura-nope"
        ));

        let no_image = ExecOutput {
            stderr: "error: template 'hura-base:x' not found\n".into(),
            ..gone.clone()
        };
        match refusal(&args, Some("hura-nope"), &no_image) {
            Error::Cli { stderr, .. } => assert_eq!(stderr, "template 'hura-base:x' not found"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_ids_and_ports_the_cli_prints_are_read_back() {
        assert_eq!(
            rule_id(
                "Rule added to policy local (scope: sandbox:hura-spike-a): \
                 fda93cfc-bbad-44bc-8e45-f29978e9a5a5 (github.com [tcp])\n"
            )
            .as_deref(),
            Some("fda93cfc-bbad-44bc-8e45-f29978e9a5a5")
        );
        assert_eq!(rule_id("nothing useful"), None);

        assert_eq!(
            placeholder(
                "Saved custom secret placeholder \"sbx-cs-TCmF6pXIfgZAPSQP\" for target \
                 \"httpbin.org\" env \"HURA_TEST_TOKEN\" in scope \"hura-spike-a\"\n\
                 Generated placeholder: sbx-cs-TCmF6pXIfgZAPSQP\n"
            )
            .as_deref(),
            Some("sbx-cs-TCmF6pXIfgZAPSQP")
        );

        assert_eq!(
            published("Published 127.0.0.1:44620 -> 8080/tcp4\n", 8080),
            Some(Port {
                host_ip: "127.0.0.1".into(),
                host_port: 44620,
                sandbox_port: 8080,
                protocol: "tcp4".into(),
            })
        );
        assert_eq!(
            published("Published 127.0.0.1:44620 -> 8081/tcp4\n", 8080),
            None
        );
    }

    #[test]
    fn the_commands_it_builds() {
        let create = CreateOpts {
            name: "hura-a".into(),
            template: "hura-base:latest".into(),
            cpus: Some(4),
            memory: Some("8g".into()),
        };
        assert_eq!(
            create.args().join(" "),
            "create shell --name hura-a --template hura-base:latest --pull never \
             --skills off --quiet --cpus 4 --memory 8g"
        );

        let rule = RuleSpec {
            decision: Decision::Allow,
            resources: vec!["github.com".into()],
            methods: vec!["GET".into(), "POST".into()],
            path: Some("/**/info/refs*".into()),
        };
        assert_eq!(
            rule.args("hura-a").join(" "),
            "policy allow network --sandbox hura-a --method GET,POST --path /**/info/refs* github.com"
        );

        let secret = SecretSpec {
            sandbox: "hura-a".into(),
            hosts: vec!["api.anthropic.com".into(), "platform.claude.com".into()],
            env: "CLAUDE_CODE_OAUTH_TOKEN".into(),
            source: SecretSource::Command("cat /home/u/.config/hura/tokens/claude".into()),
            placeholder: Some("sk-ant-oat01-{rand}".into()),
        };
        assert_eq!(
            secret.args().join(" "),
            "secret set-custom --sandbox hura-a --env CLAUDE_CODE_OAUTH_TOKEN \
             --host api.anthropic.com --host platform.claude.com \
             --command cat /home/u/.config/hura/tokens/claude --placeholder sk-ant-oat01-{rand}"
        );

        let client = CliClient::new().with_bin("/opt/sbx");
        assert_eq!(
            client
                .interactive_argv("hura-a", &["tmux", "attach"])
                .join(" "),
            "/opt/sbx exec --interactive --tty hura-a tmux attach"
        );
    }
}
