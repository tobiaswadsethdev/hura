//! The backend this tool exists for: a session inside its own microVM, made by
//! Docker Sandboxes, with the runtime's proxy holding everything that leaves it
//! to the session's rules.
//!
//! What [`crate::ops`] needs of a sandbox, behind the [`Backend`] trait so the
//! scripts above it never name the runtime.

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

    /// Take an endpoint out of every rule of one decision on this sandbox that
    /// names it, and put back what else those rules named.
    fn withdraw_from(&self, sandbox: &str, endpoint: &str, decision: Decision) -> Result<()> {
        for rule in self
            .client
            .rules(sandbox)?
            .iter()
            .filter(|r| r.is_scoped() && r.is_network() && r.decision == decision)
            .filter(|r| {
                r.resources
                    .iter()
                    .any(|x| policy::host_matches(x, endpoint))
            })
        {
            self.client.remove_rule(sandbox, &rule.id)?;
            let rest: Vec<String> = rule
                .resources
                .iter()
                .filter(|x| !policy::host_matches(x, endpoint))
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
        for rule in &lists.rules() {
            // A deny is refused beside an allow naming exactly the same thing,
            // which is what blocking an endpoint the template opens is, so the
            // template's allows of it go first.
            let landed = match rule.decision {
                Decision::Deny => rule
                    .resources
                    .iter()
                    .try_for_each(|e| self.withdraw_from(sandbox, e, Decision::Allow))
                    .and_then(|()| self.add(sandbox, rule)),
                Decision::Allow => self.add(sandbox, rule),
            };
            let Err(e) = landed else {
                continue;
            };
            let what = rule.resources.join(", ");
            if rule.decision == Decision::Deny {
                return Err(Error::Local(format!(
                    "the global block list could not be applied, so {what} would have been \
                     reachable: {e}"
                )));
            }
            warnings.push(format!(
                "the global allow list could not be applied, so {what} is not reachable: {e}"
            ));
        }
        Ok(())
    }
}

/// A runtime rule as the policy pane reads it.
fn rule_of(r: &Rule) -> policy::Rule {
    policy::Rule {
        id: r.id.clone(),
        allow: r.decision == Decision::Allow,
        hosts: r.resources.clone(),
        methods: r.methods.iter().map(|m| m.to_ascii_uppercase()).collect(),
        path: r.http_targets.first().map(|t| t.path.clone()),
        global: !r.is_scoped(),
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
        for rule in mcp::rules(&session.mcp) {
            if let Err(e) = self.add(&session.sandbox, &rule) {
                warnings.push(format!(
                    "the mcp endpoints could not be opened, so the agent will report {} \
                     unreachable: {e}",
                    session
                        .mcp
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        for rule in toolchain::rules(&draft.toolchains) {
            if let Err(e) = self.add(&session.sandbox, &rule) {
                warnings.push(format!(
                    "the toolchain registries could not be opened, so {} is not \
                     reachable and a restore will be denied: {e}",
                    rule.resources.join(", ")
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

    fn rules(&self, session: &Session) -> Result<Vec<policy::Rule>> {
        Ok(self
            .client
            .rules(&session.sandbox)?
            .iter()
            .filter(|r| r.is_network() && (r.status.is_empty() || r.status == "active"))
            .map(rule_of)
            .collect())
    }

    fn add_rules(&self, session: &Session, rules: &[RuleSpec]) -> Result<()> {
        for rule in rules {
            self.add(&session.sandbox, rule)?;
        }
        Ok(())
    }

    fn remove_rule(&self, session: &Session, id: &str) -> Result<()> {
        Ok(self.client.remove_rule(&session.sandbox, id)?)
    }

    fn withdraw(&self, session: &Session, endpoint: &str, allow: bool) -> Result<()> {
        let decision = if allow {
            Decision::Allow
        } else {
            Decision::Deny
        };
        self.withdraw_from(&session.sandbox, endpoint, decision)
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

    /// A withdrawal takes the endpoint out of the rules naming it and keeps
    /// the rest of what they named, and adds no deny: a later widen has to be
    /// able to open the endpoint again.
    #[test]
    fn a_withdrawn_endpoint_leaves_the_rest_of_its_rule() {
        let (b, fake) = backend(Settings::default());
        for rule in policy::resolve("feature-work").unwrap().rules {
            b.add("hura-a", &rule).unwrap();
        }
        b.withdraw_from("hura-a", "platform.claude.com:443", Decision::Allow)
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
    fn the_pane_reads_the_rules_on_the_sandbox() {
        let (b, fake) = backend(Settings::default());
        for rule in policy::resolve("feature-work").unwrap().rules {
            b.add("hura-a", &rule).unwrap();
        }
        let rules: Vec<policy::Rule> = fake.rules.borrow().iter().map(rule_of).collect();
        let push = rules
            .iter()
            .find(|r| {
                r.path.as_deref() == Some("/**/git-receive-pack") && r.names("github.com:443")
            })
            .expect("the push rule");
        assert_eq!(push.methods, ["POST"]);
        assert!(push.allow && !push.global);
        let api = rules
            .iter()
            .find(|r| r.names("api.anthropic.com:443"))
            .unwrap();
        assert!(api.whole());
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
