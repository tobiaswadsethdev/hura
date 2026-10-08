//! The backend this tool exists for: a session inside its own microVM, made by
//! Docker Sandboxes, with the runtime's proxy holding everything that leaves it
//! to the session's rules.
//!
//! What [`crate::ops`] needs of a sandbox, behind the [`Backend`] trait so the
//! scripts above it never name the runtime.

use std::collections::BTreeMap;

use openshell_client::{
    Endpoint, MethodPath, NetworkPolicy, Policy, PolicyRevision, PolicyUpdate, Rule as OsRule,
};
use sbx_client::{CreateOpts, Decision, Error as SbxError, Rule, RuleSpec, Sbx, Status};

use super::{Backend, Error, ExecOutput, Paths, Result, Torn};
use crate::credentials::{self, Credential};
use crate::doctor::Check;
use crate::endpoints;
use crate::events::{self, Event};
use crate::mcp;
use crate::ops::{Draft, ProviderChoice};
use crate::policy;
use crate::removed;
use crate::seed;
use crate::session::{self, Session};
use crate::store;
use crate::toolchain;

impl From<SbxError> for Error {
    fn from(e: SbxError) -> Self {
        match e {
            SbxError::NotFound(_) => Error::Missing(e.to_string()),
            other => Error::Refused(other.to_string()),
        }
    }
}

fn exec_output(out: sbx_client::ExecOutput) -> ExecOutput {
    ExecOutput {
        stdout: out.stdout,
        stderr: out.stderr,
        exit_code: out.exit_code,
    }
}

/// What a session's sandbox is made with, from the config file.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub cpus: Option<u32>,
    pub memory: Option<String>,
    pub credentials: Vec<Credential>,
}

impl Settings {
    pub fn from_config(cfg: &crate::config::Config) -> Self {
        Settings {
            cpus: cfg.sandbox_cpus,
            memory: cfg.sandbox_memory.clone(),
            credentials: cfg.credentials().to_vec(),
        }
    }
}

pub struct Sandboxed {
    client: Box<dyn Sbx>,
    settings: Settings,
}

impl Sandboxed {
    pub fn new(client: Box<dyn Sbx>, settings: Settings) -> Self {
        Sandboxed { client, settings }
    }

    /// The credentials a draft asks for, each with the placeholder it will
    /// have in the sandbox.
    ///
    /// Refused rather than skipped when a name is not configured: a session
    /// created without the credential it was asked for comes up to a login
    /// prompt or a failed clone, several steps from the cause.
    fn chosen(&self, draft: &Draft) -> Result<Vec<(&Credential, String)>> {
        let mut out: Vec<(&Credential, String)> = Vec::new();
        for name in &draft.providers {
            let c = self
                .settings
                .credentials
                .iter()
                .find(|c| &c.name == name)
                .ok_or_else(|| {
                    Error::Local(format!(
                        "no credential named `{name}`; add it under [credentials.{name}] \
                         in the config file"
                    ))
                })?;
            // One variable per kind, so two of a kind would leave the second
            // overwriting the first's placeholder.
            if let Some((other, _)) = out.iter().find(|(o, _)| o.kind == c.kind) {
                return Err(Error::Local(format!(
                    "`{}` and `{name}` are both {} credentials; a session can have one",
                    other.name,
                    c.kind.name()
                )));
            }
            out.push((c, credentials::placeholder(c.kind)));
        }
        Ok(out)
    }

    fn add(&self, sandbox: &str, rule: &RuleSpec) -> Result<String> {
        Ok(self.client.add_rule(sandbox, rule)?)
    }

    /// Apply an incremental change, written in the terms the callers still
    /// use, as rules on the sandbox.
    ///
    /// An added endpoint is an allow, narrowed to the read methods when its
    /// access is `read-only`. A path is an allow for that method and path. A
    /// removed endpoint takes the allows naming it away, and is not a deny: the
    /// same removal is what `--tighten` does, and a deny would outrank the
    /// allow a later `--widen` adds. Binaries have nothing to apply to.
    fn apply(&self, sandbox: &str, update: &PolicyUpdate) -> Result<()> {
        for spec in &update.add_endpoints {
            self.add(sandbox, &endpoint_rule(spec))?;
        }
        for spec in &update.add_allow {
            let rule = route_rule(spec)
                .ok_or_else(|| Error::Local(format!("`{spec}` is not host:port:METHOD:path")))?;
            self.add(sandbox, &rule)?;
        }
        if !update.remove_endpoints.is_empty() {
            let rules = self.client.rules(sandbox)?;
            for endpoint in &update.remove_endpoints {
                self.withdraw(sandbox, &rules, endpoint)?;
            }
        }
        Ok(())
    }

    /// Take an endpoint out of every allow on this sandbox that names it, and
    /// put back what else those rules named.
    fn withdraw(&self, sandbox: &str, rules: &[Rule], endpoint: &str) -> Result<()> {
        let host = endpoint.split(':').next().unwrap_or(endpoint);
        let names = |r: &str| r == endpoint || r == host;
        for rule in rules
            .iter()
            .filter(|r| r.is_scoped() && r.is_network() && r.decision == Decision::Allow)
            .filter(|r| r.resources.iter().any(|x| names(x)))
        {
            self.client.remove_rule(sandbox, &rule.id)?;
            let rest: Vec<String> = rule
                .resources
                .iter()
                .filter(|x| !names(x))
                .cloned()
                .collect();
            if !rest.is_empty() {
                self.add(
                    sandbox,
                    &RuleSpec {
                        decision: rule.decision,
                        resources: rest,
                        methods: rule.methods.clone(),
                        path: rule.http_targets.first().map(|t| t.path.clone()),
                    },
                )?;
            }
        }
        Ok(())
    }

    /// Apply the global allow and block lists to a sandbox that has just been
    /// made.
    ///
    /// A failed *block* fails the create. The two directions are not symmetric
    /// and pretending they are would be the worst kind of bug this tool can
    /// have: an allow that did not land leaves a session that cannot reach
    /// something, which the events pane will say out loud the moment the agent
    /// tries; a block that did not land leaves a session that *can* reach
    /// something the user asked to be unreachable, and nothing will ever
    /// mention it again. So the first is a warning and the second is fatal.
    fn impose_lists(&self, sandbox: &str, warnings: &mut Vec<String>) -> Result<()> {
        let lists = match endpoints::Lists::load() {
            Ok(l) => l,
            // An unreadable list is not a reason to refuse to create a session,
            // but it is a reason to say so: the session will not have the rules
            // its owner thinks every session has.
            Err(e) => {
                warnings.push(format!(
                    "could not read the global endpoint lists, so none were applied: {e}"
                ));
                return Ok(());
            }
        };
        for update in &lists.updates() {
            let Err(e) = self.apply(sandbox, update) else {
                continue;
            };
            if !update.remove_endpoints.is_empty() {
                return Err(Error::Local(format!(
                    "the global block list could not be applied, so {} would have been reachable: {e}",
                    update.remove_endpoints.join(", ")
                )));
            }
            warnings.push(format!(
                "the global allow list could not be applied, so {} is not reachable: {e}",
                update.add_endpoints.join(", ")
            ));
        }
        for entry in &lists.routes {
            for route in &entry.routes {
                let rule = RuleSpec {
                    decision: Decision::Allow,
                    resources: vec![entry.endpoint.clone()],
                    methods: vec![route.method.clone()],
                    path: Some(route.path.clone()),
                };
                if let Err(e) = self.add(sandbox, &rule) {
                    warnings.push(format!(
                        "the allowed path {} {} on {} could not be applied, so it is not \
                         reachable: {e}",
                        route.method, route.path, entry.endpoint
                    ));
                }
            }
        }
        Ok(())
    }
}

/// `host:port[:access[:protocol...]]` as one allow.
fn endpoint_rule(spec: &str) -> RuleSpec {
    let mut parts = spec.split(':');
    let host = parts.next().unwrap_or_default();
    let port = parts.next().filter(|p| !p.is_empty());
    let access = parts.next().unwrap_or_default();
    let resource = match port {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };
    let read_only = access == "read-only";
    RuleSpec {
        decision: Decision::Allow,
        resources: vec![resource],
        methods: if read_only {
            ["GET", "HEAD", "OPTIONS"].map(String::from).to_vec()
        } else {
            Vec::new()
        },
        path: read_only.then(|| "/**".to_string()),
    }
}

/// `host:port:METHOD:path` as one allow. Paths never hold a colon, since
/// [`endpoints::Route`] refuses one.
fn route_rule(spec: &str) -> Option<RuleSpec> {
    let mut parts = spec.splitn(4, ':');
    let (host, port, method, path) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    Some(RuleSpec {
        decision: Decision::Allow,
        resources: vec![format!("{host}:{port}")],
        methods: vec![method.to_string()],
        path: Some(path.to_string()),
    })
}

/// The rules on a sandbox, in the shape the policy pane is still drawn from:
/// one network rule per runtime rule, keyed by its id, with its hosts as
/// endpoints and its methods and paths as the endpoint's rules. A rule every
/// sandbox has is keyed `global-ID` so the pane can tell it from the session's
/// own. No binaries, because the runtime has none to name.
fn revision_of(rules: &[Rule]) -> PolicyRevision {
    let mut network = BTreeMap::new();
    for r in rules
        .iter()
        .filter(|r| r.is_network() && (r.status.is_empty() || r.status == "active"))
    {
        let http = r.resource_type == "http";
        let decided = |method: &str, path: &str| {
            let mp = MethodPath {
                method: method.to_string(),
                path: path.to_string(),
            };
            match r.decision {
                Decision::Allow => OsRule {
                    allow: Some(mp),
                    deny: None,
                },
                Decision::Deny => OsRule {
                    allow: None,
                    deny: Some(mp),
                },
            }
        };
        let mut lines = Vec::new();
        if http {
            for target in &r.http_targets {
                for m in &r.methods {
                    lines.push(decided(m, &target.path));
                }
            }
        } else if r.decision == Decision::Deny {
            lines.push(decided("*", "/**"));
        }
        let endpoints = r
            .resources
            .iter()
            .map(|res| {
                let (host, port) = match res.rsplit_once(':') {
                    Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => {
                        (h.to_string(), p.parse().unwrap_or(0))
                    }
                    _ => (res.clone(), 0),
                };
                Endpoint {
                    host,
                    port,
                    protocol: http.then(|| "rest".to_string()),
                    enforcement: Some("enforce".to_string()),
                    access: (!http && r.decision == Decision::Allow).then(|| "full".to_string()),
                    tls: None,
                    rules: lines.clone(),
                }
            })
            .collect();
        let key = if r.is_scoped() {
            r.id.clone()
        } else {
            format!("global-{}", r.id)
        };
        let verb = match r.decision {
            Decision::Allow => "allow",
            Decision::Deny => "deny",
        };
        network.insert(
            key,
            NetworkPolicy {
                name: Some(format!("{verb} {}", r.resources.join(", "))),
                endpoints,
                binaries: Vec::new(),
            },
        );
    }
    PolicyRevision {
        version: 1,
        active_version: 1,
        hash: String::new(),
        policy_source: "sandbox".to_string(),
        status: "loaded".to_string(),
        policy: Some(Policy {
            network_policies: network,
            ..Policy::default()
        }),
    }
}

fn live_status(s: &Status) -> store::Status {
    match s {
        Status::Running => store::Status::Running,
        Status::Stopped => store::Status::Stopped,
        Status::Other(o) => store::Status::Other(o.clone()),
    }
}

impl Backend for Sandboxed {
    fn paths(&self, _session: &Session) -> Paths {
        Paths::in_sandbox()
    }

    fn exec(&self, session: &Session, argv: &[&str]) -> Result<ExecOutput> {
        Ok(exec_output(self.client.exec(&session.sandbox, argv)?))
    }

    fn exec_stdin(&self, session: &Session, argv: &[&str], input: &[u8]) -> Result<ExecOutput> {
        Ok(exec_output(self.client.exec_stdin(
            &session.sandbox,
            argv,
            input,
        )?))
    }

    fn interactive_argv(&self, session: &Session, argv: &[&str]) -> Result<Vec<String>> {
        Ok(self.client.interactive_argv(&session.sandbox, argv))
    }

    /// A relay run by `hurad` itself: the runtime publishes only ports bound
    /// beyond the sandbox's loopback, and what is worth previewing is usually
    /// bound to it. See `hurad relay`.
    fn forward_argv(
        &self,
        session: &Session,
        port: u16,
        host: crate::ports::Loopback,
    ) -> Result<Vec<String>> {
        let me = std::env::current_exe()
            .map_err(|e| Error::Local(format!("could not find this program to relay with: {e}")))?;
        Ok(vec![
            me.display().to_string(),
            "relay".to_string(),
            session.sandbox.clone(),
            port.to_string(),
            host.address().to_string(),
        ])
    }

    /// The image sets the locale, and the runtime passes the image's
    /// environment through to an exec, but this is still said outright: a tmux
    /// client that does not believe it is on a UTF-8 terminal draws box rules
    /// with the DEC line-drawing set and replaces every character it cannot map
    /// with `_`, which turned Claude Code's banner and its `⏸` and `❯` glyphs
    /// into underscores once already. `-u` says "this terminal is UTF-8"
    /// whatever the environment, and `COLORTERM` is how the agent decides it
    /// may use 24-bit colour.
    fn tmux(&self) -> &'static str {
        "LANG=C.UTF-8 LC_ALL=C.UTF-8 COLORTERM=truecolor tmux -u -f /etc/tmux.conf"
    }

    /// One sandbox, one namespace: `shell-1` here cannot collide with anything,
    /// because nothing else runs in this filesystem.
    fn shell_prefix(&self, _session: &Session) -> String {
        "shell-".to_string()
    }

    fn place(&self, session: &mut Session, draft: &Draft) -> Result<()> {
        // Resolved before anything is created, so a typo in the policy fails
        // before a sandbox exists rather than after.
        let resolved = policy::resolve(&draft.policy).map_err(Error::local)?;
        session.policy = Some(resolved.label);
        let chosen = self.chosen(draft)?;

        self.client.create(&CreateOpts {
            name: session.sandbox.clone(),
            // The base image for a session with no toolchain, and the variant
            // carrying exactly the ones asked for otherwise. Not built here, for
            // the reason the base image is not: see `ops::create` on docker's
            // output.
            template: toolchain::tag(&draft.toolchains),
            cpus: self.settings.cpus,
            memory: self.settings.memory.clone(),
            env: chosen
                .iter()
                .map(|(c, ph)| (c.kind.env().to_string(), ph.clone()))
                .collect(),
        })?;
        // Without this the runtime stops the sandbox thirty seconds after the
        // last exec disconnects, and the agent in it with it.
        self.client.detach(&session.sandbox)?;
        for (c, ph) in &chosen {
            self.client
                .add_secret(&c.secret(&session.sandbox, ph))
                .map_err(|e| {
                    Error::Local(format!(
                        "the `{}` credential could not be given to the sandbox: {e}",
                        c.name
                    ))
                })?;
        }
        Ok(())
    }

    fn configure(
        &self,
        session: &Session,
        draft: &Draft,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        // The template first, then everything layered on it. Nothing runs in
        // the sandbox until the seeder, so the window before these land is
        // empty.
        let resolved = policy::resolve(&draft.policy).map_err(Error::local)?;
        for rule in &resolved.rules {
            self.add(&session.sandbox, rule).map_err(|e| {
                Error::Local(format!(
                    "the {} policy could not be applied ({}): {e}",
                    resolved.label,
                    rule.resources.join(", ")
                ))
            })?;
        }
        self.impose_lists(&session.sandbox, warnings)?;
        if let Some(update) = mcp::widen(&session.mcp)
            && let Err(e) = self.apply(&session.sandbox, &update)
        {
            warnings.push(format!(
                "the mcp endpoints could not be opened, so the agent will report {} unreachable: {e}",
                session
                    .mcp
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for update in toolchain::updates(&draft.toolchains) {
            if let Err(e) = self.apply(&session.sandbox, &update) {
                warnings.push(format!(
                    "the toolchain registries could not be opened, so {} is not \
                     reachable and a restore will be denied: {e}",
                    update.add_endpoints.join(", ")
                ));
            }
        }
        Ok(())
    }

    fn fetch_script(&self, session: &Session) -> String {
        seed::clone_and_branch(session, &self.paths(session))
    }

    fn tear_down(&self, name: &str, session: Option<&Session>) -> Result<Torn> {
        // Through the cache with a fall back to the naming convention, so a
        // session the cache has lost is still removable, which is what the
        // convention is for. The sandbox's rules and secrets go with it.
        let sandbox = session
            .map(|s| s.sandbox.clone())
            .unwrap_or_else(|| session::sandbox_name(name));
        match self.client.remove(&sandbox) {
            Ok(()) => Ok(Torn::Removed),
            // The desired end state rather than a failure: that is the case for
            // a session left behind by a create that died before the sandbox
            // existed, and refusing to remove the record would make it
            // permanent.
            Err(SbxError::NotFound(_)) => Ok(Torn::RecordOnly),
            Err(e) => Err(e.into()),
        }
    }

    fn live(&self, cached: Vec<Session>) -> Result<store::Reconciliation> {
        let live: Vec<store::Live> = self
            .client
            .list()?
            .iter()
            .filter_map(|sb| {
                let owner = session::session_of(&sb.name)?;
                Some(store::Live {
                    sandbox: sb.name.clone(),
                    session: Some(owner.to_string()),
                    status: live_status(&sb.status),
                })
            })
            .collect();
        Ok(store::reconcile(cached, &live, &removed::names()))
    }

    fn read_meta(&self, name: &str) -> Result<Session> {
        let out = self
            .client
            .exec(&session::sandbox_name(name), &seed::READ_META)?;
        seed::parse_meta(&exec_output(out)).map_err(|e| Error::Local(e.to_string()))
    }

    fn policy(&self, session: &Session) -> Result<PolicyRevision> {
        Ok(revision_of(&self.client.rules(&session.sandbox)?))
    }

    fn policy_update(&self, session: &Session, update: &PolicyUpdate) -> Result<()> {
        self.apply(&session.sandbox, update)
    }

    fn events(&self, session: &Session) -> Result<Vec<Event>> {
        Ok(events::from_sbx(&self.client.log(&session.sandbox)?))
    }

    fn providers(&self) -> Result<Vec<ProviderChoice>> {
        Ok(self
            .settings
            .credentials
            .iter()
            .map(|c| ProviderChoice {
                name: c.name.clone(),
                kind: c.kind.name().to_string(),
            })
            .collect())
    }

    fn health(&self) -> Check {
        match self.client.version() {
            Ok(v) => match v.server {
                Some(server) if server.state == "running" => Check::ok(
                    "sbx",
                    format!("{} (daemon {})", v.client.version, server.version),
                ),
                _ => Check::fail(
                    "sbx",
                    format!(
                        "{} is installed but its daemon is not running",
                        v.client.version
                    ),
                    "systemctl --user enable --now sbx-daemon, or `sbx daemon start -d`",
                ),
            },
            Err(SbxError::Spawn { .. }) => Check::fail(
                "sbx",
                "Docker Sandboxes is not installed",
                "install the release tarball from https://github.com/docker/sbx-releases \
                 (its install.sh puts sbx in ~/.docker/sbx/bin), then `sbx login`",
            ),
            Err(e) => Check::fail("sbx", e.to_string(), "sbx diagnose"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use sbx_client::{
        CustomSecret, HttpTarget, PolicyLog, Port, Result as SbxResult, Sandbox, SecretSpec,
        Template, Version,
    };

    use super::*;

    /// A runtime that keeps rules in memory and records the rest of what it
    /// is asked, so the translation from the callers' terms can be read back.
    #[derive(Default)]
    struct Fake {
        rules: RefCell<Vec<Rule>>,
        next: RefCell<u32>,
        calls: RefCell<Vec<String>>,
    }

    impl Fake {
        fn added(&self) -> Vec<(Vec<String>, Vec<String>, Option<String>)> {
            self.rules
                .borrow()
                .iter()
                .map(|r| {
                    (
                        r.resources.clone(),
                        r.methods.clone(),
                        r.http_targets.first().map(|t| t.path.clone()),
                    )
                })
                .collect()
        }
    }

    /// Shared, so a test can read the fake back after handing it to the
    /// backend.
    struct Shared(Rc<Fake>);

    impl Sbx for Shared {
        fn version(&self) -> SbxResult<Version> {
            unreachable!()
        }
        fn create(&self, opts: &CreateOpts) -> SbxResult<()> {
            self.0
                .calls
                .borrow_mut()
                .push(format!("create {}", opts.name));
            Ok(())
        }
        fn detach(&self, name: &str) -> SbxResult<()> {
            self.0.calls.borrow_mut().push(format!("detach {name}"));
            Ok(())
        }
        fn list(&self) -> SbxResult<Vec<Sandbox>> {
            unreachable!()
        }
        fn exec(&self, _: &str, _: &[&str]) -> SbxResult<sbx_client::ExecOutput> {
            unreachable!()
        }
        fn exec_stdin(&self, _: &str, _: &[&str], _: &[u8]) -> SbxResult<sbx_client::ExecOutput> {
            unreachable!()
        }
        fn interactive_argv(&self, _: &str, _: &[&str]) -> Vec<String> {
            unreachable!()
        }
        fn remove(&self, _: &str) -> SbxResult<()> {
            unreachable!()
        }
        fn rules(&self, _: &str) -> SbxResult<Vec<Rule>> {
            Ok(self.0.rules.borrow().clone())
        }
        fn add_rule(&self, sandbox: &str, spec: &RuleSpec) -> SbxResult<String> {
            let mut n = self.0.next.borrow_mut();
            *n += 1;
            let id = format!("r{n}");
            let http = !spec.methods.is_empty();
            self.0.rules.borrow_mut().push(Rule {
                id: id.clone(),
                scope: format!("sandbox:{sandbox}"),
                resource_type: if http { "http" } else { "network" }.into(),
                decision: spec.decision,
                resources: spec.resources.clone(),
                origin: "scoped".into(),
                status: "active".into(),
                editable: true,
                methods: spec.methods.clone(),
                http_targets: spec
                    .path
                    .iter()
                    .map(|p| HttpTarget {
                        host: spec.resources[0].clone(),
                        path: p.clone(),
                    })
                    .collect(),
            });
            Ok(id)
        }
        fn remove_rule(&self, _: &str, id: &str) -> SbxResult<()> {
            self.0.rules.borrow_mut().retain(|r| r.id != id);
            Ok(())
        }
        fn log(&self, _: &str) -> SbxResult<PolicyLog> {
            unreachable!()
        }
        fn ports(&self, _: &str) -> SbxResult<Vec<Port>> {
            unreachable!()
        }
        fn publish(&self, _: &str, _: u16) -> SbxResult<Port> {
            unreachable!()
        }
        fn unpublish(&self, _: &str, _: &Port) -> SbxResult<()> {
            unreachable!()
        }
        fn secrets(&self) -> SbxResult<Vec<CustomSecret>> {
            unreachable!()
        }
        fn add_secret(&self, spec: &SecretSpec) -> SbxResult<String> {
            self.0
                .calls
                .borrow_mut()
                .push(format!("secret {} {}", spec.sandbox, spec.env));
            Ok(spec.placeholder.clone().unwrap_or_default())
        }
        fn remove_secret(&self, _: Option<&str>, _: &str) -> SbxResult<()> {
            unreachable!()
        }
        fn templates(&self) -> SbxResult<Vec<Template>> {
            unreachable!()
        }
        fn load_template(&self, _: &std::path::Path) -> SbxResult<()> {
            unreachable!()
        }
    }

    fn backend(settings: Settings) -> (Sandboxed, Rc<Fake>) {
        let fake = Rc::new(Fake::default());
        (
            Sandboxed::new(Box::new(Shared(fake.clone())), settings),
            fake,
        )
    }

    fn draft(providers: &[&str]) -> Draft {
        Draft {
            name: "a".into(),
            branch: None,
            ticket: None,
            project: None,
            repo: "https://example.com/r.git".into(),
            task: "t".into(),
            base: None,
            policy: "feature-work".into(),
            providers: providers.iter().map(|p| (*p).to_string()).collect(),
            skills: Vec::new(),
            mcp: Vec::new(),
            toolchains: Vec::new(),
            start: false,
            interface: crate::chat::Interface::Terminal,
        }
    }

    #[test]
    fn an_update_in_the_old_terms_becomes_rules() {
        let (b, fake) = backend(Settings::default());
        b.apply(
            "hura-a",
            &PolicyUpdate {
                add_endpoints: vec![
                    "registry.npmjs.org:443:read-only:rest:enforce".into(),
                    "crates.io:443".into(),
                ],
                add_allow: vec!["pkgs.example.com:443:GET:/contoso/_packaging/**".into()],
                binaries: vec!["/usr/bin/node".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            fake.added(),
            [
                (
                    vec!["registry.npmjs.org:443".to_string()],
                    vec!["GET".to_string(), "HEAD".into(), "OPTIONS".into()],
                    Some("/**".to_string())
                ),
                (vec!["crates.io:443".to_string()], vec![], None),
                (
                    vec!["pkgs.example.com:443".to_string()],
                    vec!["GET".to_string()],
                    Some("/contoso/_packaging/**".to_string())
                ),
            ]
        );
    }

    /// A removal takes the endpoint out of the rules naming it and keeps the
    /// rest of what they named, and adds no deny: a later widen has to be able
    /// to open the endpoint again.
    #[test]
    fn a_removed_endpoint_leaves_the_rest_of_its_rule() {
        let (b, fake) = backend(Settings::default());
        for rule in policy::resolve("feature-work").unwrap().rules {
            b.add("hura-a", &rule).unwrap();
        }
        b.apply(
            "hura-a",
            &PolicyUpdate {
                remove_endpoints: vec!["platform.claude.com:443".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let now = fake.added();
        assert!(now.iter().any(|(r, _, _)| r == &["api.anthropic.com:443"]));
        assert!(
            !now.iter()
                .any(|(r, _, _)| r.iter().any(|h| h.starts_with("platform.")))
        );
        assert!(
            fake.rules
                .borrow()
                .iter()
                .all(|r| r.decision == Decision::Allow)
        );
    }

    #[test]
    fn the_pane_is_drawn_from_the_rules_on_the_sandbox() {
        let (b, fake) = backend(Settings::default());
        for rule in policy::resolve("feature-work").unwrap().rules {
            b.add("hura-a", &rule).unwrap();
        }
        let rev = revision_of(&fake.rules.borrow());
        assert!(rev.is_settled());
        let policy = rev.policy.unwrap();
        let endpoints: Vec<&Endpoint> = policy
            .network_policies
            .values()
            .flat_map(|n| &n.endpoints)
            .collect();
        let push = endpoints
            .iter()
            .find(|e| {
                e.host == "github.com"
                    && e.rules.iter().any(|r| {
                        r.allow
                            .as_ref()
                            .is_some_and(|mp| mp.path.ends_with("git-receive-pack"))
                    })
            })
            .expect("the push rule");
        assert_eq!(push.port, 443);
        assert_eq!(push.protocol.as_deref(), Some("rest"));
        let api = endpoints
            .iter()
            .find(|e| e.host == "api.anthropic.com")
            .unwrap();
        assert_eq!(api.access.as_deref(), Some("full"));
    }

    #[test]
    fn a_session_is_made_detached_with_the_credentials_it_asked_for() {
        let cred = |name: &str, kind| Credential {
            name: name.into(),
            kind,
            source: credentials::Source::Command("true".into()),
        };
        let settings = Settings {
            credentials: vec![
                cred("claude", credentials::Kind::ClaudeOauth),
                cred("ado", credentials::Kind::AzureDevOpsPat),
            ],
            ..Default::default()
        };
        let (b, fake) = backend(settings);
        let mut s = Session::new("a".into(), "https://example.com/r.git".into(), "t".into());
        b.place(&mut s, &draft(&["claude"])).unwrap();
        assert_eq!(s.policy.as_deref(), Some("feature-work"));
        assert_eq!(
            *fake.calls.borrow(),
            [
                "create hura-a",
                "detach hura-a",
                "secret hura-a CLAUDE_CODE_OAUTH_TOKEN"
            ]
        );
        assert!(
            b.place(&mut s, &draft(&["nope"])).is_err(),
            "an unknown credential is refused"
        );
    }
}
