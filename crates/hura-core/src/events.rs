//! The allow/deny feed.
//!
//! The sandbox runtime's proxy logs every decision it makes about a session's
//! traffic. "The agent tried to reach pastebin.com and was denied", as a live
//! event, is the thing this tool can show that claude-squad structurally
//! cannot, so it gets a pane.
//!
//! The runtime counts rather than lists: one row per host and rule since its
//! daemon started, with when it was first and last seen and how often. Each row
//! is one [`Event`] here, at its first sighting, and a row seen again updates
//! the event's count rather than adding another; see [`merge_kept`].
//!
//! Kept on disk per session, because the runtime's log starts again every time
//! its daemon does and is no record; see [`merge_kept`].

use std::fs;
use std::path::{Path, PathBuf};

/// Verdict of a policy decision.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Verdict {
    Allowed,
    Denied,
    /// A lifecycle or configuration event, which decides nothing.
    Neutral,
}

/// Severity as OpenShell graded it. The sandbox runtime grades nothing, so its
/// events are all `Info`; the rest are for events kept from before, and
/// anything above `Info` is still worth colouring.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Severity {
    Info,
    Medium,
    High,
    Critical,
    Other,
}

impl Severity {
    pub fn is_notable(self) -> bool {
        self > Severity::Info
    }
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Event {
    /// Epoch seconds. The runtime writes fractional seconds; the fraction is
    /// dropped because the feed shows a wall-clock time, not a duration.
    // `number`, not the `bigint` ts-rs assumes for a u64: serde_json writes it
    // as a JSON number and `JSON.parse` reads one back, so `bigint` would be a
    // type the runtime never produces. Epoch seconds are exact in a double
    // until the year 285000000.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub at: u64,
    /// How many times it has happened, which the runtime counts. One for an
    /// event kept before it counted.
    #[serde(default = "one")]
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub count: u64,
    /// When it last happened, as epoch seconds; `at` for one seen once.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub last: u64,
    /// `NET:OPEN`, `HTTP:GET`, `CONFIG:VALIDATED`.
    pub class: String,
    pub severity: Severity,
    pub verdict: Verdict,
    /// What the event is about: `curl(79) -> pastebin.com:443`.
    pub subject: String,
    /// The rule that decided, when one did. `-` in the log means none matched,
    /// and is normalised to `None`.
    pub policy: Option<String>,
    pub reason: Option<String>,
}

fn one() -> u64 {
    1
}

/// How many events to keep per session on disk.
///
/// Enough to be a record of a session's afternoon; small enough that reading it
/// back on every fetch stays free. Trimmed oldest-first.
const KEPT: usize = 4000;

/// Held by [`merge_kept`]. One for every session rather than one each: a merge
/// is a file read and a write, and nothing waits on another long enough to be
/// worth a map of locks.
static MERGING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Where a session's kept events live.
fn kept_path(session: &str) -> PathBuf {
    // Beside the session cache, under a directory of its own so a session name
    // can never collide with `sessions.json`.
    crate::store::Store::default_path()
        .with_file_name("events")
        .join(format!("{session}.jsonl"))
}

/// Add what was just fetched to what was already known, and keep the result.
///
/// The runtime's log starts again every time its daemon does, so the feed is
/// ours to keep. Each fetch is merged into a file per session and trimmed; the
/// pane draws the union. An event already kept takes the fetched one's count
/// and last sighting, since the runtime counts on in the same row. Losing the
/// file costs the history and nothing else, like the session cache beside it.
pub fn merge_kept(session: &str, fetched: Vec<Event>) -> Vec<Event> {
    // Across the read and the write. `hurad` merges from more than one thread,
    // its own collector and whichever pane is open, and two merges interleaved
    // lose the additions of whichever wrote first.
    let _held = MERGING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = kept_path(session);
    let mut all = read_kept(&path);
    let mut known: std::collections::HashMap<(u64, String, String), usize> = all
        .iter()
        .enumerate()
        .map(|(i, e)| (identity(e), i))
        .collect();

    let mut changed = false;
    for e in fetched {
        // Looked up as they are taken, because one fetch can carry the same
        // event twice, and a map that only knows the file would keep both.
        match known.get(&identity(&e)) {
            Some(&i) => {
                let kept = &mut all[i];
                if e.count > kept.count || e.last > kept.last {
                    kept.count = kept.count.max(e.count);
                    kept.last = kept.last.max(e.last);
                    kept.reason = e.reason.or(kept.reason.take());
                    changed = true;
                }
            }
            None => {
                known.insert(identity(&e), all.len());
                all.push(e);
                changed = true;
            }
        }
    }
    if !changed && !all.is_empty() {
        return newest_first(all);
    }

    // Oldest first while trimming, so the tail that survives is the newest.
    all.sort_by_key(|e| e.at);
    if all.len() > KEPT {
        all.drain(..all.len() - KEPT);
    }
    if let Err(e) = write_kept(&path, &all) {
        // A feed that cannot be persisted is still a feed; the pane shows what
        // was fetched and the next attempt may work.
        eprintln!("hura: could not keep events for {session}: {e}");
    }
    newest_first(all)
}

/// What makes two events the same event: when it was first seen, and what it
/// was about.
fn identity(e: &Event) -> (u64, String, String) {
    e.key()
}

fn newest_first(mut events: Vec<Event>) -> Vec<Event> {
    events.sort_by_key(|e| std::cmp::Reverse(e.at));
    events
}

fn read_kept(path: &Path) -> Vec<Event> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    // A line that will not parse is skipped rather than failing the read: this
    // is a cache, and half a history is better than none.
    text.lines()
        .filter_map(|l| serde_json::from_str::<Event>(l).ok())
        .map(|mut e| {
            e.subject = mended(&e.subject).to_string();
            e
        })
        .collect()
}

/// A subject as it was meant to be read, from one kept before the parser
/// counted nested brackets.
///
/// Those were cut at the first `]` inside a reason, and what followed became
/// more of the subject: `/usr/bin/curl(8898) -> api.github.com:443  , cmdline:
/// ). S...]`. They are on disk, and the denials they record have long left
/// OpenShell's window, so mending them as they are read is the only way they
/// can still be allowed. The tail always begins with whitespace and then the
/// `,` or `]` that followed a nested group, which no subject OpenShell wrote
/// had. Mended, they also matched the same lines read again, so a denial
/// still in the window was not kept twice.
fn mended(subject: &str) -> &str {
    subject
        .char_indices()
        .filter(|(_, c)| c.is_whitespace())
        .find(|(i, _)| {
            let next = subject[*i..].trim_start();
            next.starts_with(',') || next.starts_with(']')
        })
        .map_or(subject, |(i, _)| subject[..i].trim_end())
}

fn write_kept(path: &Path, events: &[Event]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body: Vec<String> = events
        .iter()
        .map(|e| serde_json::to_string(e).expect("an Event is plain data"))
        .collect();
    // Temp file and rename, like the session cache: an interrupted write must not
    // truncate the history.
    let tmp = path.with_extension("jsonl.tmp");
    fs::write(&tmp, body.join("\n"))?;
    fs::rename(&tmp, path)
}

/// Forget a session's kept events. Called when the session is destroyed.
pub fn forget_kept(session: &str) {
    let _ = fs::remove_file(kept_path(session));
}

/// The endpoint an event was about, when it was about one.
///
/// This is what makes the feed actionable rather than only readable: a denial
/// names a host and a port, which is exactly what an allow or a block takes.
/// Everything else in the pane is prose.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Target {
    /// `pastebin.com:443`, the unit an allow and a block both address.
    pub endpoint: String,
}

impl Event {
    /// What this event was about, as an endpoint.
    ///
    /// Three shapes have to be read: the two the runtime's log becomes, and
    /// the one events kept from before it still carry:
    ///
    /// ```text
    /// POST github.com:443/o/r.git/git-receive-pack   a request, by its path
    /// example.com:443                                a connection
    /// /usr/bin/curl(79) -> pastebin.com:443          kept from before
    /// ```
    ///
    /// Anything else -- a `CONFIG:VALIDATED` warning is a whole English
    /// sentence -- is not about an endpoint, and says so rather than being
    /// coerced into one. That is why the L7 arm insists on exactly two words
    /// with an uppercase method first: a sentence ending in something that
    /// happens to parse as `host:port` must not become a policy change.
    pub fn target(&self) -> Option<Target> {
        let subject = self.subject.trim();

        // `/usr/bin/curl(79) -> pastebin.com:443`, and the request with a
        // program in front of it, `/usr/bin/curl(5180) -> GET github.com/`.
        if let Some((_, right)) = subject.split_once(" -> ") {
            let right = request(right).unwrap_or(right);
            return Some(Target {
                endpoint: self.endpoint_of(right)?,
            });
        }

        // `GET httpbin.org:443/ip`
        if let Some(authority) = request(subject) {
            return Some(Target {
                endpoint: self.endpoint_of(authority)?,
            });
        }

        Some(Target {
            endpoint: host_port(subject)?,
        })
    }

    /// `host:port` from an authority, or from the reason when the authority
    /// left the port out.
    ///
    /// A plain-HTTP request is logged as `GET github.com/`, with the port only
    /// in the reason -- `endpoint github.com:80 is not allowed by any policy`.
    /// The reason is only believed when it names the same host, so a sentence
    /// about some other endpoint cannot redirect the change.
    fn endpoint_of(&self, authority: &str) -> Option<String> {
        if let Some(endpoint) = host_port(authority) {
            return Some(endpoint);
        }
        let named = self
            .reason
            .as_deref()?
            .split_whitespace()
            .skip_while(|w| *w != "endpoint")
            .nth(1)?;
        let endpoint = host_port(named)?;
        let host = endpoint.rsplit_once(':')?.0;
        (host == authority).then_some(endpoint)
    }

    /// What makes two events the same event, for anything that has to keep hold
    /// of one across a refetch.
    ///
    /// The feed grows at the top, so a row index is not a handle: three arrivals
    /// between two keystrokes and it points at something else. See the events
    /// pane's cursor.
    pub fn key(&self) -> (u64, String, String) {
        (self.at, self.class.clone(), self.subject.clone())
    }
}

/// The authority of `GET httpbin.org:443/ip`: exactly two words, an uppercase
/// method first, and the path dropped. Anything else is prose.
fn request(s: &str) -> Option<&str> {
    let mut words = s.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (Some(method), Some(rest), None) if method.chars().all(|c| c.is_ascii_uppercase()) => {
            Some(rest.split('/').next().unwrap_or(rest))
        }
        _ => None,
    }
}

/// An event and the endpoint it was about, which is what a client needs to
/// offer an allow or a block beside it.
///
/// A wrapper rather than a field on [`Event`], because events are persisted
/// and the target is derived: kept on disk it would be a second copy of the
/// subject that a better parser could never correct.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FeedEvent {
    #[serde(flatten)]
    pub event: Event,
    pub target: Option<Target>,
}

impl From<Event> for FeedEvent {
    fn from(event: Event) -> Self {
        let target = event.target();
        FeedEvent { event, target }
    }
}

/// `host:port`, normalised, or nothing. What an endpoint from a client has to
/// pass before it becomes a policy change.
pub fn endpoint(s: &str) -> Option<String> {
    host_port(s)
}

/// `host:port`, or nothing.
///
/// Strict on purpose. This decides whether a line in a feed can be turned into
/// a policy change, so a loose match is a change to an endpoint nobody named.
fn host_port(s: &str) -> Option<String> {
    let (host, port) = s.trim().rsplit_once(':')?;
    // A hostname, not a sentence: the dot requirement is what keeps
    // `deprecated: 443` out, and there is no single-label host worth reaching
    // from a sandbox.
    if host.is_empty()
        || !host.contains('.')
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    let port: u16 = port.parse().ok()?;
    (port != 0).then(|| format!("{host}:{port}"))
}

impl Event {
    /// `HH:MM:SS` in local time, for the feed's left column.
    ///
    /// Computed by hand rather than with a date crate: this is the only place
    /// in `hura` that formats a clock time, and the whole binary is otherwise
    /// free of a time-zone database. Uses UTC, and says so in the pane title,
    /// because guessing the offset would be worse than being explicit.
    pub fn clock_utc(&self) -> String {
        let secs = self.at % 86_400;
        format!(
            "{:02}:{:02}:{:02}",
            secs / 3600,
            (secs % 3600) / 60,
            secs % 60
        )
    }
}

/// The decisions in a Docker Sandboxes policy log, as events.
///
/// The runtime counts rather than lists: one row per host and rule since its
/// daemon started, with when it was first and last seen. Each row becomes one
/// event at its first sighting, so a row seen again is the same event. The
/// subject takes a shape [`Event::target`] already reads: `METHOD host:port/path`
/// for a request the proxy judged by its method and path, `host:port` for a
/// connection. Name lookups are left out; every connection already has a row
/// of its own, and the lookups are the half nobody acts on.
pub fn from_sbx(log: &sbx_client::PolicyLog) -> Vec<Event> {
    let rows = log
        .blocked_hosts
        .iter()
        .map(|e| (e, Verdict::Denied))
        .chain(log.allowed_hosts.iter().map(|e| (e, Verdict::Allowed)));
    let mut out: Vec<Event> = rows
        .filter(|(e, _)| e.proxy_type != "network" && !e.host.ends_with(".docker.internal"))
        .filter_map(|(e, verdict)| {
            let at = epoch_of(&e.since)?;
            let request = requested(&e.rule);
            let (class, subject) = match &request {
                Some((method, path)) => (
                    format!("HTTP:{method}"),
                    format!("{method} {}{path}", e.host),
                ),
                None => ("NET:OPEN".to_string(), e.host.clone()),
            };
            Some(Event {
                at,
                count: e.count_since.max(1),
                last: epoch_of(&e.last_seen).unwrap_or(at).max(at),
                class,
                severity: Severity::Info,
                verdict,
                subject,
                policy: deciding_rule(&e.rule),
                reason: e.reason.clone().filter(|r| !r.is_empty()),
            })
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.at));
    out
}

/// The method and path out of a judged request, from the operation the log
/// names: `op(action=http:request:post, ..., http:path:/o/r.git/git-receive-pack])`.
fn requested(rule: &str) -> Option<(String, String)> {
    let action = rule.split("action=http:request:").nth(1)?;
    let method: String = action
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    if method.is_empty() {
        return None;
    }
    let path = rule.split("http:path:").nth(1)?;
    let path: String = path
        .chars()
        .take_while(|c| !matches!(c, ']' | ','))
        .collect();
    path.starts_with('/')
        .then(|| (method.to_ascii_uppercase(), path))
}

/// The rule that decided, when one did: `local:ID` out of
/// `denied: rule "local:ID" matched op(...)`. A refusal because nothing matched
/// names no rule, which is `None` here as `-` was in OpenShell's log.
fn deciding_rule(rule: &str) -> Option<String> {
    let quoted = rule.split("rule \"").nth(1)?;
    let id = quoted.split('"').next()?;
    (!id.is_empty()).then(|| id.to_string())
}

/// Epoch seconds from an RFC 3339 time, as the runtime writes them:
/// `2026-10-08T11:34:44.903534058+02:00` or with a `Z`. The fraction is
/// dropped, for the reason [`Event::at`] gives.
fn epoch_of(stamp: &str) -> Option<u64> {
    let (date, rest) = stamp.split_once('T')?;
    let mut ymd = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (ymd.next()??, ymd.next()??, ymd.next()??);
    let (clock, offset) = if let Some(c) = rest.strip_suffix('Z') {
        (c, 0)
    } else {
        let at = rest.rfind(['+', '-'])?;
        let (c, off) = rest.split_at(at);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let (oh, om) = off[1..].split_once(':')?;
        (
            c,
            sign * (oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60),
        )
    };
    let clock = clock.split('.').next()?;
    let mut hms = clock.splitn(3, ':').map(|p| p.parse::<i64>().ok());
    let (hh, mm, ss) = (hms.next()??, hms.next()??, hms.next()??);
    // Days since the epoch from a civil date: Howard Hinnant's algorithm.
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hh * 3600 + mm * 60 + ss - offset;
    u64::try_from(secs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(at: u64, subject: &str) -> Event {
        Event {
            at,
            count: 1,
            last: at,
            class: "NET:OPEN".into(),
            severity: Severity::Info,
            verdict: Verdict::Allowed,
            subject: subject.into(),
            policy: None,
            reason: None,
        }
    }

    /// `XDG_CONFIG_HOME` is process-wide, so tests that point it somewhere else
    /// cannot run beside each other: one would move the kept file out from under
    /// another mid-test.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A directory of its own, so the kept file can be exercised for real.
    struct Home {
        dir: PathBuf,
        previous: Option<std::ffi::OsString>,
        /// Held for the test's lifetime, not read: dropping it is the point.
        _serialised: std::sync::MutexGuard<'static, ()>,
    }

    impl Home {
        fn new(tag: &str) -> Self {
            // A poisoned lock means another test already failed; taking it anyway
            // keeps this one's failure its own rather than a second symptom.
            let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
            let dir = std::env::temp_dir().join(format!("hura-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let previous = std::env::var_os("XDG_CONFIG_HOME");
            // `kept_path` is derived from the same place the session cache is, so
            // pointing that at a temporary directory is what makes this testable.
            unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
            Home {
                dir,
                previous,
                _serialised: guard,
            }
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            match &self.previous {
                Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
                None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
            }
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// The feed has to survive the runtime's daemon starting again, which
    /// takes its log with it.
    #[test]
    fn kept_events_outlive_the_log_they_came_from() {
        let _home = Home::new("events-keep");

        let first = merge_kept(
            "s",
            vec![ev(100, "a.example.com:443"), ev(200, "b.example.com:443")],
        );
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].at, 200, "newest first, like a feed");

        // Later, the log has started again: only one of them is still in it,
        // plus one that has happened since.
        let second = merge_kept(
            "s",
            vec![ev(200, "b.example.com:443"), ev(300, "c.example.com:443")],
        );
        let times: Vec<u64> = second.iter().map(|e| e.at).collect();
        assert_eq!(times, vec![300, 200, 100], "the old one is still there");

        // And a fetch that returns nothing at all must not empty the feed.
        assert_eq!(merge_kept("s", vec![]).len(), 3);
    }

    /// The runtime counts on in the same row, so the kept event takes the new
    /// count and last sighting rather than a second row appearing.
    #[test]
    fn a_counted_event_is_updated_rather_than_kept_twice() {
        let _home = Home::new("events-count");
        merge_kept("s", vec![ev(100, "a.example.com:443")]);
        let mut again = ev(100, "a.example.com:443");
        again.count = 4;
        again.last = 160;
        let kept = merge_kept("s", vec![again.clone(), again]);
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].count, kept[0].last), (4, 160));

        // An older reading never winds it back.
        let stale = ev(100, "a.example.com:443");
        assert_eq!(merge_kept("s", vec![stale])[0].count, 4);
    }

    /// Events kept before the runtime counted read as seen once.
    #[test]
    fn an_event_kept_before_counts_reads_as_once() {
        let old = r#"{"at":1787568645,"class":"NET:OPEN","severity":"Info","verdict":"Denied","subject":"/usr/bin/curl(79) -> pastebin.com:443","policy":null,"reason":null}"#;
        let e: Event = serde_json::from_str(old).unwrap();
        assert_eq!(e.count, 1);
        assert_eq!(e.target().unwrap().endpoint, "pastebin.com:443");
    }

    #[test]
    fn the_kept_file_is_trimmed_and_forgotten_with_its_session() {
        let _home = Home::new("events-trim");
        let many: Vec<Event> = (0..KEPT as u64 + 50).map(|i| ev(i, "x")).collect();
        let kept = merge_kept("s", many);
        assert_eq!(kept.len(), KEPT, "trimmed to the cap");
        assert_eq!(
            kept[0].at,
            KEPT as u64 + 49,
            "and it is the newest that stay"
        );

        forget_kept("s");
        assert!(merge_kept("s", vec![]).is_empty(), "nothing is left");
    }

    /// Subjects kept before the parser counted brackets come back readable,
    /// and so allowable. Taken from a real kept feed.
    #[test]
    fn kept_subjects_from_the_old_parser_are_mended() {
        for (kept, meant) in [
            (
                "/usr/bin/curl(8898) -> api.github.com:443  , cmdline: ). S...]",
                "/usr/bin/curl(8898) -> api.github.com:443",
            ),
            (
                "/usr/bin/curl(5422) -> api.nuget.org:443  , cmdline:",
                "/usr/bin/curl(5422) -> api.nuget.org:443",
            ),
            ("api.anthropic.com:443 ]", "api.anthropic.com:443"),
        ] {
            assert_eq!(mended(kept), meant);
        }
        // What OpenShell wrote is left exactly as it was.
        for fine in [
            "/usr/bin/curl(79) -> pastebin.com:443",
            "GET httpbin.org:443/ip",
            "ssh relay closed (channel_id=a, target=unix:/run/openshell/ssh.sock)",
        ] {
            assert_eq!(mended(fine), fine);
        }
    }

    #[test]
    fn formats_a_clock_time() {
        // 1787568645 is 10:50:45 UTC; only the time of day is shown.
        let e = ev(1_787_568_645, "x");
        assert_eq!(e.clock_utc(), "10:50:45");
        // Midnight must render as 00:00:00 rather than 24:00:00.
        assert_eq!(ev(1_787_529_600, "x").clock_utc(), "00:00:00");
    }

    /// The feed is only actionable if a line can be turned back into the
    /// endpoint it was about, in every shape it comes in.
    #[test]
    fn a_decision_yields_the_endpoint_it_was_about() {
        for (subject, endpoint) in [
            (
                "POST github.com:443/o/r.git/git-receive-pack",
                "github.com:443",
            ),
            ("example.com:443", "example.com:443"),
            ("/usr/bin/curl(79) -> pastebin.com:443", "pastebin.com:443"),
            ("/usr/bin/node(7) -> GET docs.rs:443/tokio", "docs.rs:443"),
        ] {
            assert_eq!(
                ev(1, subject).target().unwrap().endpoint,
                endpoint,
                "{subject}"
            );
        }
    }

    /// A plain-HTTP request may carry no port in its authority; the port is in
    /// the reason, which is only believed about the same host.
    #[test]
    fn a_request_takes_its_port_from_the_reason() {
        let mut e = ev(1, "GET github.com/");
        e.reason = Some("endpoint github.com:80 is not allowed by any policy".into());
        assert_eq!(e.target().unwrap().endpoint, "github.com:80");
        e.reason = Some("endpoint pastebin.com:80 is not allowed by any policy".into());
        assert_eq!(e.target(), None);
    }

    /// The whole risk of this: a subject that is prose must not become a
    /// policy change, and there is one click between a match here and a rule.
    #[test]
    fn prose_is_never_mistaken_for_an_endpoint() {
        for subject in [
            "",
            "sleep(56)",
            "applied policy revision",
            "L7 policy validation warning: 'tls: terminate' is deprecated",
            // A single-label host: nothing worth reaching from a sandbox, and
            // allowing it would be allowing a word.
            "localhost:443",
            "pastebin.com:https",
            "pastebin.com:0",
            "pastebin.com:99999",
            // A lowercase first word is not an HTTP method, so this is prose.
            "get httpbin.org:443/ip",
            "denied reaching pastebin.com:443",
        ] {
            assert_eq!(ev(1, subject).target(), None, "{subject:?}");
        }
    }

    /// The pane holds on to a selected event across a refetch, and the feed
    /// grows at the top, so the handle cannot be a row index.
    #[test]
    fn the_key_identifies_an_event_across_a_refetch() {
        let a = ev(100, "a.example.com:443");
        let b = ev(100, "b.example.com:443");
        assert_ne!(a.key(), b.key(), "same second, different subject");
        assert_eq!(a.key(), a.clone().key());
        // The same notion of sameness the kept file merges on, or the cursor
        // would follow an event the merge had folded away.
        assert_eq!(a.key(), identity(&a));
    }

    #[test]
    fn severity_orders_so_notable_means_above_info() {
        assert!(Severity::Medium.is_notable());
        assert!(Severity::Critical.is_notable());
        assert!(!Severity::Info.is_notable());
    }

    /// The log the runtime keeps, read into the feed's own events: a refused
    /// request by its method and path, a refused connection by its endpoint,
    /// and the lookups left out.
    #[test]
    fn the_runtime_log_reads_as_events() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sbx-client/tests/fixtures/policy-log.json"
        ))
        .unwrap();
        let log: sbx_client::PolicyLog = serde_json::from_str(&text).unwrap();
        let events = from_sbx(&log);
        assert!(
            events
                .iter()
                .all(|e| !e.subject.contains(".docker.internal"))
        );

        let push = events
            .iter()
            .find(|e| e.class == "HTTP:POST" && e.subject.contains("git-receive-pack"))
            .expect("the refused push");
        assert_eq!(push.verdict, Verdict::Denied);
        assert_eq!(
            push.subject,
            "POST github.com:443/octocat/Hello-World.git/git-receive-pack"
        );
        assert_eq!(
            push.policy.as_deref(),
            Some("local:148582d1-e89d-4118-9083-63ab6db02a4a")
        );
        assert_eq!(push.target().unwrap().endpoint, "github.com:443");
        assert!(push.count >= 1 && push.last >= push.at);

        let refused = events
            .iter()
            .find(|e| e.subject == "example.com:443")
            .expect("the refused connection");
        assert_eq!(refused.class, "NET:OPEN");
        assert_eq!(refused.policy, None);
        assert_eq!(refused.target().unwrap().endpoint, "example.com:443");

        assert!(events.iter().any(|e| e.verdict == Verdict::Allowed));
        assert!(
            events.windows(2).all(|w| w[0].at >= w[1].at),
            "newest first"
        );
    }

    #[test]
    fn runtime_timestamps_are_read_in_their_own_offset() {
        assert_eq!(epoch_of("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch_of("2026-10-08T09:34:44Z"), Some(1_791_452_084));
        assert_eq!(
            epoch_of("2026-10-08T11:34:44.903534058+02:00"),
            Some(1_791_452_084)
        );
        assert_eq!(epoch_of("2026-10-08T04:34:44-05:00"), Some(1_791_452_084));
        assert_eq!(epoch_of("yesterday"), None);
    }
}
