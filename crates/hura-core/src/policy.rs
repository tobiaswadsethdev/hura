//! Policy templates, the mid-run widen, and the policy pane's body.
//!
//! The isolation is the reason this tool exists rather than claude-squad, so it
//! is deliberately visible: a named template at creation, the effective rules
//! in a pane, and a keybinding to widen or tighten egress while the agent runs.
//!
//! Templates are hura's own: a list of network rules in TOML, embedded in the
//! binary and parsed here, which each become one rule on the session's sandbox.

use std::path::Path;

use sbx_client::{Decision, RuleSpec};

use crate::endpoints::{Lists, Route};
use crate::pane;

/// A policy shipped with the binary, selectable by name.
pub struct Template {
    pub name: &'static str,
    /// One line, for `--help` and the pane.
    pub summary: &'static str,
    pub toml: &'static str,
}

/// The templates, widest-denying first. Order is the order `hura new --help`
/// lists them in, so it reads as a range rather than a set.
pub const TEMPLATES: [Template; 3] = [
    Template {
        name: "readonly-explore",
        summary: "clone and read; no push, no model API",
        toml: include_str!("../../../policies/readonly-explore.toml"),
    },
    Template {
        name: "feature-work",
        summary: "clone, agent, push (github + azure devops)",
        toml: include_str!("../../../policies/feature-work.toml"),
    },
    Template {
        name: "net-open",
        summary: "feature-work plus npm and PyPI",
        toml: include_str!("../../../policies/net-open.toml"),
    },
];

/// The template used when `--policy` is not given.
pub const DEFAULT_TEMPLATE: &str = "feature-work";

pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

/// Template names and summaries, for help text.
pub fn help() -> String {
    TEMPLATES
        .iter()
        .map(|t| format!("{:<17}{}", t.name, t.summary))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A policy resolved to the rules a sandbox is given.
#[derive(Debug)]
pub struct Resolved {
    /// What to record in the session: the template name, or the path as given.
    pub label: String,
    pub rules: Vec<RuleSpec>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no policy template or file `{spec}`\n\navailable templates:\n{available}")]
    Unknown { spec: String, available: String },
    #[error("could not read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{name} is not a valid policy: {message}")]
    Invalid { name: String, message: String },
    #[error(
        "{path} is an OpenShell policy, which nothing enforces any more; write its \
         network rules as a hura template instead (see policies/feature-work.toml)"
    )]
    OpenShell { path: String },
}

/// Whether `--policy` names a file rather than a template. The one rule both
/// [`resolve`] and the config file's validation use, so the two cannot
/// disagree about what is a path.
pub fn looks_like_path(spec: &str) -> bool {
    spec.contains('/')
        || [".toml", ".yaml", ".yml"]
            .iter()
            .any(|ext| spec.ends_with(ext))
}

/// Resolve `--policy`: a template name, or a path to a template file.
///
/// A name is tried first, so a file called `net-open.toml` in the working
/// directory cannot silently shadow the template of that name, but a spec
/// that looks like a path (`./net-open`, `policies/net-open.toml`) is never
/// matched against a template, so the checked-in files stay usable directly.
pub fn resolve(spec: &str) -> Result<Resolved, Error> {
    if !looks_like_path(spec)
        && let Some(t) = find(spec)
    {
        return Ok(Resolved {
            label: t.name.to_string(),
            rules: parse(t.name, t.toml)?,
        });
    }
    let path = Path::new(spec);
    if !path.is_file() {
        return Err(Error::Unknown {
            spec: spec.to_string(),
            available: help(),
        });
    }
    if spec.ends_with(".yaml") || spec.ends_with(".yml") {
        return Err(Error::OpenShell {
            path: spec.to_string(),
        });
    }
    let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
        path: spec.to_string(),
        source,
    })?;
    Ok(Resolved {
        label: spec.to_string(),
        rules: parse(spec, &text)?,
    })
}

/// A template file, exactly: an unknown key is a typo, and a typo in a policy
/// is a rule that silently is not there.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTemplate {
    #[serde(default)]
    rule: Vec<RawRule>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    hosts: Vec<String>,
    #[serde(default)]
    methods: Vec<String>,
    path: Option<String>,
    decision: Option<String>,
}

/// The HTTP methods a rule may name, as the sandbox runtime takes them.
const METHODS: [&str; 10] = [
    "GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "CONNECT", "TRACE", "ANY",
];

/// Parse a template's text into rules. `name` is only for the messages.
pub fn parse(name: &str, text: &str) -> Result<Vec<RuleSpec>, Error> {
    let invalid = |message: String| Error::Invalid {
        name: name.to_string(),
        message,
    };
    let raw: RawTemplate = toml::from_str(text).map_err(|e| invalid(e.to_string()))?;
    raw.rule
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let n = i + 1;
            if r.hosts.is_empty() || r.hosts.iter().any(|h| h.trim().is_empty()) {
                return Err(invalid(format!("rule {n} names no host")));
            }
            if let Some(m) = r.methods.iter().find(|m| !METHODS.contains(&m.as_str())) {
                return Err(invalid(format!("rule {n}: `{m}` is not an HTTP method")));
            }
            if let Some(p) = &r.path {
                if r.methods.is_empty() {
                    return Err(invalid(format!(
                        "rule {n} has a path but no methods; a path narrows an HTTP rule"
                    )));
                }
                if !p.starts_with('/') {
                    return Err(invalid(format!("rule {n}: the path must start with `/`")));
                }
            }
            let decision = match r.decision.as_deref() {
                None | Some("allow") => Decision::Allow,
                Some("deny") => Decision::Deny,
                Some(other) => {
                    return Err(invalid(format!(
                        "rule {n}: decision is `allow` or `deny`, not `{other}`"
                    )));
                }
            };
            Ok(RuleSpec {
                decision,
                resources: r.hosts,
                methods: r.methods,
                path: r.path,
            })
        })
        .collect()
}

/// The mid-run widen: what a session reads packages from.
///
/// Read-only rather than open: a registry fetch is thousands of unpredictable
/// paths, but every one of them is a read, and nothing a session should do to
/// a registry is a write. Docker Hub is here too, since pulling an image is
/// how a session's own Docker Engine reads one.
pub struct Preset {
    pub label: &'static str,
    /// `host:port`.
    pub endpoints: &'static [&'static str],
}

pub const REGISTRIES: Preset = Preset {
    label: "package registries (npm, PyPI, Docker Hub)",
    endpoints: &[
        "registry.npmjs.org:443",
        "pypi.org:443",
        "files.pythonhosted.org:443",
        "registry-1.docker.io:443",
        "auth.docker.io:443",
        "production.cloudflare.docker.com:443",
        "production.cloudfront.docker.com:443",
    ],
};

/// The methods that read, which is what a read-only rule opens.
pub const READ_METHODS: [&str; 3] = ["GET", "HEAD", "OPTIONS"];

impl Preset {
    /// The rule that opens the preset: its hosts, for reading.
    pub fn rules(&self) -> Vec<RuleSpec> {
        vec![read_only(
            self.endpoints.iter().map(|e| (*e).to_string()).collect(),
        )]
    }

    /// Whether a session's rules already open every endpoint of it.
    pub fn applied(&self, view: &View) -> bool {
        self.endpoints.iter().all(|e| view.opens(e))
    }
}

/// An allow of these hosts for the read methods on any path.
pub fn read_only(hosts: Vec<String>) -> RuleSpec {
    RuleSpec {
        decision: Decision::Allow,
        resources: hosts,
        methods: READ_METHODS.map(String::from).to_vec(),
        path: Some("/**".to_string()),
    }
}

/// The policy pane's content, as facts rather than as text.
///
/// Introduced when a second thing had to draw it. `render` used to build marked
/// up text straight out of the gateway's reply, which is fine for one renderer
/// and wrong for two: a web view would have had to parse the terminal's markup
/// back into structure it never should have lost, and the wire would have
/// carried a rendering rather than an answer.
///
/// **Facts, not prose.** Each renderer says what these mean in its own voice,
/// so nothing here is English a renderer must show verbatim.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct View {
    /// The template the session was created from, which is recorded on the
    /// session rather than derivable from its rules.
    pub template: Option<String>,
    /// Every network rule that applies to the session, its own first.
    pub rules: Vec<Rule>,
    /// Omitted when the global lists are empty, which is the common case.
    pub lists: Option<ListsView>,
}

/// One rule on a session's sandbox, as the runtime holds it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rule {
    /// What removing it addresses.
    pub id: String,
    /// An allow, or a deny, which outranks every allow.
    pub allow: bool,
    /// Hosts, globs (`*.example.com` one label, `**.example.com` any number) or
    /// IPs, each with an optional `:port`.
    pub hosts: Vec<String>,
    /// The HTTP methods it is narrowed to. Empty for the whole host: any
    /// request, or any connection at all.
    pub methods: Vec<String>,
    /// The path glob it is narrowed to, when it is narrowed.
    pub path: Option<String>,
    /// Whether every sandbox has it, from the runtime's own policy, rather
    /// than this session alone. Only this session's can be changed here.
    pub global: bool,
}

impl Rule {
    /// Whether this rule is about an endpoint, `host:port`.
    pub fn names(&self, endpoint: &str) -> bool {
        self.hosts.iter().any(|h| host_matches(h, endpoint))
    }

    /// Whether it is about the whole host rather than some requests on it.
    pub fn whole(&self) -> bool {
        self.methods.is_empty()
    }
}

/// Whether a rule's host, glob or IP, with or without a port, covers an
/// endpoint. The runtime's own matching, as its documentation states it: a
/// bare host is any port and not its subdomains, `*.` is one label, `**.` any
/// number, and `**` is everything.
pub fn host_matches(resource: &str, endpoint: &str) -> bool {
    let (host, port) = match endpoint.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
        _ => (endpoint, None),
    };
    let (pattern, wanted) = match resource.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
        _ => (resource, None),
    };
    if wanted.is_some() && wanted != port {
        return false;
    }
    let host = host.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    if pattern == "**" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("**.") {
        return host.ends_with(&format!(".{suffix}"));
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return host
            .strip_suffix(&format!(".{suffix}"))
            .is_some_and(|label| !label.is_empty() && !label.contains('.'));
    }
    host == pattern
}

impl View {
    pub fn of(rules: Vec<Rule>, template: Option<&str>, lists: &Lists) -> Self {
        let mut rules = rules;
        // The session's own first, which are the ones a reader can change.
        rules.sort_by_key(|r| r.global);
        let lists = ListsView::of(&rules, lists);
        View {
            template: template.map(str::to_string),
            rules,
            lists,
        }
    }

    /// Whether a deny of the whole host is on it, which outranks any allow.
    pub fn denies(&self, endpoint: &str) -> bool {
        self.rules
            .iter()
            .any(|r| !r.allow && r.whole() && r.names(endpoint))
    }

    /// Whether some rule lets requests through to it, all of them or some.
    pub fn opens(&self, endpoint: &str) -> bool {
        !self.denies(endpoint) && self.rules.iter().any(|r| r.allow && r.names(endpoint))
    }

    /// Whether every request to it gets through, rather than only some paths.
    pub fn opens_whole(&self, endpoint: &str) -> bool {
        !self.denies(endpoint)
            && self
                .rules
                .iter()
                .any(|r| r.allow && r.whole() && r.names(endpoint))
    }
}

/// The global lists, each said against what this session actually has.
///
/// The second column is the one worth having: a list entry only describes what
/// a *new* session gets, and this session may predate the entry or have moved
/// since.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ListsView {
    pub allow: Vec<Listed>,
    /// Allows narrowed to methods and paths.
    pub routes: Vec<ListedRoutes>,
    pub block: Vec<Listed>,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Listed {
    pub endpoint: String,
    /// Whether this session has it: the allow, or for a block, the deny.
    pub in_policy: bool,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ListedRoutes {
    pub endpoint: String,
    pub routes: Vec<Route>,
    /// Whether every one of the routes is an allow on this session.
    pub in_policy: bool,
}

impl ListsView {
    fn of(rules: &[Rule], lists: &Lists) -> Option<Self> {
        if lists.is_empty() {
            return None;
        }
        let allowed = |endpoint: &str| rules.iter().any(|r| r.allow && r.names(endpoint));
        let denied = |endpoint: &str| {
            rules
                .iter()
                .any(|r| !r.allow && r.whole() && r.names(endpoint))
        };
        let routed = |endpoint: &str, route: &Route| {
            rules.iter().any(|r| {
                r.allow
                    && r.names(endpoint)
                    && (r.whole()
                        || (r.methods.iter().any(|m| *m == route.method || m == "ANY")
                            && r.path.as_deref() == Some(route.path.as_str())))
            })
        };
        Some(ListsView {
            allow: lists
                .allow
                .iter()
                .map(|a| Listed {
                    endpoint: a.endpoint.clone(),
                    in_policy: allowed(&a.endpoint) && !denied(&a.endpoint),
                })
                .collect(),
            routes: lists
                .routes
                .iter()
                .map(|r| ListedRoutes {
                    endpoint: r.endpoint.clone(),
                    routes: r.routes.clone(),
                    in_policy: !denied(&r.endpoint)
                        && r.routes.iter().all(|route| routed(&r.endpoint, route)),
                })
                .collect(),
            block: lists
                .block
                .iter()
                .map(|e| Listed {
                    endpoint: e.clone(),
                    in_policy: denied(e),
                })
                .collect(),
        })
    }
}

/// Render a [`View`] as the policy pane's body, for the terminal.
///
/// The session's own rules first: they are the ones that can be changed while
/// the agent runs, and the ones worth reading. The prose is this renderer's,
/// not the view's.
pub fn render(view: &View) -> String {
    let mut out = String::new();
    pane::section(&mut out, "policy");
    pane::field(
        &mut out,
        "template",
        view.template.as_deref().unwrap_or("(none recorded)"),
    );

    let (own, global): (Vec<&Rule>, Vec<&Rule>) = view.rules.iter().partition(|r| !r.global);
    out.push('\n');
    pane::section(&mut out, "network - this session");
    if own.is_empty() {
        pane::notice(
            &mut out,
            "no rules of its own: nothing in this sandbox has egress",
        );
    }
    for r in own {
        render_rule(&mut out, r);
    }
    if !global.is_empty() {
        out.push('\n');
        pane::section(&mut out, "network - every sandbox");
        for r in global {
            render_rule(&mut out, r);
        }
    }
    if let Some(lists) = &view.lists {
        render_lists(&mut out, lists);
    }
    out.push('\n');
    pane::notice(
        &mut out,
        "rules are for the whole sandbox, not for one program in it; a deny outranks",
    );
    pane::notice(&mut out, "every allow");
    out
}

fn render_rule(out: &mut String, r: &Rule) {
    let verdict = if r.allow { "allow" } else { "deny" };
    let what = if r.whole() {
        "any request".to_string()
    } else {
        format!(
            "{} {}",
            r.methods.join(","),
            r.path.as_deref().unwrap_or("/**")
        )
    };
    pane::field(out, verdict, format!("{}  {what}", r.hosts.join(", ")));
}

/// The global allow and block lists, and whether this sandbox reflects them.
///
/// Drawn here because this is the pane someone opens to answer "what may this
/// reach?", and a standing decision that is applied to every new session but
/// visible in no pane is exactly the kind of state that turns into a bug report.
fn render_lists(out: &mut String, lists: &ListsView) {
    out.push('\n');
    pane::section(out, "global lists - applied to every new session");
    let state = |on: bool| {
        if on {
            "in this session"
        } else {
            "NOT in this session"
        }
    };
    for a in &lists.allow {
        pane::field(
            out,
            "allow",
            format!("{}  {}", a.endpoint, state(a.in_policy)),
        );
    }
    for r in &lists.routes {
        pane::field(
            out,
            "allow",
            format!("{}  {}", r.endpoint, state(r.in_policy)),
        );
        for route in &r.routes {
            pane::field(out, "", format!("only {route}"));
        }
    }
    for b in &lists.block {
        pane::field(
            out,
            "block",
            format!("{}  {}", b.endpoint, state(b.in_policy)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, allow: bool, hosts: &[&str], methods: &[&str], path: Option<&str>) -> Rule {
        Rule {
            id: id.into(),
            allow,
            hosts: hosts.iter().map(|h| (*h).to_string()).collect(),
            methods: methods.iter().map(|m| (*m).to_string()).collect(),
            path: path.map(str::to_string),
            global: false,
        }
    }

    #[test]
    fn hosts_match_the_way_the_runtime_matches_them() {
        assert!(host_matches("github.com", "github.com:443"));
        assert!(host_matches("github.com:443", "github.com:443"));
        assert!(!host_matches("github.com:80", "github.com:443"));
        assert!(!host_matches("github.com", "api.github.com:443"));
        assert!(host_matches("*.github.com", "api.github.com:443"));
        assert!(!host_matches("*.github.com", "a.b.github.com:443"));
        assert!(!host_matches("*.github.com", "github.com:443"));
        assert!(host_matches(
            "**.githubusercontent.com",
            "a.b.githubusercontent.com:443"
        ));
        assert!(host_matches("**", "anything.example:22"));
        assert!(host_matches("GitHub.com", "github.com:443"));
    }

    /// A deny of the whole host outranks every allow, so an endpoint with one
    /// is not open whatever else names it.
    #[test]
    fn a_deny_closes_what_an_allow_opened() {
        let lists = Lists::default();
        let mut rules = vec![
            rule(
                "a",
                true,
                &["github.com:443"],
                &["GET"],
                Some("/**/info/refs*"),
            ),
            rule("b", true, &["api.anthropic.com:443"], &[], None),
        ];
        let view = View::of(rules.clone(), Some("feature-work"), &lists);
        assert!(view.opens("github.com:443"));
        assert!(!view.opens_whole("github.com:443"));
        assert!(view.opens_whole("api.anthropic.com:443"));
        assert!(!view.opens("example.com:443"));

        rules.push(rule("c", false, &["api.anthropic.com"], &[], None));
        let view = View::of(rules, None, &lists);
        assert!(view.denies("api.anthropic.com:443"));
        assert!(!view.opens("api.anthropic.com:443"));
    }

    #[test]
    fn the_session_s_own_rules_come_first() {
        let mut global = rule("g", true, &["**.docker.io"], &[], None);
        global.global = true;
        let own = rule("o", true, &["github.com"], &[], None);
        let view = View::of(vec![global, own], None, &Lists::default());
        assert_eq!(view.rules[0].id, "o");
    }

    #[test]
    fn the_global_lists_are_shown_against_this_session() {
        let mut lists = Lists::default();
        lists.allow("docs.rs:443");
        lists.allow("crates.io:443");
        lists.allow_routes(
            "pkgs.example.com:443",
            vec![Route::checked("GET", "/feed/**").unwrap()],
        );
        lists.block("pastebin.com:443");
        lists.block("platform.claude.com:443");
        let rules = vec![
            rule("a", true, &["docs.rs:443"], &[], None),
            rule(
                "b",
                true,
                &["pkgs.example.com:443"],
                &["GET"],
                Some("/feed/**"),
            ),
            rule("c", false, &["pastebin.com:443"], &[], None),
        ];
        let lv = View::of(rules, None, &lists).lists.unwrap();
        let on =
            |list: &[Listed], e: &str| list.iter().find(|x| x.endpoint == e).unwrap().in_policy;
        assert!(on(&lv.allow, "docs.rs:443"));
        assert!(!on(&lv.allow, "crates.io:443"));
        assert!(lv.routes[0].in_policy);
        assert!(on(&lv.block, "pastebin.com:443"));
        assert!(!on(&lv.block, "platform.claude.com:443"));
        assert!(View::of(vec![], None, &Lists::default()).lists.is_none());
    }

    #[test]
    fn the_widen_preset_is_read_only_and_known_once_applied() {
        let rules = REGISTRIES.rules();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].methods, READ_METHODS);
        assert!(
            rules[0]
                .resources
                .iter()
                .any(|h| h == "registry-1.docker.io:443")
        );
        let applied: Vec<Rule> = rules
            .iter()
            .map(|r| Rule {
                id: "w".into(),
                allow: true,
                hosts: r.resources.clone(),
                methods: r.methods.clone(),
                path: r.path.clone(),
                global: false,
            })
            .collect();
        let lists = Lists::default();
        assert!(REGISTRIES.applied(&View::of(applied, None, &lists)));
        assert!(!REGISTRIES.applied(&View::of(vec![], None, &lists)));
    }

    #[test]
    fn renders_the_session_s_rules_and_says_what_they_are_for() {
        let mut global = rule("g", true, &["**.docker.io"], &[], None);
        global.global = true;
        let view = View::of(
            vec![
                rule(
                    "a",
                    true,
                    &["github.com:443"],
                    &["POST"],
                    Some("/**/git-receive-pack"),
                ),
                rule("b", false, &["pastebin.com"], &[], None),
                global,
            ],
            Some("feature-work"),
            &Lists::default(),
        );
        let text = crate::pane::to_plain(&render(&view));
        assert!(text.contains("feature-work"), "{text}");
        assert!(
            text.contains("github.com:443  POST /**/git-receive-pack"),
            "{text}"
        );
        assert!(text.contains("pastebin.com  any request"), "{text}");
        assert!(text.contains("every sandbox"), "{text}");
        assert!(text.contains("not for one program"), "{text}");
    }

    #[test]
    fn every_template_is_findable_and_parses() {
        for t in &TEMPLATES {
            assert!(find(t.name).is_some(), "{} not findable", t.name);
            assert!(!t.summary.is_empty());
            let rules = parse(t.name, t.toml).unwrap();
            assert!(!rules.is_empty(), "{} opens nothing", t.name);
        }
        assert!(find(DEFAULT_TEMPLATE).is_some());
        assert!(find("no-such-template").is_none());
    }

    /// Whether a template opens `path` on `host` for `method`, by its rules
    /// rather than by its text, so a comment mentioning a host does not count.
    fn opens(name: &str, host: &str, method: &str, path: &str) -> bool {
        let rules = parse(name, find(name).unwrap().toml).unwrap();
        rules.iter().any(|r| {
            r.decision == Decision::Allow
                && r.resources
                    .iter()
                    .any(|h| h == host || h == &format!("{host}:443"))
                && (r.methods.is_empty() || r.methods.iter().any(|m| m == method))
                && r.path.as_deref().is_none_or(|p| glob(p, path))
        })
    }

    /// The runtime's path globs, enough for these tests: `**` any number of
    /// segments, `*` within one.
    fn glob(pattern: &str, path: &str) -> bool {
        fn go(p: &[u8], s: &[u8]) -> bool {
            match (p.first(), s.first()) {
                (None, None) => true,
                (Some(b'*'), _) if p.get(1) == Some(&b'*') => {
                    (0..=s.len()).any(|i| go(&p[2..], &s[i..]))
                }
                (Some(b'*'), _) => (0..=s.len())
                    .take_while(|&i| i == 0 || s[i - 1] != b'/')
                    .any(|i| go(&p[1..], &s[i..])),
                (Some(a), Some(b)) if a == b => go(&p[1..], &s[1..]),
                _ => false,
            }
        }
        go(pattern.as_bytes(), path.as_bytes())
    }

    /// readonly-explore has to actually deny what it claims to: no model API,
    /// and no push. A template that quietly allowed either would make the
    /// strict end of the range a lie.
    #[test]
    fn readonly_explore_denies_the_model_api_and_push() {
        let ro = "readonly-explore";
        assert!(!opens(ro, "api.anthropic.com", "POST", "/v1/messages"));
        let push = "/contoso/tools/_git/Repo/git-receive-pack";
        assert!(
            !opens(ro, "github.com", "POST", push),
            "push must not be allowed"
        );
        assert!(
            !opens(ro, "dev.azure.com", "POST", push),
            "push must not be allowed"
        );
        let fetch = "/octocat/Hello-World.git/git-upload-pack";
        assert!(
            opens(ro, "github.com", "POST", fetch),
            "fetch must still work"
        );
    }

    #[test]
    fn net_open_is_feature_work_plus_registries() {
        for name in ["feature-work", "net-open"] {
            assert!(opens(name, "api.anthropic.com", "POST", "/v1/messages"));
            assert!(opens(name, "api.github.com", "GET", "/repos/o/r"));
            assert!(opens(
                name,
                "github.com",
                "POST",
                "/o/r.git/git-receive-pack"
            ));
        }
        for host in ["registry.npmjs.org", "pypi.org", "files.pythonhosted.org"] {
            assert!(
                !opens("feature-work", host, "GET", "/x"),
                "feature-work must not reach {host}"
            );
            assert!(
                opens("net-open", host, "GET", "/x"),
                "net-open must reach {host}"
            );
            assert!(
                !opens("net-open", host, "PUT", "/x"),
                "net-open must not publish to {host}"
            );
        }
    }

    /// Both forges in every template. Reachability to a forge the session holds
    /// no credential for grants nothing that matters: the credential is a
    /// per-session secret, and a host is useless without it. To narrow it,
    /// copy the file and delete a block; `--policy <path>` takes it directly.
    #[test]
    fn every_template_covers_both_forges() {
        for t in &TEMPLATES {
            // Azure DevOps has the extra project level, but the git paths are
            // tail-anchored and identical, verified against a real clone of
            // /contoso/tools/_git/Contoso.DotFiles.
            let refs = "/contoso/tools/_git/Contoso.DotFiles/info/refs?service=git-upload-pack";
            assert!(
                opens(
                    t.name,
                    "dev.azure.com",
                    "GET",
                    refs.split('?').next().unwrap()
                ),
                "{}",
                t.name
            );
            assert!(
                opens(
                    t.name,
                    "github.com",
                    "GET",
                    "/octocat/Hello-World.git/info/refs"
                ),
                "{}",
                t.name
            );
        }
    }

    /// dev.azure.com serves git *and* the REST API, so `_apis` is the only
    /// thing separating "can fetch" from "can open a pull request". The
    /// read-only template must not have it.
    #[test]
    fn only_the_publishing_templates_reach_the_azure_rest_api() {
        let pr = "/contoso/tools/_apis/git/repositories/r/pullrequests";
        assert!(
            !opens("readonly-explore", "dev.azure.com", "POST", pr),
            "readonly-explore must not open PRs"
        );
        for name in ["feature-work", "net-open"] {
            assert!(
                opens(name, "dev.azure.com", "POST", pr),
                "{name} cannot open a PR"
            );
        }
    }

    #[test]
    fn resolves_a_template_by_name() {
        let r = resolve("feature-work").unwrap();
        assert_eq!(r.label, "feature-work");
        assert_eq!(
            r.rules,
            parse("feature-work", find("feature-work").unwrap().toml).unwrap()
        );
    }

    /// A path is relative to the working directory, which under `cargo test` is
    /// the package root rather than the workspace root, hence the manifest dir
    /// rather than a bare `policies/...`.
    #[test]
    fn resolves_a_path_to_a_template_file() {
        let spec = concat!(env!("CARGO_MANIFEST_DIR"), "/../../policies/net-open.toml");
        let r = resolve(spec).unwrap();
        assert_eq!(r.label, spec);
        assert!(!r.rules.is_empty());
    }

    /// An OpenShell policy is refused by name rather than read as something it
    /// is not: its binaries and filesystem sections have nothing to apply to.
    #[test]
    fn an_openshell_policy_file_is_refused_with_its_reason() {
        let dir = std::env::temp_dir().join(format!("hura-policy-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let yaml = dir.join("mine.yaml");
        std::fs::write(&yaml, "version: 1\n").unwrap();
        let e = resolve(yaml.to_str().unwrap()).unwrap_err();
        assert!(matches!(e, Error::OpenShell { .. }), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A typo is a rule that is silently not there, so it is refused.
    #[test]
    fn a_template_with_a_mistake_is_refused_with_where_it_is() {
        let typo = "[[rule]]\nhost = [\"github.com\"]\n";
        assert!(matches!(parse("t", typo), Err(Error::Invalid { .. })));
        let verb = "[[rule]]\nhosts = [\"github.com\"]\nmethods = [\"FETCH\"]\n";
        let e = parse("t", verb).unwrap_err().to_string();
        assert!(e.contains("rule 1") && e.contains("FETCH"), "{e}");
        let pathless = "[[rule]]\nhosts = [\"github.com\"]\npath = \"/x\"\n";
        assert!(
            parse("t", pathless).is_err(),
            "a path without methods narrows nothing"
        );
        let deny = "[[rule]]\nhosts = [\"x.com\"]\ndecision = \"deny\"\n";
        assert_eq!(parse("t", deny).unwrap()[0].decision, Decision::Deny);
    }

    /// A spec that looks like a path is never matched against a template, so
    /// the checked-in `policies/*.toml` stay directly usable, and a template
    /// name still wins over a same-named file in the working directory, so
    /// `--policy net-open` cannot be hijacked by a local file.
    #[test]
    fn a_template_name_and_a_path_do_not_shadow_each_other() {
        assert!(resolve("net-open").unwrap().label == "net-open");
        assert!(matches!(resolve("./net-open"), Err(Error::Unknown { .. }),));
        assert!(matches!(resolve("nope"), Err(Error::Unknown { .. })));
        // The error has to be actionable, so it lists what is available.
        let e = resolve("nope").unwrap_err().to_string();
        for t in &TEMPLATES {
            assert!(e.contains(t.name), "{e}");
        }
    }
}
