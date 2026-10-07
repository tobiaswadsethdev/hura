//! The global allow and block lists.
//!
//! `w` and `t` change one session's egress and forget it. These are the same
//! decision made once: an endpoint on the allow list is opened on every
//! `hura new`, and one on the block list is closed on every `hura new` --
//! including endpoints the policy template itself grants, which is the only
//! thing a block list *can* mean under an engine that denies by default.
//!
//! That asymmetry is worth being explicit about, because "blacklist" invites
//! the wrong model. OpenShell has no deny-overrides-allow layer at L4; an
//! endpoint is unreachable unless a rule names it. So blocking `pastebin.com`
//! is a no-op -- it was never reachable -- and blocking `platform.claude.com`
//! is real, because `feature-work.yaml` grants it. The pane says which.
//!
//! `$XDG_CONFIG_HOME/hura/endpoints.json`, beside the session cache, and a cache
//! in the same sense: losing it costs the lists and nothing else. Written under
//! a lock of its own for the reason [`crate::store`] holds one -- a TUI and a
//! `hura new` in another terminal are the normal case, not an edge, and
//! load-modify-save without a lock loses whichever write lands second.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use openshell_client::{Policy, PolicyUpdate};

use crate::store::Store;

/// The access class an allow entry grants.
///
/// Not configurable, and deliberately the broadest one. This list is written by
/// pressing a key next to a denial that has already happened, which means the
/// answer to "what does this need?" is "whatever it was trying to do"; offering
/// a choice between `full` and `read-only` at that moment would be asking a
/// question the user is not in a position to answer. A narrower grant belongs in
/// a policy template, where it can be written down with a reason.
const ACCESS: &str = "full";

/// An endpoint opened for every new session, and what may reach it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Allow {
    /// `host:port`, which is what `policy update` addresses.
    pub endpoint: String,
    /// Kernel-resolved binary paths, as the log reported them. Never empty: an
    /// endpoint rule with no binaries grants nothing, so an allow that could not
    /// name one is refused at the point it is asked for rather than written
    /// here and silently doing nothing. See [`Lists::allow`].
    pub binaries: Vec<String>,
}

/// One method and path an allow is narrowed to:
/// `GET /contoso/tools/_packaging/feed/nuget/v3/**`.
///
/// Enforced rather than advisory, because every endpoint this module opens is
/// `rest`, which the gateway inspects request by request. A rules block with no
/// access class beside it is default-deny, so a rule carrying only routes
/// grants those and nothing else on the host.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Route {
    /// Upper case, or `*` for any method.
    pub method: String,
    /// From the first `/`, with `*` and `**` as globs.
    pub path: String,
}

impl Route {
    /// A route as it may go into a policy change, or why it may not.
    pub fn checked(method: &str, path: &str) -> Result<Route, String> {
        let method = method.trim().to_ascii_uppercase();
        let is_method = method == "*"
            || (!method.is_empty()
                && method.len() <= 16
                && method.chars().all(|c| c.is_ascii_uppercase()));
        if !is_method {
            return Err(format!("`{method}` is not an HTTP method"));
        }
        let path = path.trim();
        if !path.starts_with('/') {
            return Err(format!("`{path}` is not a path: it has to start with /"));
        }
        // `--add-allow` is `host:port:METHOD:path_glob`, and the CLI refuses a
        // colon in the glob rather than guessing where the path starts.
        if path.contains(':') {
            return Err(format!(
                "`{path}` has a colon in it, which the gateway refuses"
            ));
        }
        if path.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(format!("`{path}` has a space in it"));
        }
        Ok(Route {
            method,
            path: path.to_string(),
        })
    }

    /// What `--add-allow` takes.
    fn spec(&self, endpoint: &str) -> String {
        format!("{endpoint}:{}:{}", self.method, self.path)
    }
}

impl std::fmt::Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.method, self.path)
    }
}

/// An endpoint opened for every new session, for some paths only.
///
/// Its own list rather than a field on [`Allow`], so that a build from before
/// routes skips these entries instead of reading them as an allow of the whole
/// host: serde ignores a key it does not know, and here the key it does not
/// know is the entry rather than the narrowing.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RouteAllow {
    /// `host:port`.
    pub endpoint: String,
    /// Kernel-resolved binary paths. Who gets the routes when no rule names
    /// the endpoint yet; when one does, the routes join it and these have to
    /// be among the binaries it already grants. See [`routes_update`].
    pub binaries: Vec<String>,
    /// Never empty. Sorted, without repeats.
    pub routes: Vec<Route>,
}

/// The lists, as they are on disk.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Lists {
    pub allow: Vec<Allow>,
    /// Allows narrowed to routes. An endpoint is on at most one of `allow`,
    /// `routes` and `block`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<RouteAllow>,
    /// `host:port` for each. No binaries: removing an endpoint removes it for
    /// everything, which is the only granularity `--remove-endpoint` has.
    pub block: Vec<String>,
}

impl Lists {
    /// `$XDG_CONFIG_HOME/hura/endpoints.json`, beside the session cache.
    pub fn default_path() -> PathBuf {
        Store::default_path().with_file_name("endpoints.json")
    }

    pub fn load() -> io::Result<Self> {
        Self::load_from(Self::default_path())
    }

    pub fn load_from(path: impl Into<PathBuf>) -> io::Result<Self> {
        match fs::read_to_string(path.into()) {
            Ok(text) => serde_json::from_str(&text).map_err(io::Error::other),
            // No file is the normal first-run case, not an error.
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Lists::default()),
            Err(e) => Err(e),
        }
    }

    /// Temp file and rename, like the session cache: an interrupted write must
    /// not truncate the lists.
    fn save_to(&self, path: &std::path::Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)
    }

    pub fn is_empty(&self) -> bool {
        self.allow.is_empty() && self.routes.is_empty() && self.block.is_empty()
    }

    /// Whether the lists name an endpoint, and which way.
    pub fn verdict(&self, endpoint: &str) -> Option<Listed> {
        if self.block.iter().any(|e| e == endpoint) {
            return Some(Listed::Blocked);
        }
        (self.allow.iter().any(|a| a.endpoint == endpoint)
            || self.routes.iter().any(|r| r.endpoint == endpoint))
        .then_some(Listed::Allowed)
    }

    /// Put an endpoint on the allow list, taking it off the block list.
    ///
    /// The two are mutually exclusive by construction rather than by rule: an
    /// endpoint on both would make the answer depend on which list is consulted
    /// first, and there is no reading of "allowed and blocked" worth having.
    ///
    /// Replaces an existing entry rather than merging its binaries, so pressing
    /// the key twice against two different denials leaves the list saying what
    /// the second one said. Merging would grow a rule nobody wrote.
    pub fn allow(&mut self, endpoint: &str, binaries: Vec<String>) {
        self.block.retain(|e| e != endpoint);
        self.routes.retain(|r| r.endpoint != endpoint);
        self.allow.retain(|a| a.endpoint != endpoint);
        self.allow.push(Allow {
            endpoint: endpoint.to_string(),
            binaries,
        });
        self.allow.sort_by(|a, b| a.endpoint.cmp(&b.endpoint));
    }

    /// Put routes of an endpoint on the allow list, taking the endpoint off
    /// the block list and replacing an allow of the whole of it.
    ///
    /// Added to the routes already listed when the binaries are the same,
    /// which is not the merge [`Self::allow`] refuses: each route was asked for
    /// against a denial of its own, and a feed needs its index, its metadata
    /// and its packages before a restore works. Different binaries replace,
    /// for the reason [`Self::allow`] gives.
    pub fn allow_routes(&mut self, endpoint: &str, binaries: Vec<String>, routes: Vec<Route>) {
        self.block.retain(|e| e != endpoint);
        self.allow.retain(|a| a.endpoint != endpoint);
        let mut routes = routes;
        if let Some(i) = self.routes.iter().position(|r| r.endpoint == endpoint) {
            let listed = self.routes.remove(i);
            if same_set(&listed.binaries, &binaries) {
                routes.extend(listed.routes);
            }
        }
        routes.sort();
        routes.dedup();
        self.routes.push(RouteAllow {
            endpoint: endpoint.to_string(),
            binaries,
            routes,
        });
        self.routes.sort_by(|a, b| a.endpoint.cmp(&b.endpoint));
    }

    /// Put an endpoint on the block list, taking it off the allow list.
    pub fn block(&mut self, endpoint: &str) {
        self.allow.retain(|a| a.endpoint != endpoint);
        self.routes.retain(|r| r.endpoint != endpoint);
        if !self.block.iter().any(|e| e == endpoint) {
            self.block.push(endpoint.to_string());
            self.block.sort();
        }
    }

    /// Take an endpoint off whichever list it is on. Whether it was on one.
    ///
    /// Only the lists change: a live sandbox keeps what the entry gave it, the
    /// same way a new entry does not reach back into sessions already running.
    pub fn forget(&mut self, endpoint: &str) -> bool {
        let before = self.allow.len() + self.routes.len() + self.block.len();
        self.allow.retain(|a| a.endpoint != endpoint);
        self.routes.retain(|r| r.endpoint != endpoint);
        self.block.retain(|e| e != endpoint);
        before != self.allow.len() + self.routes.len() + self.block.len()
    }

    /// The policy updates that impose these lists on a fresh sandbox.
    ///
    /// Usually one. More only when the allow list names endpoints with
    /// different binaries, because `--binary` applies to *every*
    /// `--add-endpoint` in an invocation -- the same constraint that makes
    /// [`crate::policy::Preset`] a single call with a merged binary list. Here
    /// the entries were written at different times against different denials,
    /// so merging them would grant each endpoint every other one's binaries.
    ///
    /// Removals ride on the first update, or on one of their own when there is
    /// nothing to add.
    ///
    /// Not [`Self::routes`]: how a route lands depends on what the policy
    /// already says about its endpoint, so those are planned against it with
    /// [`routes_update`] once these have been applied.
    pub fn updates(&self) -> Vec<PolicyUpdate> {
        // Grouped by binary list so endpoints that share one share a call.
        // Ordered, so the plan is the same every time it is computed.
        let mut groups: BTreeMap<Vec<String>, Vec<String>> = BTreeMap::new();
        for a in &self.allow {
            groups
                .entry(a.binaries.clone())
                .or_default()
                .push(format!("{}:{ACCESS}:rest:enforce", a.endpoint));
        }

        let mut out: Vec<PolicyUpdate> = groups
            .into_iter()
            .map(|(binaries, add_endpoints)| PolicyUpdate {
                add_endpoints,
                binaries,
                // Rejected outright for a multi-endpoint update, and the
                // gateway's own derived name (`allow_pastebin_com_443`) is
                // already the clearer of the two.
                rule_name: None,
                // The next thing this sandbox does is clone a repository, so
                // returning before the rules load would be a lie.
                wait: true,
                ..Default::default()
            })
            .collect();

        if self.block.is_empty() {
            return out;
        }
        match out.first_mut() {
            Some(first) => first.remove_endpoints = self.block.clone(),
            None => out.push(PolicyUpdate {
                remove_endpoints: self.block.clone(),
                wait: true,
                ..Default::default()
            }),
        }
        out
    }
}

/// Which list an endpoint is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listed {
    Allowed,
    Blocked,
}

impl Listed {
    pub fn label(self) -> &'static str {
        match self {
            Listed::Allowed => "allowed everywhere",
            Listed::Blocked => "blocked everywhere",
        }
    }
}

/// One endpoint's worth of change to a live sandbox.
///
/// Shared by the events pane and [`Lists::updates`]'s callers so a decision
/// applied here and the same decision applied at the next `hura new` cannot
/// drift apart.
///
/// Measured against 0.0.110: adding an endpoint an existing rule already covers
/// does not fold into that rule, it becomes a rule of its own and the CLI says
/// so on stderr -- `would grant binary '/usr/bin/curl' undeclared authorization
/// for github.com`. That is the right outcome (the binary genuinely was not
/// authorised) and it is why the pane re-reads the policy afterwards instead of
/// reporting what it asked for.
pub fn allow_update(endpoint: &str, binaries: &[String]) -> PolicyUpdate {
    PolicyUpdate {
        add_endpoints: vec![format!("{endpoint}:{ACCESS}:rest:enforce")],
        binaries: binaries.to_vec(),
        rule_name: None,
        wait: true,
        ..Default::default()
    }
}

/// Open routes of an endpoint to a live sandbox, planned against the policy it
/// has now.
///
/// The gateway's `--add-allow` picks the rule it widens by host and port
/// alone, and `--binary` only rides on an `--add-endpoint`, so there are two
/// shapes this can take and a third it cannot:
///
/// * **no rule names the endpoint**: a new one, granted to `binaries` and to
///   nothing on the host but `routes`;
/// * **one rule names it, and already grants every one of `binaries`**: the
///   routes join that rule, for everything it grants. That is the case of a
///   path denied by the rule's own method and path rules, where the binaries
///   on offer were that rule's to begin with;
/// * **anything else is refused.** A new rule beside one that already names
///   the endpoint leaves two, and the gateway will not guess which to widen --
///   measured against 0.0.110, it says "resolves to azure_git, test_dotnet"
///   and asks for the whole YAML. Adding to the one rule instead would hand the
///   paths to binaries nobody named and withhold them from the one that was.
pub fn routes_update(
    policy: &Policy,
    endpoint: &str,
    binaries: &[String],
    routes: &[Route],
) -> Result<PolicyUpdate, String> {
    if routes.is_empty() {
        return Err(format!("no paths named to allow on {endpoint}"));
    }
    let naming: Vec<(&String, &openshell_client::NetworkPolicy)> = policy
        .network_policies
        .iter()
        .filter(|(_, r)| r.endpoints.iter().any(|e| e.host_port() == endpoint))
        .collect();
    let add_allow = routes.iter().map(|r| r.spec(endpoint)).collect();

    match naming.as_slice() {
        [] => {
            if binaries.is_empty() {
                return Err(format!("nothing named to allow {endpoint} for"));
            }
            Ok(PolicyUpdate {
                // No access class: rules alone are default-deny, so the rule
                // grants the routes and nothing else on the host.
                add_endpoints: vec![format!("{endpoint}::rest:enforce")],
                add_allow,
                binaries: binaries.to_vec(),
                rule_name: None,
                wait: true,
                ..Default::default()
            })
        }
        [(key, rule)] => {
            let granted: Vec<&str> = rule.binaries.iter().map(|b| b.path.as_str()).collect();
            let missing: Vec<&str> = binaries
                .iter()
                .map(String::as_str)
                .filter(|b| !granted.contains(b))
                .collect();
            if !missing.is_empty() {
                return Err(format!(
                    "{endpoint} is already granted to {} by `{key}`, and the gateway \
                     can only add paths to that rule, for those binaries; {} would \
                     need the whole host, or the policy replaced",
                    or_nothing(&granted),
                    missing.join(", ")
                ));
            }
            Ok(PolicyUpdate {
                add_allow,
                wait: true,
                ..Default::default()
            })
        }
        several => Err(format!(
            "{endpoint} is named by {} rules ({}), and the gateway cannot tell \
             which one to add paths to",
            several.len(),
            several
                .iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn or_nothing(binaries: &[&str]) -> String {
    if binaries.is_empty() {
        "no binary".to_string()
    } else {
        binaries.join(", ")
    }
}

fn same_set(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().all(|x| b.contains(x))
}

/// Remove an endpoint from a live sandbox.
///
/// A no-op when the endpoint is not there, verified with `policy update
/// --dry-run` against 0.0.110: the CLI exits zero and the merged policy is
/// unchanged. So this never has to be guarded by a read, and blocking something
/// that was never reachable costs a round trip and says so.
pub fn block_update(endpoint: &str) -> PolicyUpdate {
    PolicyUpdate {
        remove_endpoints: vec![endpoint.to_string()],
        wait: true,
        ..Default::default()
    }
}

/// Read the lists, change them, and write them back, with the file locked
/// throughout. The reasoning is [`crate::store::update`]'s, verbatim: more than
/// one writer is the normal case -- a TUI on one screen, a `hura new` on another
/// -- and load-modify-save without a lock loses whichever write lands second.
///
/// Takes its paths rather than deriving them from [`Lists::default_path`], so a
/// caller under test writes to a temporary file instead of to the developer's
/// own configuration. The TUI holds the real pair in `App::lists_path`.
pub fn update_at<T>(
    lists: impl Into<PathBuf>,
    lock: impl Into<PathBuf>,
    f: impl FnOnce(&mut Lists) -> T,
) -> io::Result<T> {
    let lists = lists.into();
    let lock = lock.into();
    if let Some(dir) = lock.parent() {
        fs::create_dir_all(dir)?;
    }
    let guard = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock)?;
    guard.lock()?;

    let result = (|| {
        let mut l = Lists::load_from(&lists)?;
        let out = f(&mut l);
        l.save_to(&lists)?;
        Ok(out)
    })();

    // Explicit rather than left to the drop, so the order is visible: the write
    // above has to be inside the lock.
    let _ = guard.unlock();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lists(allow: &[(&str, &[&str])], block: &[&str]) -> Lists {
        let mut l = Lists::default();
        for (e, bins) in allow {
            l.allow(e, bins.iter().map(|b| (*b).to_string()).collect());
        }
        for e in block {
            l.block(e);
        }
        l
    }

    /// An endpoint on both lists would make the answer depend on which is read
    /// first, so putting it on one takes it off the other.
    #[test]
    fn the_two_lists_never_hold_the_same_endpoint() {
        let mut l = lists(&[("pypi.org:443", &["/usr/local/bin/uv"])], &[]);
        assert_eq!(l.verdict("pypi.org:443"), Some(Listed::Allowed));

        l.block("pypi.org:443");
        assert_eq!(l.verdict("pypi.org:443"), Some(Listed::Blocked));
        assert!(l.allow.is_empty(), "the allow entry is gone, not shadowed");

        l.allow("pypi.org:443", vec!["/usr/bin/node".into()]);
        assert_eq!(l.verdict("pypi.org:443"), Some(Listed::Allowed));
        assert!(l.block.is_empty());
        assert_eq!(l.verdict("nothing.example.com:443"), None);
    }

    /// Pressing the key twice against two denials of the same endpoint must
    /// leave the list saying what the second one said, not the union: merging
    /// grows a rule nobody wrote.
    #[test]
    fn allowing_the_same_endpoint_twice_replaces_rather_than_merges() {
        let l = lists(
            &[
                ("pastebin.com:443", &["/usr/bin/curl"]),
                ("pastebin.com:443", &["/usr/bin/wget"]),
            ],
            &[],
        );
        assert_eq!(l.allow.len(), 1);
        assert_eq!(l.allow[0].binaries, ["/usr/bin/wget"]);
    }

    /// Adding and removing in one call is what keeps a create paying for one
    /// six-second `--wait` rather than two.
    #[test]
    fn one_update_carries_every_endpoint_that_shares_a_binary_list() {
        let l = lists(
            &[
                ("pypi.org:443", &["/usr/local/bin/uv"]),
                ("files.pythonhosted.org:443", &["/usr/local/bin/uv"]),
            ],
            &["platform.claude.com:443"],
        );
        let updates = l.updates();
        assert_eq!(updates.len(), 1, "one binary list, one call");
        assert_eq!(
            updates[0].add_endpoints,
            [
                "files.pythonhosted.org:443:full:rest:enforce",
                "pypi.org:443:full:rest:enforce"
            ],
            "sorted, so the plan is the same every time it is computed"
        );
        assert_eq!(updates[0].binaries, ["/usr/local/bin/uv"]);
        assert_eq!(updates[0].remove_endpoints, ["platform.claude.com:443"]);
        assert!(updates[0].wait, "the clone runs straight after");
    }

    /// `--binary` applies to every `--add-endpoint` in the invocation, so two
    /// entries written against two different denials cannot share a call
    /// without each one gaining the other's binaries.
    #[test]
    fn endpoints_with_different_binaries_get_a_call_each() {
        let l = lists(
            &[
                ("pypi.org:443", &["/usr/local/bin/uv"]),
                ("registry.npmjs.org:443", &["/usr/bin/node"]),
            ],
            &[],
        );
        let updates = l.updates();
        assert_eq!(updates.len(), 2);
        let bins: Vec<&Vec<String>> = updates.iter().map(|u| &u.binaries).collect();
        assert!(bins.contains(&&vec!["/usr/bin/node".to_string()]));
        assert!(bins.contains(&&vec!["/usr/local/bin/uv".to_string()]));
        // Exactly one of them carries the removals, or a create would send them
        // twice.
        assert_eq!(
            updates
                .iter()
                .filter(|u| !u.remove_endpoints.is_empty())
                .count(),
            0,
            "nothing to remove here"
        );
    }

    /// A block list on its own still has to produce a call, or the endpoints a
    /// template grants would stay granted.
    #[test]
    fn a_block_list_alone_still_produces_an_update() {
        let l = lists(&[], &["platform.claude.com:443", "api.github.com:443"]);
        let updates = l.updates();
        assert_eq!(updates.len(), 1);
        assert!(updates[0].add_endpoints.is_empty());
        assert_eq!(
            updates[0].remove_endpoints,
            ["api.github.com:443", "platform.claude.com:443"]
        );
    }

    #[test]
    fn forgetting_takes_an_endpoint_off_either_list() {
        let mut l = lists(
            &[("pypi.org:443", &["/usr/local/bin/uv"])],
            &["x.example.com:443"],
        );
        assert!(l.forget("pypi.org:443"));
        assert!(l.forget("x.example.com:443"));
        assert!(l.is_empty());
        assert!(!l.forget("pypi.org:443"), "nothing left to forget");
    }

    #[test]
    fn empty_lists_ask_the_gateway_for_nothing() {
        assert!(Lists::default().updates().is_empty());
        assert!(Lists::default().is_empty());
    }

    /// The file is the artifact; it has to survive a round trip, and a missing
    /// one has to read as empty rather than as an error.
    #[test]
    fn the_file_round_trips_and_a_missing_one_is_empty() {
        let dir = std::env::temp_dir().join(format!("hura-endpoints-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("endpoints.json");
        let lock = dir.join("endpoints.lock");

        assert_eq!(Lists::load_from(&path).unwrap(), Lists::default());

        update_at(&path, &lock, |l| {
            l.allow("pastebin.com:443", vec!["/usr/bin/curl".into()]);
            l.block("platform.claude.com:443");
        })
        .unwrap();

        let read = Lists::load_from(&path).unwrap();
        assert_eq!(read.verdict("pastebin.com:443"), Some(Listed::Allowed));
        assert_eq!(
            read.verdict("platform.claude.com:443"),
            Some(Listed::Blocked)
        );
        assert_eq!(read.allow[0].binaries, ["/usr/bin/curl"]);

        // A second writer sees the first one's work rather than clobbering it.
        update_at(&path, &lock, |l| l.block("pypi.org:443")).unwrap();
        let read = Lists::load_from(&path).unwrap();
        assert_eq!(read.block.len(), 2);
        assert_eq!(read.allow.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    fn route(method: &str, path: &str) -> Route {
        Route::checked(method, path).unwrap()
    }

    fn policy(rules: &[(&str, &str, &[&str])]) -> Policy {
        let mut p = Policy::default();
        for (key, endpoint, bins) in rules {
            let (host, port) = endpoint.rsplit_once(':').unwrap();
            p.network_policies.insert(
                key.to_string(),
                openshell_client::NetworkPolicy {
                    name: None,
                    endpoints: vec![openshell_client::Endpoint {
                        host: host.into(),
                        port: port.parse().unwrap(),
                        ..Default::default()
                    }],
                    binaries: bins
                        .iter()
                        .map(|b| openshell_client::Binary { path: (*b).into() })
                        .collect(),
                },
            );
        }
        p
    }

    const DOTNET: &str = "/usr/share/dotnet/dotnet";
    const FEED: &str = "/contoso/tools/_packaging/feed/nuget/v3/**";

    /// The case this exists for: a feed on a host nothing grants yet, opened to
    /// the binary that was refused and to nothing on the host but the feed.
    #[test]
    fn routes_on_a_new_endpoint_make_a_rule_with_no_access_class() {
        let u = routes_update(
            &policy(&[("github_git", "github.com:443", &["/usr/bin/git"])]),
            "pkgs.dev.azure.com:443",
            &[DOTNET.into()],
            &[route("GET", FEED)],
        )
        .unwrap();
        // An empty access field, so the rule is rules-only, which is
        // default-deny: `full` here would make the route decoration.
        assert_eq!(u.add_endpoints, ["pkgs.dev.azure.com:443::rest:enforce"]);
        assert_eq!(u.add_allow, [format!("pkgs.dev.azure.com:443:GET:{FEED}")]);
        assert_eq!(u.binaries, [DOTNET]);
        assert!(u.wait);
    }

    /// A second path on the same feed, or a path the rule's own method rules
    /// refused: the rule that names the endpoint takes it, and `--binary` is
    /// not sent, because the gateway only accepts it beside `--add-endpoint`.
    #[test]
    fn routes_join_the_one_rule_that_already_grants_the_binaries() {
        let p = policy(&[(
            "allow_pkgs_dev_azure_com_443",
            "pkgs.dev.azure.com:443",
            &[DOTNET],
        )]);
        let u = routes_update(
            &p,
            "pkgs.dev.azure.com:443",
            &[DOTNET.into()],
            &[route("GET", "/contoso/_packaging/**")],
        )
        .unwrap();
        assert!(u.add_endpoints.is_empty(), "{:?}", u.add_endpoints);
        assert!(u.binaries.is_empty(), "{:?}", u.binaries);
        assert_eq!(
            u.add_allow,
            ["pkgs.dev.azure.com:443:GET:/contoso/_packaging/**"]
        );
    }

    /// The two shapes `--add-allow` cannot express. Both are refused here, with
    /// the rule named, rather than sent and failing at the gateway or, worse,
    /// landing on a rule for binaries nobody asked for.
    #[test]
    fn routes_are_refused_where_the_gateway_would_have_to_guess() {
        let one = policy(&[(
            "azure_git",
            "dev.azure.com:443",
            &["/usr/bin/git", "/usr/bin/curl"],
        )]);
        let e = routes_update(
            &one,
            "dev.azure.com:443",
            &[DOTNET.into()],
            &[route("GET", "/x/**")],
        )
        .unwrap_err();
        assert!(e.contains("azure_git"), "{e}");
        assert!(e.contains(DOTNET), "{e}");

        let two = policy(&[
            ("azure_git", "dev.azure.com:443", &["/usr/bin/git"]),
            ("allow_dev_azure_com_443", "dev.azure.com:443", &[DOTNET]),
        ]);
        let e = routes_update(
            &two,
            "dev.azure.com:443",
            &[DOTNET.into()],
            &[route("GET", "/x/**")],
        )
        .unwrap_err();
        assert!(e.contains("2 rules"), "{e}");

        // And nothing to grant is nothing to send.
        assert!(
            routes_update(
                &Policy::default(),
                "a.example.com:443",
                &[],
                &[route("GET", "/")]
            )
            .is_err()
        );
        assert!(
            routes_update(
                &Policy::default(),
                "a.example.com:443",
                &[DOTNET.into()],
                &[]
            )
            .is_err()
        );
    }

    /// What the gateway would refuse, refused before it is asked, and what it
    /// would read differently, normalised.
    #[test]
    fn a_route_is_a_method_and_a_path_or_nothing() {
        assert_eq!(
            route("get", " /a/** "),
            Route {
                method: "GET".into(),
                path: "/a/**".into()
            }
        );
        assert_eq!(route("*", "/").method, "*");
        for (method, path) in [
            ("G3T", "/a"),
            ("", "/a"),
            ("GET", "a/b"),
            ("GET", "https://pkgs.dev.azure.com/a"),
            ("GET", "/a b"),
            ("GET", "/a:b"),
        ] {
            assert!(Route::checked(method, path).is_err(), "{method} {path}");
        }
    }

    /// Paths on one feed are asked for one denial at a time, and all of them
    /// are needed, so the list keeps every one. Different binaries replace,
    /// like an allow does; an allow of the whole host replaces both ways.
    #[test]
    fn routes_accumulate_for_the_same_binaries_and_replace_otherwise() {
        let mut l = Lists::default();
        l.allow_routes(
            "pkgs.dev.azure.com:443",
            vec![DOTNET.into()],
            vec![route("GET", FEED)],
        );
        l.allow_routes(
            "pkgs.dev.azure.com:443",
            vec![DOTNET.into()],
            vec![route("GET", "/contoso/_packaging/**"), route("GET", FEED)],
        );
        assert_eq!(l.routes.len(), 1);
        assert_eq!(l.routes[0].routes.len(), 2, "kept, and not twice");
        assert_eq!(l.verdict("pkgs.dev.azure.com:443"), Some(Listed::Allowed));

        l.allow_routes(
            "pkgs.dev.azure.com:443",
            vec!["/usr/bin/curl".into()],
            vec![route("GET", "/x")],
        );
        assert_eq!(l.routes[0].routes, [route("GET", "/x")]);

        l.allow("pkgs.dev.azure.com:443", vec![DOTNET.into()]);
        assert!(l.routes.is_empty(), "the whole host replaces its paths");
        l.allow_routes(
            "pkgs.dev.azure.com:443",
            vec![DOTNET.into()],
            vec![route("GET", FEED)],
        );
        assert!(l.allow.is_empty(), "and its paths replace the whole host");

        l.block("pkgs.dev.azure.com:443");
        assert!(l.routes.is_empty());
        assert_eq!(l.verdict("pkgs.dev.azure.com:443"), Some(Listed::Blocked));
    }

    /// Routes are planned against the policy at create, so the plain updates
    /// must not also open their endpoints to the whole host.
    #[test]
    fn routed_entries_are_not_in_the_plain_updates() {
        let mut l = Lists::default();
        l.allow_routes(
            "pkgs.dev.azure.com:443",
            vec![DOTNET.into()],
            vec![route("GET", FEED)],
        );
        assert!(!l.is_empty());
        assert!(l.updates().is_empty(), "{:?}", l.updates());
        assert!(l.forget("pkgs.dev.azure.com:443"));
        assert!(l.is_empty());
    }

    /// A build from before routes reads this file too. Lists with no routes
    /// are written exactly as they were, and a file with routes keeps them
    /// under a key of their own, which an older build skips rather than taking
    /// as an allow of the whole host.
    #[test]
    fn routes_live_under_a_key_an_older_build_skips() {
        let mut l = lists(&[("pypi.org:443", &["/usr/local/bin/uv"])], &[]);
        let plain = serde_json::to_value(&l).unwrap();
        assert!(plain.get("routes").is_none(), "{plain}");

        l.allow_routes(
            "pkgs.dev.azure.com:443",
            vec![DOTNET.into()],
            vec![route("GET", FEED)],
        );
        let json = serde_json::to_value(&l).unwrap();
        assert_eq!(json["allow"].as_array().unwrap().len(), 1, "{json}");
        assert_eq!(json["routes"][0]["endpoint"], "pkgs.dev.azure.com:443");
        assert_eq!(serde_json::from_value::<Lists>(json).unwrap(), l);
    }

    /// A file written by a future hura that has grown a key must not stop this
    /// one from starting -- and a file missing a key it has since gained must
    /// read as an empty list rather than failing.
    #[test]
    fn a_half_written_file_still_reads() {
        let dir =
            std::env::temp_dir().join(format!("hura-endpoints-partial-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("endpoints.json");

        fs::write(&path, r#"{"block":["pypi.org:443"]}"#).unwrap();
        let l = Lists::load_from(&path).unwrap();
        assert!(l.allow.is_empty());
        assert_eq!(l.verdict("pypi.org:443"), Some(Listed::Blocked));

        let _ = fs::remove_dir_all(&dir);
    }
}
