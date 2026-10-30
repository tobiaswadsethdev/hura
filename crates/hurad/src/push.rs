//! Notifications on a phone, through the hura relay.
//!
//! The relay (`relay/` at the repository root, `docs/relay-api.md`) holds the
//! APNs key. What this server holds is a push token per paired client, which
//! can make that client's phone show a notification and nothing more. A client
//! configures its route with [`hura_proto::Request::PushConfigure`];
//! [`watch`] notices what is worth saying; this module keeps the routes,
//! queues what is to be said, and says it.
//!
//! **A route belongs to a token, not to the server.** It is keyed by the
//! token's hash, which is what [`crate::auth::Tokens`] keeps, so revoking a
//! pairing removes its route the next time the watcher looks -- and pairing
//! again under the same name is a new token with no route yet, rather than
//! one that inherits a phone.
//!
//! **Queued on disk and retried.** A relay or network outage delays a
//! notification instead of losing it, for up to the six hours the relay gives
//! APNs anyway. Every retry carries the same `event_id`, which the relay
//! recognises as a duplicate, so a retry after a lost answer is never a second
//! buzz.
//!
//! **Blocking, on a thread of its own.** Delivery is an HTTPS request at a
//! time, and the server's async runtime has no business waiting on one.

pub mod crypto;
pub mod watch;

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use hura_core::state;
use hura_proto::{Failure, Outcome, PushRoute, PushView, Reply, Request};

use crate::auth::Entry;
use rand::TryRng;
use serde::{Deserialize, Serialize};

// The kinds the relay understands. It maps each to a category and an
// interruption level itself, so a push token cannot be used to break through
// Focus at will.
pub const CLAUDE_TURN_FINISHED: &str = "claude_turn_finished";
pub const CLAUDE_NEEDS_INPUT: &str = "claude_needs_input";
pub const CLAUDE_FAILED: &str = "claude_failed";
pub const TEST: &str = "test";
/// A chat session's permission request, which the phone can answer from the
/// notification: `claude_needs_input` with an [`Notification::ask_id`], and a
/// category with Allow and Deny on it. Switched with `claude_needs_input`'s
/// preference, since to a person it is the same thing.
pub const CLAUDE_PERMISSION: &str = "claude_permission";

/// The kinds a client can switch off.
pub const PREF_KINDS: [&str; 3] = [CLAUDE_NEEDS_INPUT, CLAUDE_TURN_FINISHED, CLAUDE_FAILED];

/// More than this and the oldest go: a phone that has been unreachable long
/// enough to fill it is better told the recent things than all of them.
const MAX_QUEUE: usize = 500;
/// Matches the APNs expiration the relay sets.
const MAX_AGE: u64 = 6 * 60 * 60;
const MAX_BACKOFF: u64 = 5 * 60;
/// What a `429` without a `Retry-After` waits.
const DEFAULT_RETRY: u64 = 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// One notification for the relay: the body of `POST /v1/events`, before any
/// encryption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// Unique per server and stable across retries; see [`Pusher::event_id`].
    pub event_id: String,
    pub kind: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtitle: String,
    pub body: String,
    /// The session's name, which the app opens when the notification is
    /// tapped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The request a [`CLAUDE_PERMISSION`] notification answers, which the
    /// app sends back with Allow or Deny. Bound into an encrypted
    /// notification's additional data, so the relay cannot point an Allow at
    /// a different request from the one whose text the phone shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_id: Option<String>,
}

/// Where one paired client's notifications go, as kept in `push.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Route {
    /// The hash of the token that configured it.
    key: String,
    /// The token's name, for a person reading the file.
    client: String,
    relay_url: String,
    token: String,
    #[serde(default)]
    push_key: Option<String>,
    server_id: String,
    label: String,
    prefs: BTreeMap<String, bool>,
    created_at: u64,
    #[serde(default)]
    broken: Option<String>,
    #[serde(default)]
    last_success_at: Option<u64>,
    #[serde(default)]
    last_error: Option<String>,
}

impl Route {
    fn wants(&self, kind: &str) -> bool {
        let kind = if kind == CLAUDE_PERMISSION {
            CLAUDE_NEEDS_INPUT
        } else {
            kind
        };
        kind == TEST || self.prefs.get(kind).copied().unwrap_or(true)
    }
}

/// One notification waiting for one route, as kept in `push-queue.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Item {
    route: String,
    n: Notification,
    attempts: u32,
    /// Epoch seconds.
    next_at: u64,
    created_at: u64,
}

/// What the relay said to one request, reduced to what the rules need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    /// `0` when no answer came at all.
    pub status: u16,
    /// Seconds, from a `429`.
    pub retry_after: Option<u64>,
    /// The relay's reason or error, for the route's `last_error` and the log.
    pub message: String,
}

/// The two things this server asks of the relay. A trait so the response rules
/// can be tested against a fake.
pub trait Relay: Send + Sync {
    /// `GET /v1/link`: the link's label, or why the token is no good.
    fn link(&self, base: &str, token: &str) -> Result<String, Failure>;
    /// `POST /v1/events`.
    fn post(&self, base: &str, token: &str, body: &str) -> Posted;
}

/// The routes and the queue, and the one lock around both.
pub struct Pusher {
    dir: PathBuf,
    inner: Mutex<Inner>,
    /// Signalled when something is queued, so the delivery thread need not
    /// poll an empty queue.
    wake: Condvar,
    /// Random per start, so event ids from before a restart cannot collide
    /// with ones after it.
    boot: String,
    next: AtomicU64,
}

#[derive(Default)]
struct Inner {
    routes: BTreeMap<String, Route>,
    queue: Vec<Item>,
    woken: bool,
}

impl Pusher {
    pub fn default_dir() -> PathBuf {
        state::dir()
    }

    /// The routes and whatever was still queued when the server last stopped.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        let routes: Vec<Route> = read_json(&dir.join("push.json"))?;
        let queue: Vec<Item> = read_json(&dir.join("push-queue.json"))?;
        let mut boot = [0u8; 5];
        rand::rngs::SysRng
            .try_fill_bytes(&mut boot)
            .map_err(io::Error::other)?;
        Ok(Pusher {
            dir,
            inner: Mutex::new(Inner {
                routes: routes.into_iter().map(|r| (r.key.clone(), r)).collect(),
                queue,
                woken: false,
            }),
            wake: Condvar::new(),
            boot: crate::auth::hex(&boot),
            next: AtomicU64::new(1),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A fresh id for a notification.
    pub fn event_id(&self) -> String {
        format!(
            "h_{}-{}",
            self.boot,
            self.next.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// Whether anyone is listening. The watcher does nothing at all until
    /// someone is, so a server nobody configured push on pays nothing for it.
    pub fn has_routes(&self) -> bool {
        self.lock().routes.values().any(|r| r.broken.is_none())
    }

    /// Check the token with the relay and keep the route for the token `key`.
    ///
    /// Anything still queued for an older configuration of the same token
    /// goes: it was encrypted for, or aimed at, a phone that has just said
    /// where it is now.
    pub fn configure(
        &self,
        relay: &dyn Relay,
        key: &str,
        client: &str,
        route: PushRoute,
    ) -> Result<PushView, Failure> {
        let base = relay_url(&route.relay_url).map_err(Failure::failed)?;
        if !route.push_token.starts_with("hup_") {
            return Err(Failure::failed("push_token is not a hura relay push token"));
        }
        if let Some(k) = &route.push_key {
            crypto::parse_key(k).map_err(Failure::failed)?;
        }
        if !server_id(&route.server_id) {
            return Err(Failure::failed(
                "server_id must be `h_` and 16 base32 characters",
            ));
        }
        if let Some(k) = route
            .prefs
            .keys()
            .find(|k| !PREF_KINDS.contains(&k.as_str()))
        {
            return Err(Failure::failed(format!(
                "unknown notification kind `{k}` in prefs"
            )));
        }

        // Outside the lock: it is a round trip to the relay.
        let label = relay.link(&base, &route.push_token)?;

        let prefs = PREF_KINDS
            .iter()
            .map(|k| (k.to_string(), route.prefs.get(*k).copied().unwrap_or(true)))
            .collect();
        let r = Route {
            key: key.to_string(),
            client: client.to_string(),
            relay_url: base,
            token: route.push_token,
            push_key: route.push_key,
            server_id: route.server_id,
            label,
            prefs,
            created_at: now(),
            broken: None,
            last_success_at: None,
            last_error: None,
        };
        let mut inner = self.lock();
        inner.routes.insert(key.to_string(), r);
        inner.queue.retain(|it| it.route != key);
        self.save_routes(&inner)
            .map_err(|e| Failure::failed(format!("could not keep the route: {e}")))?;
        self.save_queue(&inner);
        eprintln!("hurad: push configured for {client}");
        Ok(view(&inner, key))
    }

    /// Forget the route for `key`, and what is queued for it.
    pub fn remove(&self, key: &str) -> PushView {
        let mut inner = self.lock();
        if let Some(r) = inner.routes.remove(key) {
            inner.queue.retain(|it| it.route != key);
            if let Err(e) = self.save_routes(&inner) {
                eprintln!("hurad: could not save the push routes: {e}");
            }
            self.save_queue(&inner);
            eprintln!("hurad: push removed for {}", r.client);
        }
        view(&inner, key)
    }

    pub fn status(&self, key: &str) -> PushView {
        view(&self.lock(), key)
    }

    /// Drop every route whose token is no longer accepted. `live` is the hashes
    /// of the tokens that are.
    pub fn prune(&self, live: &HashSet<String>) {
        let mut inner = self.lock();
        let before = inner.routes.len();
        inner.routes.retain(|key, _| live.contains(key));
        if inner.routes.len() != before {
            let Inner { routes, queue, .. } = &mut *inner;
            queue.retain(|it| routes.contains_key(&it.route));
            if let Err(e) = self.save_routes(&inner) {
                eprintln!("hurad: could not save the push routes: {e}");
            }
            self.save_queue(&inner);
        }
    }

    /// Queue `n` for every working route that wants its kind, or only for
    /// `only` when it is given. Returns how many routes it went to.
    pub fn enqueue(&self, n: Notification, only: Option<&str>) -> usize {
        let mut inner = self.lock();
        let at = now();
        let targets: Vec<String> = inner
            .routes
            .values()
            .filter(|r| r.broken.is_none() && r.wants(&n.kind))
            .filter(|r| only.is_none_or(|k| k == r.key))
            .map(|r| r.key.clone())
            .collect();
        for route in &targets {
            inner.queue.push(Item {
                route: route.clone(),
                n: n.clone(),
                attempts: 0,
                next_at: at,
                created_at: at,
            });
        }
        if let Some(over) = inner.queue.len().checked_sub(MAX_QUEUE).filter(|o| *o > 0) {
            eprintln!("hurad: the push queue is full; dropping the oldest {over}");
            inner.queue.drain(..over);
        }
        if !targets.is_empty() {
            self.save_queue(&inner);
            inner.woken = true;
            self.wake.notify_all();
        }
        targets.len()
    }

    /// Send everything that is due, oldest first, and say when the next thing
    /// will be: `None` for an empty queue.
    pub fn deliver(&self, relay: &dyn Relay) -> Option<u64> {
        while let Some((item, route)) = self.next_due() {
            let posted = match event_body(&route, &item.n) {
                Ok(body) => relay.post(&route.relay_url, &route.token, &body),
                Err(e) => Posted {
                    status: 400,
                    retry_after: None,
                    message: e,
                },
            };
            self.settle(&item, &route, posted);
        }
        self.lock().queue.iter().map(|it| it.next_at).min()
    }

    /// The oldest item that is due, with its route. Drops what has expired or
    /// lost its route on the way.
    fn next_due(&self) -> Option<(Item, Route)> {
        let mut inner = self.lock();
        let at = now();
        let before = inner.queue.len();
        let Inner { routes, queue, .. } = &mut *inner;
        queue.retain(|it| {
            at.saturating_sub(it.created_at) <= MAX_AGE
                && routes.get(&it.route).is_some_and(|r| r.broken.is_none())
        });
        if inner.queue.len() != before {
            self.save_queue(&inner);
        }
        let item = inner.queue.iter().find(|it| it.next_at <= at)?.clone();
        let route = inner.routes.get(&item.route)?.clone();
        Some((item, route))
    }

    /// Apply the relay's answer to one item: the table under `POST
    /// /v1/events` in `docs/relay-api.md`.
    fn settle(&self, item: &Item, route: &Route, posted: Posted) {
        let mut inner = self.lock();
        let same = |it: &Item| it.route == item.route && it.n.event_id == item.n.event_id;
        match posted.status {
            // Sent, a duplicate, or a phone that has gone: done either way.
            200 => {
                inner.queue.retain(|it| !same(it));
                if let Some(r) = inner.routes.get_mut(&route.key) {
                    r.last_success_at = Some(now());
                    r.last_error = None;
                }
            }
            // Malformed: sending it again will not make it less so.
            400 => {
                inner.queue.retain(|it| !same(it));
                if let Some(r) = inner.routes.get_mut(&route.key) {
                    r.last_error = Some(format!(
                        "the relay refused a notification: {}",
                        posted.message
                    ));
                }
            }
            // The token is revoked or unknown. Nothing more will get through
            // on it until the app configures the route again.
            401 => {
                inner.queue.retain(|it| it.route != route.key);
                if let Some(r) = inner.routes.get_mut(&route.key) {
                    r.broken = Some(
                        "the relay rejected the push token; configure push again from the app"
                            .into(),
                    );
                }
                eprintln!(
                    "hurad: the relay rejected the push token for {}",
                    route.client
                );
            }
            _ => {
                if let Some(it) = inner.queue.iter_mut().find(|it| same(it)) {
                    it.attempts += 1;
                    let backoff = 1u64 << it.attempts.min(9);
                    it.next_at = now() + posted.retry_after.unwrap_or(backoff.min(MAX_BACKOFF));
                }
                if let Some(r) = inner.routes.get_mut(&route.key) {
                    r.last_error = Some(posted.message.clone());
                }
            }
        }
        if let Err(e) = self.save_routes(&inner) {
            eprintln!("hurad: could not save the push routes: {e}");
        }
        self.save_queue(&inner);
    }

    /// Deliver until the process ends. Sleeps until the next item is due or
    /// something new is queued, whichever is sooner.
    pub fn run(&self, relay: &dyn Relay) {
        loop {
            let next = self.deliver(relay);
            let inner = self.lock();
            if inner.woken {
                drop(inner);
                self.lock().woken = false;
                continue;
            }
            let wait = next
                .map(|at| Duration::from_secs(at.saturating_sub(now()).max(1)))
                .unwrap_or(Duration::from_secs(3600));
            let (mut inner, _) = self
                .wake
                .wait_timeout_while(inner, wait, |i| !i.woken)
                .unwrap_or_else(|e| e.into_inner());
            inner.woken = false;
        }
    }

    fn save_routes(&self, inner: &Inner) -> io::Result<()> {
        let routes: Vec<&Route> = inner.routes.values().collect();
        let text = serde_json::to_string_pretty(&routes).map_err(io::Error::other)?;
        state::write_private(&self.dir.join("push.json"), &(text + "\n"))
    }

    /// Best effort: a queue that could not be saved is still delivered from
    /// memory, and only a restart in the meantime would lose it.
    fn save_queue(&self, inner: &Inner) {
        let written = serde_json::to_string(&inner.queue)
            .map_err(io::Error::other)
            .and_then(|text| state::write_private(&self.dir.join("push-queue.json"), &text));
        if let Err(e) = written {
            eprintln!("hurad: could not save the push queue: {e}");
        }
    }
}

/// Answer a push request for the connection whose token is `caller`, or
/// `None` for any other request.
///
/// Blocking: configuring is a round trip to the relay.
pub fn handle(pusher: &Pusher, caller: &Entry, request: &Request) -> Option<Outcome> {
    let key = caller.hash.as_str();
    let view = match request {
        Request::PushConfigure(route) => {
            pusher.configure(&Https::new(), key, &caller.name, route.clone())
        }
        Request::PushStatus => Ok(pusher.status(key)),
        Request::PushRemove => Ok(pusher.remove(key)),
        Request::PushTest => {
            let status = pusher.status(key);
            if !status.configured {
                Err(Failure::failed(
                    "push is not configured for this connection",
                ))
            } else if let Some(broken) = status.broken {
                Err(Failure::failed(broken))
            } else {
                pusher.enqueue(
                    Notification {
                        event_id: pusher.event_id(),
                        kind: TEST.into(),
                        title: "hura".into(),
                        subtitle: String::new(),
                        body: format!("Test notification from {}", crate::default_host()),
                        session: None,
                        ask_id: None,
                    },
                    Some(key),
                );
                Ok(pusher.status(key))
            }
        }
        _ => return None,
    };
    Some(match view {
        Ok(v) => Reply::Push(v).into(),
        Err(f) => f.into(),
    })
}

fn view(inner: &Inner, key: &str) -> PushView {
    let Some(r) = inner.routes.get(key) else {
        return PushView::default();
    };
    PushView {
        configured: true,
        relay_url: r.relay_url.clone(),
        label: r.label.clone(),
        prefs: r.prefs.clone(),
        encrypted: r.push_key.is_some(),
        broken: r.broken.clone(),
        last_success_at: r.last_success_at,
        last_error: r.last_error.clone(),
        queued: inner.queue.iter().filter(|it| it.route == key).count() as u32,
    }
}

/// The JSON for `POST /v1/events`: the text as it is, or for a route with a
/// key, sealed with a fresh nonce in place of all three fields.
fn event_body(route: &Route, n: &Notification) -> Result<String, String> {
    let Some(key) = &route.push_key else {
        return serde_json::to_string(n).map_err(|e| e.to_string());
    };
    let key = crypto::parse_key(key)?;
    let encrypted = crypto::encrypt(&key, &route.server_id, n)?;
    let mut body = serde_json::json!({
        "event_id": n.event_id,
        "kind": n.kind,
        "encrypted": encrypted,
    });
    // Left out rather than `null` when there is none, as the plaintext body
    // leaves it out.
    if let Some(session) = &n.session {
        body["session"] = session.clone().into();
    }
    if let Some(ask) = &n.ask_id {
        body["ask_id"] = ask.clone().into();
    }
    serde_json::to_string(&body).map_err(|e| e.to_string())
}

/// `https`, or `http` only on a loopback address, with no credentials, query
/// or fragment. Returned without a trailing slash, ready for a path.
fn relay_url(raw: &str) -> Result<String, String> {
    const BAD: &str = "relay_url must be an https URL";
    let (scheme, rest) = raw.split_once("://").ok_or(BAD)?;
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() || rest.contains(['@', '?', '#']) {
        return Err(BAD.into());
    }
    match scheme {
        "https" => {}
        "http" => {
            let host = match authority.strip_prefix('[') {
                Some(v6) => v6.split(']').next().unwrap_or_default(),
                None => authority.rsplit_once(':').map_or(authority, |(h, _)| h),
            };
            if !host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()) {
                return Err("relay_url must use https".into());
            }
        }
        _ => return Err(BAD.into()),
    }
    Ok(raw.trim_end_matches('/').to_string())
}

/// `h_` and 16 lowercase base32 characters, as the relay checks it.
fn server_id(id: &str) -> bool {
    id.strip_prefix("h_").is_some_and(|rest| {
        rest.len() == 16
            && rest
                .bytes()
                .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
    })
}

fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// The relay over the network.
pub struct Https {
    agent: ureq::Agent,
}

impl Https {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        Https { agent }
    }
}

impl Relay for Https {
    fn link(&self, base: &str, token: &str) -> Result<String, Failure> {
        let mut resp = self
            .agent
            .get(format!("{base}/v1/link"))
            .header("Authorization", format!("Bearer {token}"))
            .call()
            .map_err(|e| Failure::gateway(format!("could not reach the relay: {e}")))?;
        match resp.status().as_u16() {
            200 => {}
            401 => return Err(Failure::failed("the relay rejected the push token")),
            other => return Err(Failure::gateway(format!("the relay answered {other}"))),
        }
        #[derive(Deserialize)]
        struct Link {
            #[serde(default)]
            label: String,
        }
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        Ok(serde_json::from_str::<Link>(&text)
            .map(|l| l.label)
            .unwrap_or_default())
    }

    fn post(&self, base: &str, token: &str, body: &str) -> Posted {
        let sent = self
            .agent
            .post(format!("{base}/v1/events"))
            .header("Authorization", format!("Bearer {token}"))
            .content_type("application/json")
            .send(body);
        let mut resp = match sent {
            Ok(r) => r,
            Err(e) => {
                return Posted {
                    status: 0,
                    retry_after: None,
                    message: e.to_string(),
                };
            }
        };
        let status = resp.status().as_u16();
        let retry_after = (status == 429).then(|| {
            resp.headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .filter(|s| *s > 0)
                .unwrap_or(DEFAULT_RETRY)
        });
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        Posted {
            status,
            retry_after,
            message: answer(status, &text),
        }
    }
}

/// What an answer from the relay says, in a line.
fn answer(status: u16, text: &str) -> String {
    #[derive(Deserialize, Default)]
    struct Body {
        #[serde(default)]
        sent: bool,
        #[serde(default)]
        reason: String,
        #[serde(default)]
        error: Option<Error>,
    }
    #[derive(Deserialize)]
    struct Error {
        message: String,
    }
    let body: Body = serde_json::from_str(text).unwrap_or_default();
    match (status, body) {
        (200, b) if b.sent => "sent".into(),
        (200, b) => b.reason,
        (s, Body { error: Some(e), .. }) => format!("{s}: {}", e.message),
        (s, _) => format!("the relay answered {s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relay as a script: what to answer, and what it was sent.
    #[derive(Default)]
    pub(super) struct Fake {
        pub answers: Mutex<Vec<Posted>>,
        pub sent: Mutex<Vec<serde_json::Value>>,
        pub link: Option<Failure>,
    }

    impl Fake {
        fn answering(answers: Vec<Posted>) -> Self {
            Fake {
                answers: Mutex::new(answers),
                ..Default::default()
            }
        }
        fn sent(&self) -> Vec<serde_json::Value> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl Relay for Fake {
        fn link(&self, _: &str, _: &str) -> Result<String, Failure> {
            match &self.link {
                Some(f) => Err(f.clone()),
                None => Ok("box".into()),
            }
        }
        fn post(&self, _: &str, _: &str, body: &str) -> Posted {
            let event: serde_json::Value = serde_json::from_str(body).unwrap();
            self.sent.lock().unwrap().push(event.clone());
            // What the real one refuses, so a shape it would not take fails
            // here rather than on somebody's phone.
            if let Err(why) = relay_would_take(&event) {
                return Posted {
                    status: 400,
                    retry_after: None,
                    message: why,
                };
            }
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() {
                ok()
            } else {
                answers.remove(0)
            }
        }
    }

    /// The relay's own checks on `POST /v1/events` (`docs/relay-api.md`).
    fn relay_would_take(event: &serde_json::Value) -> Result<(), String> {
        let event_id = event["event_id"].as_str().ok_or("event_id is required")?;
        if event_id.is_empty()
            || event_id.len() > 100
            || !event_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
        {
            return Err(format!("event_id is malformed: {event_id}"));
        }
        if let Some(session) = event.get("session").filter(|v| !v.is_null()) {
            let session = session.as_str().ok_or("session must be a string")?;
            let ok = !session.is_empty()
                && session.len() <= 64
                && !session.starts_with('-')
                && session
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !ok {
                return Err(format!("session is malformed: {session}"));
            }
        }
        if let Some(ask) = event.get("ask_id") {
            let ask = ask.as_str().ok_or("ask_id must be a string")?;
            if ask.is_empty()
                || ask.len() > 100
                || !ask
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            {
                return Err(format!("ask_id is malformed: {ask}"));
            }
        }
        Ok(())
    }

    fn ok() -> Posted {
        Posted {
            status: 200,
            retry_after: None,
            message: "sent".into(),
        }
    }

    fn status(code: u16) -> Posted {
        Posted {
            status: code,
            retry_after: None,
            message: format!("{code}"),
        }
    }

    fn route() -> PushRoute {
        PushRoute {
            relay_url: "https://relay.example/".into(),
            push_token: "hup_token".into(),
            push_key: None,
            server_id: "h_aaaabbbbccccdddd".into(),
            prefs: BTreeMap::new(),
        }
    }

    fn pusher() -> (Pusher, tempdir::Dir) {
        let dir = tempdir::Dir::new();
        (Pusher::open(dir.path()).unwrap(), dir)
    }

    fn note(p: &Pusher, kind: &str) -> Notification {
        Notification {
            event_id: p.event_id(),
            kind: kind.into(),
            title: "fix-tests".into(),
            subtitle: "box".into(),
            body: "Claude is asking to use Bash".into(),
            session: Some("fix-tests".into()),
            ask_id: None,
        }
    }

    /// A directory that is gone when the test is, without a dev-dependency.
    mod tempdir {
        pub struct Dir(std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                use std::sync::atomic::{AtomicU32, Ordering};
                static N: AtomicU32 = AtomicU32::new(0);
                let path = std::env::temp_dir().join(format!(
                    "hurad-push-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir_all(&path).unwrap();
                Dir(path)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn configure_checks_the_token_and_keeps_the_route() {
        let (p, dir) = pusher();
        let v = p
            .configure(&Fake::default(), "k", "phone", route())
            .unwrap();
        assert!(v.configured);
        assert_eq!(v.relay_url, "https://relay.example");
        assert_eq!(v.label, "box");
        assert!(!v.encrypted);
        assert!(PREF_KINDS.iter().all(|k| v.prefs[*k]));
        // And it is still there after a restart.
        assert!(Pusher::open(dir.path()).unwrap().status("k").configured);
    }

    #[test]
    fn configure_refuses_what_the_relay_would() {
        let (p, _dir) = pusher();
        let relay = Fake::default();
        let bad = [
            PushRoute {
                relay_url: "http://relay.example".into(),
                ..route()
            },
            PushRoute {
                relay_url: "ftp://relay.example".into(),
                ..route()
            },
            PushRoute {
                relay_url: "https://user@relay.example".into(),
                ..route()
            },
            PushRoute {
                push_token: "hdt_nope".into(),
                ..route()
            },
            PushRoute {
                push_key: Some("short".into()),
                ..route()
            },
            PushRoute {
                server_id: "h_UPPERCASE000000".into(),
                ..route()
            },
            PushRoute {
                prefs: BTreeMap::from([("everything".to_string(), false)]),
                ..route()
            },
        ];
        for r in bad {
            let shown = format!("{r:?}");
            assert!(p.configure(&relay, "k", "phone", r).is_err(), "{shown}");
        }
        assert!(!p.status("k").configured);
    }

    #[test]
    fn plain_http_is_for_loopback_only() {
        assert!(relay_url("http://127.0.0.1:8787").is_ok());
        assert!(relay_url("http://[::1]:8787/").is_ok());
        assert!(relay_url("http://localhost:8787").is_err());
        assert!(relay_url("http://10.0.0.7").is_err());
    }

    #[test]
    fn a_rejected_token_is_not_kept() {
        let (p, _dir) = pusher();
        let relay = Fake {
            link: Some(Failure::failed("the relay rejected the push token")),
            ..Default::default()
        };
        assert!(p.configure(&relay, "k", "phone", route()).is_err());
        assert!(!p.has_routes());
    }

    #[test]
    fn delivers_to_routes_that_want_the_kind() {
        let (p, _dir) = pusher();
        let relay = Fake::default();
        p.configure(&relay, "a", "phone", route()).unwrap();
        let off = PushRoute {
            prefs: BTreeMap::from([(CLAUDE_TURN_FINISHED.to_string(), false)]),
            ..route()
        };
        p.configure(&relay, "b", "tablet", off).unwrap();

        assert_eq!(p.enqueue(note(&p, CLAUDE_TURN_FINISHED), None), 1);
        assert_eq!(p.enqueue(note(&p, CLAUDE_NEEDS_INPUT), None), 2);
        assert_eq!(p.deliver(&relay), None);
        assert_eq!(p.status("a").last_error, None, "the relay took them");
        let sent = relay.sent();
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[0]["kind"], CLAUDE_TURN_FINISHED);
        assert_eq!(sent[0]["title"], "fix-tests");
        assert_eq!(sent[0]["session"], "fix-tests");
        assert!(sent[0].get("turn").is_none());
        assert!(p.status("a").last_success_at.is_some());
    }

    #[test]
    fn a_test_goes_only_where_it_was_asked_for() {
        let (p, _dir) = pusher();
        let relay = Fake::default();
        p.configure(&relay, "a", "phone", route()).unwrap();
        p.configure(&relay, "b", "tablet", route()).unwrap();
        assert_eq!(p.enqueue(note(&p, TEST), Some("b")), 1);
    }

    #[test]
    fn a_key_means_only_ciphertext_leaves() {
        let (p, _dir) = pusher();
        let relay = Fake::default();
        let keyed = PushRoute {
            push_key: Some("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8".into()),
            ..route()
        };
        assert!(p.configure(&relay, "k", "phone", keyed).unwrap().encrypted);
        p.enqueue(note(&p, CLAUDE_NEEDS_INPUT), None);
        p.deliver(&relay);
        let sent = &relay.sent()[0];
        assert!(sent["encrypted"].is_string());
        for field in ["title", "subtitle", "body"] {
            assert!(sent.get(field).is_none(), "{field} leaked: {sent}");
        }
        assert_eq!(sent["session"], "fix-tests");
    }

    #[test]
    fn a_failure_is_retried_later_with_the_same_event_id() {
        let (p, _dir) = pusher();
        let relay = Fake::answering(vec![status(0)]);
        p.configure(&relay, "k", "phone", route()).unwrap();
        p.enqueue(note(&p, CLAUDE_NEEDS_INPUT), None);
        let next = p.deliver(&relay).expect("still queued");
        assert!(next > now(), "retried later, not at once");
        assert_eq!(p.status("k").queued, 1);
        assert!(p.status("k").last_error.is_some());

        // Make it due, and the retry goes out under the same id.
        p.lock().queue[0].next_at = 0;
        assert_eq!(p.deliver(&relay), None);
        let sent = relay.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0]["event_id"], sent[1]["event_id"]);
        assert_eq!(p.status("k").last_error, None);
    }

    #[test]
    fn rate_limits_wait_as_long_as_the_relay_says() {
        let (p, _dir) = pusher();
        let relay = Fake::answering(vec![Posted {
            status: 429,
            retry_after: Some(120),
            message: "rate limited".into(),
        }]);
        p.configure(&relay, "k", "phone", route()).unwrap();
        p.enqueue(note(&p, CLAUDE_NEEDS_INPUT), None);
        let next = p.deliver(&relay).unwrap();
        assert!(next >= now() + 119, "{next}");
    }

    #[test]
    fn a_malformed_event_is_dropped() {
        let (p, _dir) = pusher();
        let relay = Fake::answering(vec![status(400)]);
        p.configure(&relay, "k", "phone", route()).unwrap();
        p.enqueue(note(&p, CLAUDE_NEEDS_INPUT), None);
        assert_eq!(p.deliver(&relay), None);
        assert!(p.status("k").last_error.is_some());
        assert!(p.has_routes());
    }

    #[test]
    fn a_revoked_push_token_breaks_the_route_until_configured_again() {
        let (p, _dir) = pusher();
        let relay = Fake::answering(vec![status(401)]);
        p.configure(&relay, "k", "phone", route()).unwrap();
        p.enqueue(note(&p, CLAUDE_NEEDS_INPUT), None);
        p.enqueue(note(&p, CLAUDE_FAILED), None);
        assert_eq!(p.deliver(&relay), None);
        assert_eq!(
            relay.sent().len(),
            1,
            "nothing more is tried on a dead token"
        );
        assert!(p.status("k").broken.is_some());
        assert!(!p.has_routes());
        assert_eq!(p.enqueue(note(&p, CLAUDE_NEEDS_INPUT), None), 0);

        p.configure(&relay, "k", "phone", route()).unwrap();
        assert!(p.has_routes());
    }

    #[test]
    fn revoking_the_pairing_removes_the_route() {
        let (p, _dir) = pusher();
        let relay = Fake::default();
        p.configure(&relay, "a", "phone", route()).unwrap();
        p.configure(&relay, "b", "tablet", route()).unwrap();
        p.enqueue(note(&p, CLAUDE_NEEDS_INPUT), None);
        p.prune(&HashSet::from(["b".to_string()]));
        assert!(!p.status("a").configured);
        assert!(p.status("b").configured);
        assert_eq!(p.status("b").queued, 1);
        assert_eq!(p.lock().queue.len(), 1);
    }

    #[test]
    fn the_queue_survives_a_restart() {
        let (p, dir) = pusher();
        let relay = Fake::default();
        p.configure(&relay, "k", "phone", route()).unwrap();
        p.enqueue(note(&p, CLAUDE_NEEDS_INPUT), None);
        drop(p);
        let again = Pusher::open(dir.path()).unwrap();
        assert_eq!(again.status("k").queued, 1);
        again.deliver(&relay);
        assert_eq!(relay.sent().len(), 1);
    }

    #[test]
    fn what_is_too_old_is_not_sent() {
        let (p, _dir) = pusher();
        let relay = Fake::default();
        p.configure(&relay, "k", "phone", route()).unwrap();
        p.enqueue(note(&p, CLAUDE_NEEDS_INPUT), None);
        p.lock().queue[0].created_at = now() - MAX_AGE - 1;
        assert_eq!(p.deliver(&relay), None);
        assert!(relay.sent().is_empty());
    }

    #[test]
    fn reconfiguring_drops_what_was_queued_for_the_old_route() {
        let (p, _dir) = pusher();
        let relay = Fake::default();
        p.configure(&relay, "k", "phone", route()).unwrap();
        p.enqueue(note(&p, CLAUDE_NEEDS_INPUT), None);
        p.configure(&relay, "k", "phone", route()).unwrap();
        assert_eq!(p.status("k").queued, 0);
    }

    #[test]
    fn event_ids_do_not_repeat() {
        let (p, _dir) = pusher();
        let (q, _other) = pusher();
        let ids: HashSet<String> = (0..10)
            .map(|_| p.event_id())
            .chain((0..10).map(|_| q.event_id()))
            .collect();
        assert_eq!(ids.len(), 20);
    }

    fn permission(p: &Pusher) -> Notification {
        Notification {
            ask_id: Some("toolu_01abc".into()),
            ..note(p, CLAUDE_PERMISSION)
        }
    }

    #[test]
    fn a_permission_request_carries_its_id_beside_the_ciphertext() {
        let (p, _dir) = pusher();
        let relay = Fake::default();
        let keyed = PushRoute {
            push_key: Some("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8".into()),
            ..route()
        };
        p.configure(&relay, "k", "phone", keyed).unwrap();
        p.enqueue(permission(&p), None);
        p.deliver(&relay);
        let sent = &relay.sent()[0];
        assert_eq!(sent["kind"], CLAUDE_PERMISSION);
        assert_eq!(sent["ask_id"], "toolu_01abc");
        assert!(sent["encrypted"].is_string());
    }

    #[test]
    fn a_permission_request_follows_the_input_preference() {
        let (p, _dir) = pusher();
        let off = PushRoute {
            prefs: BTreeMap::from([(CLAUDE_NEEDS_INPUT.to_string(), false)]),
            ..route()
        };
        p.configure(&Fake::default(), "k", "phone", off).unwrap();
        assert_eq!(p.enqueue(permission(&p), None), 0);
    }

    #[test]
    fn a_session_goes_by_its_name() {
        let event = serde_json::json!({"event_id": "h_1-1", "session": "fix-tests"});
        assert!(relay_would_take(&event).is_ok());
        let event = serde_json::json!({"event_id": "h_1-1", "session": "Not A Name"});
        assert!(relay_would_take(&event).is_err());
    }

    #[test]
    fn an_encrypted_event_without_a_session_leaves_it_out() {
        let (p, _dir) = pusher();
        let relay = Fake::default();
        let keyed = PushRoute {
            push_key: Some("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8".into()),
            ..route()
        };
        p.configure(&relay, "k", "phone", keyed).unwrap();
        p.enqueue(
            Notification {
                session: None,
                ..note(&p, TEST)
            },
            None,
        );
        p.deliver(&relay);
        assert!(relay.sent()[0].get("session").is_none());
    }

    #[test]
    fn answers_read_as_a_line() {
        assert_eq!(answer(200, r#"{"sent":true}"#), "sent");
        assert_eq!(
            answer(200, r#"{"sent":false,"reason":"duplicate"}"#),
            "duplicate"
        );
        assert_eq!(
            answer(400, r#"{"error":{"code":"bad_request","message":"kind"}}"#),
            "400: kind"
        );
        assert_eq!(answer(502, ""), "the relay answered 502");
    }
}
