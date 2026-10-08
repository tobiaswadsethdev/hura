//! The global allow and block lists.
//!
//! `w` and `t` change one session's egress and forget it. These are the same
//! decision made once: an endpoint on the allow list is opened on every new
//! session, and one on the block list is denied on every new session,
//! including endpoints the policy template itself opens.
//!
//! A block is a deny, which outranks every allow on the sandbox, so blocking
//! an endpoint the template opens really closes it, and blocking one nothing
//! opens says so in advance.
//!
//! `$XDG_CONFIG_HOME/hura/endpoints.json`, beside the session cache, and a cache
//! in the same sense: losing it costs the lists and nothing else. Written under
//! a lock of its own for the reason [`crate::store`] holds one: a window and a
//! `hurad new` in another terminal are the normal case, not an edge, and
//! load-modify-save without a lock loses whichever write lands second.

use std::fs;
use std::io;
use std::path::PathBuf;

use sbx_client::{Decision, RuleSpec};

use crate::store::Store;

/// An endpoint opened for every new session.
///
/// Files written while the lists named binaries carry a `binaries` key too,
/// which is read past: rules are for the whole sandbox now.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Allow {
    /// `host:port`.
    pub endpoint: String,
}

/// One method and path an allow is narrowed to:
/// `GET /contoso/tools/_packaging/feed/nuget/v3/**`.
///
/// Enforced rather than advisory: the sandbox's proxy inspects every request to
/// a host with rules like this, and anything on it the rules do not name is
/// refused.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Route {
    /// Upper case, or `ANY` for any method.
    pub method: String,
    /// From the first `/`, with `*` and `**` as globs.
    pub path: String,
}

impl Route {
    /// A route as it may go into a rule, or why it may not.
    ///
    /// The runtime's own rules for a path: it starts with `/`, and has no
    /// query, fragment, percent-encoding or `..` segment.
    pub fn checked(method: &str, path: &str) -> Result<Route, String> {
        let method = match method.trim().to_ascii_uppercase().as_str() {
            "*" | "ANY" => "ANY".to_string(),
            m => m.to_string(),
        };
        let is_method = !method.is_empty()
            && method.len() <= 16
            && method.chars().all(|c| c.is_ascii_uppercase());
        if !is_method {
            return Err(format!("`{method}` is not an HTTP method"));
        }
        let path = path.trim();
        if !path.starts_with('/') {
            return Err(format!("`{path}` is not a path: it has to start with /"));
        }
        if let Some(c) = path.chars().find(|c| matches!(c, '?' | '#' | '%')) {
            return Err(format!(
                "`{path}` has a `{c}` in it; a path rule names the path alone"
            ));
        }
        if path.split('/').any(|seg| seg == "..") {
            return Err(format!("`{path}` has a `..` in it"));
        }
        if path.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(format!("`{path}` has a space in it"));
        }
        Ok(Route {
            method,
            path: path.to_string(),
        })
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
    /// `host:port` for each.
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

    /// Put an endpoint on the allow list, taking it off the block list and
    /// replacing an allow of some of its paths.
    ///
    /// The lists are mutually exclusive by construction rather than by rule: an
    /// endpoint on both would make the answer depend on which list is consulted
    /// first, and there is no reading of "allowed and blocked" worth having.
    pub fn allow(&mut self, endpoint: &str) {
        self.block.retain(|e| e != endpoint);
        self.routes.retain(|r| r.endpoint != endpoint);
        if !self.allow.iter().any(|a| a.endpoint == endpoint) {
            self.allow.push(Allow {
                endpoint: endpoint.to_string(),
            });
            self.allow.sort_by(|a, b| a.endpoint.cmp(&b.endpoint));
        }
    }

    /// Put routes of an endpoint on the allow list, taking the endpoint off
    /// the block list and replacing an allow of the whole of it.
    ///
    /// Added to the routes already listed: each was asked for against a denial
    /// of its own, and a feed needs its index, its metadata and its packages
    /// before a restore works.
    pub fn allow_routes(&mut self, endpoint: &str, routes: Vec<Route>) {
        self.block.retain(|e| e != endpoint);
        self.allow.retain(|a| a.endpoint != endpoint);
        let mut routes = routes;
        if let Some(i) = self.routes.iter().position(|r| r.endpoint == endpoint) {
            routes.extend(self.routes.remove(i).routes);
        }
        routes.sort();
        routes.dedup();
        self.routes.push(RouteAllow {
            endpoint: endpoint.to_string(),
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

    /// The rules that impose these lists on a fresh sandbox, allows first.
    pub fn rules(&self) -> Vec<RuleSpec> {
        let mut out: Vec<RuleSpec> = self
            .allow
            .iter()
            .flat_map(|a| allow_rules(&a.endpoint, &[]))
            .collect();
        out.extend(
            self.routes
                .iter()
                .flat_map(|r| allow_rules(&r.endpoint, &r.routes)),
        );
        out.extend(self.block.iter().map(|e| deny_rule(e)));
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

/// The rules that open an endpoint: the whole of it when `routes` is empty,
/// and one rule per path otherwise, each for the methods named on it.
///
/// Shared by the events pane and [`Lists::rules`], so a decision applied to a
/// live session and the same decision applied at the next create cannot drift
/// apart.
pub fn allow_rules(endpoint: &str, routes: &[Route]) -> Vec<RuleSpec> {
    if routes.is_empty() {
        return vec![RuleSpec {
            decision: Decision::Allow,
            resources: vec![endpoint.to_string()],
            methods: Vec::new(),
            path: None,
        }];
    }
    let mut by_path: Vec<(String, Vec<String>)> = Vec::new();
    for r in routes {
        match by_path.iter_mut().find(|(p, _)| *p == r.path) {
            Some((_, methods)) if !methods.contains(&r.method) => methods.push(r.method.clone()),
            Some(_) => {}
            None => by_path.push((r.path.clone(), vec![r.method.clone()])),
        }
    }
    by_path
        .into_iter()
        .map(|(path, methods)| RuleSpec {
            decision: Decision::Allow,
            resources: vec![endpoint.to_string()],
            methods,
            path: Some(path),
        })
        .collect()
}

/// The rule that closes an endpoint, whatever else opens it.
pub fn deny_rule(endpoint: &str) -> RuleSpec {
    RuleSpec {
        decision: Decision::Deny,
        resources: vec![endpoint.to_string()],
        methods: Vec::new(),
        path: None,
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

    fn route(method: &str, path: &str) -> Route {
        Route::checked(method, path).unwrap()
    }

    /// An endpoint on both lists would make the answer depend on which is read
    /// first, so putting it on one takes it off the other.
    #[test]
    fn the_lists_never_hold_the_same_endpoint() {
        let mut l = Lists::default();
        l.allow("pypi.org:443");
        assert_eq!(l.verdict("pypi.org:443"), Some(Listed::Allowed));

        l.block("pypi.org:443");
        assert_eq!(l.verdict("pypi.org:443"), Some(Listed::Blocked));
        assert!(l.allow.is_empty(), "the allow entry is gone, not shadowed");

        l.allow_routes("pypi.org:443", vec![route("GET", "/simple/**")]);
        assert_eq!(l.verdict("pypi.org:443"), Some(Listed::Allowed));
        assert!(l.block.is_empty());
        l.allow("pypi.org:443");
        assert!(l.routes.is_empty(), "the whole host replaces some of it");
        assert_eq!(l.verdict("nothing.example.com:443"), None);
    }

    #[test]
    fn routes_accumulate_for_one_endpoint() {
        let mut l = Lists::default();
        l.allow_routes(
            "pkgs.example.com:443",
            vec![route("GET", "/feed/index.json")],
        );
        l.allow_routes(
            "pkgs.example.com:443",
            vec![
                route("GET", "/feed/v3/**"),
                route("GET", "/feed/index.json"),
            ],
        );
        assert_eq!(l.routes.len(), 1);
        assert_eq!(l.routes[0].routes.len(), 2);
    }

    /// Allows, the routes as one rule per path, then the blocks as denies.
    #[test]
    fn the_lists_become_rules() {
        let mut l = Lists::default();
        l.allow("docs.rs:443");
        l.allow_routes(
            "pkgs.example.com:443",
            vec![
                route("GET", "/feed/**"),
                route("HEAD", "/feed/**"),
                route("POST", "/upload"),
            ],
        );
        l.block("pastebin.com:443");
        let rules = l.rules();
        assert_eq!(rules.len(), 4, "{rules:?}");
        assert_eq!(rules[0], allow_rules("docs.rs:443", &[])[0]);
        let feed = rules
            .iter()
            .find(|r| r.path.as_deref() == Some("/feed/**"))
            .unwrap();
        assert_eq!(feed.methods, ["GET", "HEAD"]);
        assert_eq!(rules[3], deny_rule("pastebin.com:443"));
        assert!(Lists::default().rules().is_empty());
    }

    #[test]
    fn a_route_is_a_method_and_a_path_or_nothing() {
        assert_eq!(route("get", "/a/**").method, "GET");
        assert_eq!(route("*", "/a").method, "ANY");
        assert!(Route::checked("G3T", "/a").is_err());
        assert!(Route::checked("GET", "a/b").is_err());
        assert!(Route::checked("GET", "/a?b=c").is_err());
        assert!(Route::checked("GET", "/a/%2e").is_err());
        assert!(Route::checked("GET", "/a/../b").is_err());
        assert!(Route::checked("GET", "/a b").is_err());
        // A colon was OpenShell's to refuse; the runtime takes it.
        assert!(Route::checked("GET", "/a:b").is_ok());
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
            l.allow("pastebin.com:443");
            l.block("platform.claude.com:443");
        })
        .unwrap();

        let read = Lists::load_from(&path).unwrap();
        assert_eq!(read.verdict("pastebin.com:443"), Some(Listed::Allowed));
        assert_eq!(
            read.verdict("platform.claude.com:443"),
            Some(Listed::Blocked)
        );

        // A second writer sees the first one's work rather than clobbering it.
        update_at(&path, &lock, |l| l.block("pypi.org:443")).unwrap();
        let read = Lists::load_from(&path).unwrap();
        assert_eq!(read.block.len(), 2);
        assert_eq!(read.allow.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    /// A file written while the lists named binaries still reads, keeping its
    /// endpoints and dropping what no longer applies.
    #[test]
    fn a_file_from_before_still_reads() {
        let old = r#"{
          "allow": [{"endpoint": "docs.rs:443", "binaries": ["/usr/bin/curl"]}],
          "routes": [{"endpoint": "pkgs.example.com:443", "binaries": ["/usr/bin/dotnet"],
                      "routes": [{"method": "GET", "path": "/feed/**"}]}],
          "block": ["pastebin.com:443"]
        }"#;
        let l: Lists = serde_json::from_str(old).unwrap();
        assert_eq!(l.allow[0].endpoint, "docs.rs:443");
        assert_eq!(l.routes[0].routes[0].path, "/feed/**");
        assert_eq!(l.verdict("pastebin.com:443"), Some(Listed::Blocked));
    }

    /// A file written by a future hura that has grown a key must not stop this
    /// one from starting, and a file missing a key must read as an empty list.
    #[test]
    fn a_half_written_file_still_reads() {
        let l: Lists = serde_json::from_str(r#"{"block":["pypi.org:443"]}"#).unwrap();
        assert!(l.allow.is_empty());
        assert_eq!(l.verdict("pypi.org:443"), Some(Listed::Blocked));
    }
}
