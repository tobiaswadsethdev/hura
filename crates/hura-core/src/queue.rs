//! Sessions waiting to start, and messages waiting to be said.
//!
//! Two kinds of waiting, kept in one file because one thing acts on both: the
//! scheduler in `hurad serve`, which looks every few seconds.
//!
//! * **A session** waits for a time (`hurad new --at 02:00`) or for room: a
//!   queued one starts once fewer than [`crate::config::Config::queue_limit`]
//!   sessions are creating, seeding or running. A scheduled one starts at its
//!   time whatever the limit says -- a time is a decision someone made, and
//!   holding it back for a slot would make it a different one.
//! * **A message** waits for a time, if it was given one, and for a session's
//!   agent to finish what it is doing, and is then said to it as though
//!   typed, one at a time and in order. A message whose time comes while the
//!   agent is mid-turn is said when the turn ends, rather than typed over it.
//!
//! **On disk, and only taken off once it has happened.** The point of a queue
//! is that nobody is watching it, so it has to outlive the server and the
//! machine. A session about to start is marked [`QueuedSession::started`]
//! before it is handed over and removed once its create has finished; a
//! server that dies in between finds the mark on its way up ([`recover`]) and
//! either sees the session it was making or starts it again. A message is
//! removed after it was said, so the worst a crash can do is say one twice.
//!
//! Every change is a locked read-modify-write, for [`crate::store`]'s reason:
//! a CLI adding to the queue while the server takes from it is the normal case.
//!
//! What to start and what to say is decided by pure functions ([`Queue::due`],
//! [`Queue::next_messages`]) over a description of the sessions, so the rules
//! are tested without a sandbox; looking at the sessions is the server's.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ops::NewSession;

/// How many sessions the queue lets run at once when the config file does not
/// say.
pub const DEFAULT_LIMIT: u32 = 2;

/// A session waiting to start.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedSession {
    /// Unique across the whole file, sessions and messages alike, and never
    /// reused: removing one by id must not remove whatever took its place.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: u64,
    /// What to start, as a client asked for it. Its name is settled when it is
    /// added, so the queue can say what it will be called and a second queued
    /// session for the same task is named around it.
    pub new: NewSession,
    /// When to start it, in seconds since the epoch. `None` waits for room.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub at: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub added: u64,
    /// Set while it is being started. See the module note.
    #[serde(default)]
    pub started: bool,
    /// Why it could not start when its turn came, when it could not: the
    /// config file changed under it, say. It waits, without starting, until
    /// it is removed or started again by hand.
    #[serde(default)]
    pub failed: Option<String>,
}

impl QueuedSession {
    /// The name it will have, which [`Queue::add_session`] made sure of.
    pub fn name(&self) -> &str {
        self.new.name.as_deref().unwrap_or_default()
    }
}

/// Something to say to a session once its agent is free, and not before
/// `at` when it has one.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedMessage {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: u64,
    pub session: String,
    pub text: String,
    /// When to say it, in seconds since the epoch. `None` is as soon as the
    /// agent has finished its turn.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub at: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub added: u64,
}

/// Everything waiting, as the file holds it and as a client is sent it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "QueueView"))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queue {
    /// Oldest first, which is the order a queued one is started in.
    #[serde(default)]
    pub sessions: Vec<QueuedSession>,
    /// Oldest first; a session's are said in this order.
    #[serde(default)]
    pub messages: Vec<QueuedMessage>,
    /// [`crate::config::Config::queue_limit`], filled in for a client and not
    /// kept: the config file is where it is set.
    #[serde(default)]
    pub limit: u32,
    /// The next id to hand out.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(skip))]
    next: u64,
}

/// A session as the scheduler sees it on one look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub name: String,
    /// Creating, seeding, or its agent in the middle of a turn: what takes a
    /// place in the queue's limit.
    pub busy: bool,
    /// Its agent is there and has nothing to do: what a message waits for.
    pub idle: bool,
}

impl Queue {
    fn take_id(&mut self) -> u64 {
        // Past every id present as well as the counter, so a file written by
        // hand, or one from before the counter, cannot hand out one twice.
        let highest = self
            .sessions
            .iter()
            .map(|s| s.id)
            .chain(self.messages.iter().map(|m| m.id))
            .max()
            .unwrap_or(0);
        self.next = self.next.max(highest) + 1;
        self.next
    }

    /// Every name a queued session will take, so a new session is named
    /// around them.
    pub fn names(&self) -> Vec<String> {
        self.sessions.iter().map(|s| s.name().to_string()).collect()
    }

    /// Add a session. Its name has to be settled already; see
    /// [`QueuedSession::new`].
    pub fn add_session(&mut self, new: NewSession, at: Option<u64>, now: u64) -> u64 {
        let id = self.take_id();
        self.sessions.push(QueuedSession {
            id,
            new,
            at,
            added: now,
            started: false,
            failed: None,
        });
        id
    }

    pub fn add_message(&mut self, session: &str, text: &str, at: Option<u64>, now: u64) -> u64 {
        let id = self.take_id();
        self.messages.push(QueuedMessage {
            id,
            session: session.to_string(),
            text: text.to_string(),
            at,
            added: now,
        });
        id
    }

    /// Take a session or a message off, by id. Whether there was one. A
    /// session already being started is left: it is a create in flight by
    /// now, and destroying the session is how to be rid of one of those.
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.sessions.len() + self.messages.len();
        self.sessions.retain(|s| s.id != id || s.started);
        self.messages.retain(|m| m.id != id);
        before != self.sessions.len() + self.messages.len()
    }

    /// Start a waiting session at the next look, room or not. Whether there
    /// was one.
    pub fn start_now(&mut self, id: u64, now: u64) -> bool {
        match self.sessions.iter_mut().find(|s| s.id == id && !s.started) {
            Some(s) => {
                s.at = Some(now);
                s.failed = None;
                true
            }
            None => false,
        }
    }

    /// Forget whatever is waiting to be said to a session, which is what its
    /// going away means.
    pub fn forget_session(&mut self, name: &str) {
        self.messages.retain(|m| m.session != name);
    }

    /// The sessions to start now, by id, in the order to start them.
    ///
    /// Every scheduled one whose time has come, then queued ones oldest first
    /// while there is room. Room is `limit` less what is busy, counting the
    /// ones being started and the scheduled ones starting now: all of them
    /// will be running a moment from now.
    pub fn due(&self, now: u64, seen: &[Seen], limit: u32) -> Vec<u64> {
        let mut busy: BTreeSet<&str> = seen
            .iter()
            .filter(|s| s.busy)
            .map(|s| s.name.as_str())
            .collect();
        busy.extend(
            self.sessions
                .iter()
                .filter(|s| s.started)
                .map(QueuedSession::name),
        );

        let waiting = || {
            self.sessions
                .iter()
                .filter(|s| !s.started && s.failed.is_none())
        };
        let mut out: Vec<u64> = Vec::new();
        for s in waiting().filter(|s| s.at.is_some_and(|at| at <= now)) {
            busy.insert(s.name());
            out.push(s.id);
        }
        let mut room = (limit as usize).saturating_sub(busy.len());
        for s in waiting().filter(|s| s.at.is_none()) {
            if room == 0 {
                break;
            }
            room -= 1;
            out.push(s.id);
        }
        out
    }

    /// Whether the queue has a session waiting for room, which is the only
    /// thing that needs every session looked at to decide.
    pub fn wants_room(&self) -> bool {
        self.sessions
            .iter()
            .any(|s| !s.started && s.failed.is_none() && s.at.is_none())
    }

    /// Every session something is waiting to be said to, whether or not its
    /// time has come.
    pub fn told(&self) -> BTreeSet<&str> {
        self.messages.iter().map(|m| m.session.as_str()).collect()
    }

    /// The sessions with a message whose time has come: the ones worth
    /// looking at to see whether they are free.
    pub fn due_told(&self, now: u64) -> BTreeSet<&str> {
        self.messages
            .iter()
            .filter(|m| m.at.is_none_or(|at| at <= now))
            .map(|m| m.session.as_str())
            .collect()
    }

    /// The message to say now to each session that is free to hear one: the
    /// oldest whose time has come. One per session per look, so the next
    /// waits for the turn this one starts to end.
    ///
    /// Oldest by when it was added, not by its time: two scheduled for the
    /// same session that both fall due while it is busy are said in the order
    /// they were written, which is the order they were meant to be read in.
    pub fn next_messages(&self, now: u64, seen: &[Seen]) -> Vec<&QueuedMessage> {
        let mut out: Vec<&QueuedMessage> = Vec::new();
        for m in &self.messages {
            if m.at.is_some_and(|at| at > now) {
                continue;
            }
            let free = seen.iter().any(|s| s.name == m.session && s.idle);
            if free && !out.iter().any(|o| o.session == m.session) {
                out.push(m);
            }
        }
        out
    }
}

/// `queue.json`, beside the session cache.
pub fn default_path() -> PathBuf {
    crate::store::Store::default_path().with_file_name("queue.json")
}

fn lock_path(path: &Path) -> PathBuf {
    path.with_extension("lock")
}

/// The queue as it stands. A file that is not there is an empty queue.
pub fn load() -> io::Result<Queue> {
    load_at(&default_path())
}

pub fn load_at(path: &Path) -> io::Result<Queue> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Queue::default()),
        Err(e) => Err(e),
    }
}

/// Read the queue, change it and write it back, with the file locked
/// throughout. See [`crate::store::update`].
pub fn update<T>(f: impl FnOnce(&mut Queue) -> T) -> io::Result<T> {
    update_at(&default_path(), f)
}

pub fn update_at<T>(path: &Path, f: impl FnOnce(&mut Queue) -> T) -> io::Result<T> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let guard = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path(path))?;
    guard.lock()?;

    let result = (|| {
        let mut q = load_at(path)?;
        let out = f(&mut q);
        q.limit = 0;
        let json = serde_json::to_string_pretty(&q).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)?;
        Ok(out)
    })();

    let _ = guard.unlock();
    result
}

/// Settle the sessions a server left half-started when it stopped.
///
/// One whose session exists got as far as a record, and the create either
/// finished or recorded why not; it is done with. One whose session does not
/// was lost before it had one, and goes back to waiting -- at the front, since
/// it was its turn. What came back to waiting, by name.
pub fn recover(q: &mut Queue, existing: &[String]) -> Vec<String> {
    let mut back = Vec::new();
    q.sessions.retain_mut(|s| {
        if !s.started {
            return true;
        }
        if existing.iter().any(|n| n == s.name()) {
            return false;
        }
        s.started = false;
        back.push(s.name().to_string());
        true
    });
    back
}

/// Queue a session, or schedule it for `at`, and answer with its id.
///
/// The request is turned into a draft here exactly as a create would turn it,
/// so whatever would refuse it then -- a name with a slash in it, a toolchain
/// nobody has heard of -- refuses it now, while someone is there to read it.
/// The draft itself is not kept: the config file may change before it starts,
/// and a session should start from the file as it is then. What is kept is the
/// name and title it was given, named around every session and every other
/// queued one.
pub fn add(
    mut new: NewSession,
    at: Option<u64>,
    cfg: &crate::config::Config,
) -> Result<u64, String> {
    let mut taken = crate::store::Store::load()
        .map(|s| s.names())
        .unwrap_or_default();
    taken.extend(load().map(|q| q.names()).unwrap_or_default());
    let draft = new.clone().into_draft(cfg, &taken)?;
    new.name = Some(draft.name);
    new.title = draft.title;
    update(|q| q.add_session(new, at, now())).map_err(|e| e.to_string())
}

/// Queue a message for a session's agent, to be said once it is free and not
/// before `at`, and answer with its id.
pub fn say(session: &str, text: &str, at: Option<u64>) -> Result<u64, String> {
    if text.trim().is_empty() {
        return Err("nothing to say".into());
    }
    let store = crate::store::Store::load().map_err(|e| e.to_string())?;
    if !store.contains(session) {
        return Err(format!("no session named `{session}`"));
    }
    update(|q| q.add_message(session, text, at, now())).map_err(|e| e.to_string())
}

/// The queue as a client is shown it: the file, and the limit it works to.
pub fn view(cfg: &crate::config::Config) -> io::Result<Queue> {
    let mut q = load()?;
    q.limit = cfg.queue_limit();
    Ok(q)
}

/// Seconds since the epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> NewSession {
        NewSession {
            name: Some(name.into()),
            ..Default::default()
        }
    }

    fn seen(name: &str, busy: bool, idle: bool) -> Seen {
        Seen {
            name: name.into(),
            busy,
            idle,
        }
    }

    #[test]
    fn ids_are_never_reused() {
        let mut q = Queue::default();
        let a = q.add_session(named("a"), None, 0);
        let b = q.add_message("a", "hi", None, 0);
        assert!(q.remove(b));
        let c = q.add_message("a", "again", None, 0);
        assert_ne!(b, c);
        assert!(a < b && b < c);
    }

    #[test]
    fn queued_sessions_fill_the_room_oldest_first() {
        let mut q = Queue::default();
        let a = q.add_session(named("a"), None, 0);
        let b = q.add_session(named("b"), None, 1);
        q.add_session(named("c"), None, 2);
        assert_eq!(q.due(10, &[], 2), vec![a, b]);
        assert_eq!(q.due(10, &[seen("x", true, false)], 2), vec![a]);
        assert!(
            q.due(10, &[seen("x", true, false), seen("y", true, false)], 2)
                .is_empty()
        );
    }

    #[test]
    fn idle_and_waiting_sessions_leave_room() {
        let mut q = Queue::default();
        let a = q.add_session(named("a"), None, 0);
        let idle = [seen("x", false, true), seen("y", false, false)];
        assert_eq!(q.due(10, &idle, 1), vec![a]);
    }

    #[test]
    fn one_being_started_takes_its_place() {
        let mut q = Queue::default();
        let a = q.add_session(named("a"), None, 0);
        q.add_session(named("b"), None, 1);
        q.sessions[0].started = true;
        assert!(q.due(10, &[], 1).is_empty());
        // Seen by the scheduler too, once its record exists: counted once.
        assert!(q.due(10, &[seen("a", true, false)], 1).is_empty());
        assert!(!q.due(10, &[], 2).contains(&a));
    }

    #[test]
    fn a_scheduled_session_waits_for_its_time_and_not_for_room() {
        let mut q = Queue::default();
        let s = q.add_session(named("later"), Some(100), 0);
        let full = [seen("x", true, false), seen("y", true, false)];
        assert!(q.due(99, &full, 2).is_empty());
        assert_eq!(q.due(100, &full, 2), vec![s]);
    }

    #[test]
    fn a_scheduled_session_starting_uses_up_room() {
        let mut q = Queue::default();
        let s = q.add_session(named("now"), Some(5), 0);
        let queued = q.add_session(named("q"), None, 1);
        assert_eq!(q.due(10, &[], 1), vec![s]);
        assert_eq!(q.due(10, &[], 2), vec![s, queued]);
    }

    #[test]
    fn one_that_could_not_start_waits_until_started_by_hand() {
        let mut q = Queue::default();
        let a = q.add_session(named("a"), None, 0);
        q.sessions[0].failed = Some("no such toolchain".into());
        assert!(q.due(10, &[], 2).is_empty());
        assert!(!q.wants_room());
        assert!(q.start_now(a, 10));
        assert_eq!(q.due(10, &[], 2), vec![a]);
    }

    #[test]
    fn start_now_jumps_the_queue() {
        let mut q = Queue::default();
        q.add_session(named("a"), None, 0);
        let b = q.add_session(named("b"), None, 1);
        assert!(q.start_now(b, 50));
        assert_eq!(q.due(50, &[seen("x", true, false)], 1), vec![b]);
    }

    #[test]
    fn one_being_started_cannot_be_removed() {
        let mut q = Queue::default();
        let a = q.add_session(named("a"), None, 0);
        q.sessions[0].started = true;
        assert!(!q.remove(a));
        assert!(!q.start_now(a, 1));
    }

    #[test]
    fn each_free_session_hears_its_oldest_message() {
        let mut q = Queue::default();
        let first = q.add_message("a", "one", None, 0);
        q.add_message("a", "two", None, 1);
        let other = q.add_message("b", "three", None, 2);
        q.add_message("c", "busy", None, 3);
        let s = [
            seen("a", false, true),
            seen("b", false, true),
            seen("c", true, false),
        ];
        let ids: Vec<u64> = q.next_messages(10, &s).iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![first, other]);
    }

    #[test]
    fn a_timed_message_waits_for_its_time_and_then_for_the_turn() {
        let mut q = Queue::default();
        let later = q.add_message("a", "at nine", Some(100), 0);
        let free = [seen("a", false, true)];
        assert!(q.next_messages(99, &free).is_empty());
        assert!(q.due_told(99).is_empty());
        assert!(q.next_messages(100, &[seen("a", true, false)]).is_empty());
        let ids: Vec<u64> = q.next_messages(150, &free).iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![later]);
    }

    #[test]
    fn an_untimed_message_is_not_held_behind_a_timed_one() {
        let mut q = Queue::default();
        q.add_message("a", "tomorrow", Some(1_000), 0);
        let now = q.add_message("a", "now", None, 1);
        let ids: Vec<u64> = q
            .next_messages(10, &[seen("a", false, true)])
            .iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec![now]);
    }

    #[test]
    fn a_session_going_takes_its_messages() {
        let mut q = Queue::default();
        q.add_message("a", "one", None, 0);
        q.add_message("b", "two", None, 0);
        q.forget_session("a");
        assert_eq!(q.told(), BTreeSet::from(["b"]));
    }

    #[test]
    fn a_half_started_session_is_settled_on_the_way_up() {
        let mut q = Queue::default();
        q.add_session(named("made"), None, 0);
        q.add_session(named("lost"), None, 1);
        q.add_session(named("waiting"), None, 2);
        q.sessions[0].started = true;
        q.sessions[1].started = true;
        let back = recover(&mut q, &["made".into()]);
        assert_eq!(back, vec!["lost".to_string()]);
        assert_eq!(q.names(), vec!["lost", "waiting"]);
        assert!(q.sessions.iter().all(|s| !s.started));
    }

    #[test]
    fn the_file_survives_a_round_trip_and_keeps_its_counter() {
        let dir = std::env::temp_dir().join(format!("hura-queue-{}", std::process::id()));
        let path = dir.join("queue.json");
        let a = update_at(&path, |q| q.add_message("s", "hi", None, 1)).unwrap();
        update_at(&path, |q| q.remove(a)).unwrap();
        let b = update_at(&path, |q| q.add_message("s", "again", None, 2)).unwrap();
        assert!(b > a);
        let q = load_at(&path).unwrap();
        assert_eq!(q.messages.len(), 1);
        assert_eq!(q.messages[0].text, "again");
        let _ = fs::remove_dir_all(dir);
    }
}
