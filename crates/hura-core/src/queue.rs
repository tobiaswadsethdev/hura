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
//! * **A repeating session** is one of the first kind that comes round again:
//!   a template that never starts itself. Each time its time comes a copy of
//!   it is started, named afresh, and its time moves on to the next by its
//!   [`Repeat`]. A server that was down when one was due starts it once on its
//!   way back, not once for every time it missed.
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

/// How a repeating session comes round. T3 Code's two kinds of schedule.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Repeat {
    /// Every so many minutes, counted from the time it was due.
    Interval { minutes: u32 },
    /// At a time of day on the server's clock, on the days of the week given
    /// (0 is Sunday), or every day when none are.
    FixedTime {
        hour: u8,
        minute: u8,
        #[serde(default)]
        weekdays: Vec<u8>,
    },
}

/// The least an interval can be. A worktree is a sandbox and a clone, which
/// take minutes to make; one started more often than this is a mistake, and
/// one that would pile up sandboxes nobody is reading.
pub const LEAST_INTERVAL_MINUTES: u32 = 15;

impl Repeat {
    /// Why it cannot be kept, when it cannot.
    pub fn check(&self) -> Result<(), String> {
        match self {
            Repeat::Interval { minutes } if *minutes < LEAST_INTERVAL_MINUTES => Err(format!(
                "a worktree can come round every {LEAST_INTERVAL_MINUTES} minutes at the most often"
            )),
            Repeat::FixedTime { hour, minute, .. } if *hour > 23 || *minute > 59 => {
                Err(format!("{hour}:{minute:02} is not a time of day"))
            }
            Repeat::FixedTime { weekdays, .. } if weekdays.iter().any(|d| *d > 6) => {
                Err("a day of the week is 0 (Sunday) to 6".into())
            }
            _ => Ok(()),
        }
    }

    /// The first time after `after` (seconds since the epoch) it comes round,
    /// on this machine's clock.
    pub fn next_after(&self, after: u64) -> Option<u64> {
        self.next_after_in(&chrono::Local, after)
    }

    /// The same in a time zone given, so the rule is tested without the
    /// machine's.
    pub fn next_after_in<Tz: chrono::TimeZone>(&self, tz: &Tz, after: u64) -> Option<u64> {
        use chrono::{Datelike, Duration, NaiveTime};
        match self {
            Repeat::Interval { minutes } => {
                Some(after + u64::from((*minutes).max(LEAST_INTERVAL_MINUTES)) * 60)
            }
            Repeat::FixedTime {
                hour,
                minute,
                weekdays,
            } => {
                let time = NaiveTime::from_hms_opt(u32::from(*hour), u32::from(*minute), 0)?;
                let start = tz.timestamp_opt(i64::try_from(after).ok()?, 0).single()?;
                // Today and the week after it: some day in eight is one of
                // the days asked for, and a time on it is after `after`.
                (0..=7).find_map(|ahead| {
                    let day = start.date_naive() + Duration::days(ahead);
                    let wanted = weekdays.is_empty()
                        || weekdays.contains(&(day.weekday().num_days_from_sunday() as u8));
                    if !wanted {
                        return None;
                    }
                    // A time the clocks skip on the day they go forward is
                    // not on that day at all; the next day's is.
                    let at = tz.from_local_datetime(&day.and_time(time)).earliest()?;
                    let at = u64::try_from(at.timestamp()).ok()?;
                    (at > after).then_some(at)
                })
            }
        }
    }
}

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
    /// Set on a template that comes round again: see the module note. Its
    /// `at` is always the next time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Repeat>,
    /// How many times a repeating one has started.
    #[serde(default)]
    pub runs: u32,
    /// The name its last start was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
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
    /// Said again and again, on this rule: each time it is said its `at`
    /// moves on to the next, rather than it coming off the queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Repeat>,
    /// How many times a repeating one has been said.
    #[serde(default)]
    pub runs: u32,
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
    /// The server's offset from UTC now, in seconds, filled in for a client
    /// and not kept: a repeating session's time of day is on the server's
    /// clock, and a client on another says so. Absent from a server that
    /// cannot repeat one, which is how a client knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub utc_offset: Option<i32>,
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
        self.add_repeating(new, at, None, now)
    }

    /// Add a session that comes round again on `repeat`, first at `at`.
    pub fn add_repeating(
        &mut self,
        new: NewSession,
        at: Option<u64>,
        repeat: Option<Repeat>,
        now: u64,
    ) -> u64 {
        let id = self.take_id();
        self.sessions.push(QueuedSession {
            id,
            new,
            at,
            added: now,
            started: false,
            failed: None,
            repeat,
            runs: 0,
            last: None,
        });
        id
    }

    /// Start a repeating one once: a copy of it, marked as being started and
    /// named afresh around `taken` and everything else queued, goes on the
    /// queue, and the template's time moves on to the next after `now`. The
    /// copy's id and what to start. `None` for an id that is not a repeating
    /// template.
    pub fn run_once(&mut self, id: u64, taken: &[String], now: u64) -> Option<(u64, NewSession)> {
        let template = self.sessions.iter().find(|s| s.id == id && !s.started)?;
        let repeat = template.repeat.clone()?;
        let others: Vec<String> = self
            .sessions
            .iter()
            .filter(|s| s.id != id)
            .map(|s| s.name().to_string())
            .chain(taken.iter().cloned())
            .collect();
        let mut new = template.new.clone();
        let name = crate::session::unique_name(template.name(), &others);
        new.name = Some(name.clone());
        let due = template.at.unwrap_or(now);
        let next = repeat.next_after(due.max(now));

        let copy = self.take_id();
        self.sessions.push(QueuedSession {
            id: copy,
            new: new.clone(),
            at: Some(now),
            added: now,
            started: true,
            failed: None,
            repeat: None,
            runs: 0,
            last: None,
        });
        let template = self.sessions.iter_mut().find(|s| s.id == id)?;
        template.at = next;
        template.runs += 1;
        template.last = Some(name);
        // A time that will not come round again (no day in a week had one)
        // is a template that never starts: said, rather than kept silently.
        if next.is_none() {
            template.failed = Some("it does not come round again".into());
        }
        Some((copy, new))
    }

    pub fn add_message(&mut self, session: &str, text: &str, at: Option<u64>, now: u64) -> u64 {
        self.add_repeating_message(session, text, at, None, now)
    }

    /// Add a message said again and again on `repeat`, first at `at`.
    pub fn add_repeating_message(
        &mut self,
        session: &str,
        text: &str,
        at: Option<u64>,
        repeat: Option<Repeat>,
        now: u64,
    ) -> u64 {
        let id = self.take_id();
        self.messages.push(QueuedMessage {
            id,
            session: session.to_string(),
            text: text.to_string(),
            at,
            added: now,
            repeat,
            runs: 0,
        });
        id
    }

    /// A message has been said: off the queue, or for a repeating one, on to
    /// its next time after `now`.
    pub fn said(&mut self, id: u64, now: u64) {
        let next = self.messages.iter().find(|m| m.id == id).and_then(|m| {
            m.repeat
                .as_ref()
                .map(|r| r.next_after(m.at.unwrap_or(now).max(now)))
        });
        match next {
            Some(Some(at)) => {
                if let Some(m) = self.messages.iter_mut().find(|m| m.id == id) {
                    m.at = Some(at);
                    m.runs += 1;
                }
            }
            // Not repeating, or a rule that never comes round again.
            _ => {
                self.messages.retain(|m| m.id != id);
            }
        }
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
        q.utc_offset = None;
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
///
/// With `repeat` it comes round again; without `at` its first time is the
/// rule's next.
pub fn add(
    mut new: NewSession,
    at: Option<u64>,
    repeat: Option<Repeat>,
    cfg: &crate::config::Config,
) -> Result<u64, String> {
    let at = match &repeat {
        Some(rule) => {
            rule.check()?;
            Some(match at {
                Some(at) => at,
                None => rule
                    .next_after(now())
                    .ok_or("that schedule never comes round")?,
            })
        }
        None => at,
    };
    let mut taken = crate::store::Store::load()
        .map(|s| s.names())
        .unwrap_or_default();
    taken.extend(load().map(|q| q.names()).unwrap_or_default());
    let draft = new.clone().into_draft(cfg, &taken)?;
    new.name = Some(draft.name);
    new.title = draft.title;
    update(|q| q.add_repeating(new, at, repeat, now())).map_err(|e| e.to_string())
}

/// Queue a message for a session's agent, to be said once it is free and not
/// before `at`, and answer with its id.
///
/// With `repeat` it is said again and again; without `at` its first time is
/// the rule's next.
pub fn say(
    session: &str,
    text: &str,
    at: Option<u64>,
    repeat: Option<Repeat>,
) -> Result<u64, String> {
    if text.trim().is_empty() {
        return Err("nothing to say".into());
    }
    let at = match &repeat {
        Some(rule) => {
            rule.check()?;
            Some(match at {
                Some(at) => at,
                None => rule
                    .next_after(now())
                    .ok_or("that schedule never comes round")?,
            })
        }
        None => at,
    };
    let store = crate::store::Store::load().map_err(|e| e.to_string())?;
    if !store.contains(session) {
        return Err(format!("no session named `{session}`"));
    }
    update(|q| q.add_repeating_message(session, text, at, repeat, now())).map_err(|e| e.to_string())
}

/// The queue as a client is shown it: the file, and the limit it works to.
pub fn view(cfg: &crate::config::Config) -> io::Result<Queue> {
    let mut q = load()?;
    q.limit = cfg.queue_limit();
    q.utc_offset = Some(chrono::Local::now().offset().local_minus_utc());
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

    fn at(tz: &chrono::FixedOffset, day: u32, hour: u32, minute: u32) -> u64 {
        use chrono::TimeZone;
        // November 2026: the 26th is a Thursday, the 27th a Friday, the
        // 30th a Monday.
        tz.with_ymd_and_hms(2026, 11, day, hour, minute, 0)
            .unwrap()
            .timestamp() as u64
    }

    /// A time of day comes round on the next day it falls after, on the
    /// clock the rule is read on, and only on the days asked for.
    #[test]
    fn a_time_of_day_comes_round_on_the_days_asked_for() {
        let tz = chrono::FixedOffset::east_opt(3600).unwrap();
        let daily = Repeat::FixedTime {
            hour: 9,
            minute: 0,
            weekdays: vec![],
        };
        assert_eq!(
            daily.next_after_in(&tz, at(&tz, 26, 8, 0)),
            Some(at(&tz, 26, 9, 0))
        );
        assert_eq!(
            daily.next_after_in(&tz, at(&tz, 26, 9, 0)),
            Some(at(&tz, 27, 9, 0))
        );
        let weekdays = Repeat::FixedTime {
            hour: 9,
            minute: 0,
            weekdays: vec![1, 2, 3, 4, 5],
        };
        assert_eq!(
            weekdays.next_after_in(&tz, at(&tz, 27, 10, 0)),
            Some(at(&tz, 30, 9, 0))
        );
        let sundays = Repeat::FixedTime {
            hour: 23,
            minute: 30,
            weekdays: vec![0],
        };
        assert_eq!(
            sundays.next_after_in(&tz, at(&tz, 26, 10, 0)),
            Some(at(&tz, 29, 23, 30))
        );
    }

    #[test]
    fn an_interval_comes_round_after_it_and_never_more_often_than_the_least() {
        let hourly = Repeat::Interval { minutes: 60 };
        assert_eq!(hourly.next_after(1000), Some(1000 + 3600));
        let too_often = Repeat::Interval { minutes: 5 };
        assert!(too_often.check().is_err());
        assert_eq!(
            too_often.next_after(0),
            Some(u64::from(LEAST_INTERVAL_MINUTES) * 60)
        );
        assert!(
            Repeat::FixedTime {
                hour: 24,
                minute: 0,
                weekdays: vec![]
            }
            .check()
            .is_err()
        );
        assert!(
            Repeat::FixedTime {
                hour: 9,
                minute: 0,
                weekdays: vec![7]
            }
            .check()
            .is_err()
        );
        assert!(
            Repeat::FixedTime {
                hour: 9,
                minute: 0,
                weekdays: vec![0, 6]
            }
            .check()
            .is_ok()
        );
    }

    /// Each start is a copy, named afresh and marked as being started; the
    /// template stays, its time moved on, and a server that was away for a
    /// while starts it once rather than once for each time it missed.
    #[test]
    fn a_repeating_session_starts_copies_and_moves_on() {
        let mut q = Queue::default();
        let hourly = Repeat::Interval { minutes: 60 };
        let id = q.add_repeating(named("nightly"), Some(100), Some(hourly), 0);

        let (first, new) = q.run_once(id, &[], 100).unwrap();
        assert_eq!(new.name.as_deref(), Some("nightly"));
        let template = q.sessions.iter().find(|s| s.id == id).unwrap();
        assert_eq!(
            (template.at, template.runs, template.last.as_deref()),
            (Some(3700), 1, Some("nightly"))
        );
        let copy = q.sessions.iter().find(|s| s.id == first).unwrap();
        assert!(copy.started && copy.repeat.is_none());

        // The first is still there when the second comes round.
        let (_, again) = q.run_once(id, &["nightly".into()], 100_000).unwrap();
        assert_ne!(again.name.as_deref(), Some("nightly"));
        let template = q.sessions.iter().find(|s| s.id == id).unwrap();
        assert_eq!(template.at, Some(100_000 + 3600));
        assert_eq!(template.runs, 2);

        // Not a template: nothing to run once.
        assert!(q.run_once(first, &[], 100_000).is_none());
    }

    /// A file from before repeating sessions reads, and one without any
    /// writes no trace of them.
    #[test]
    fn a_queue_without_repeats_is_written_as_it_was() {
        let mut q = Queue::default();
        q.add_session(named("once"), Some(5), 1);
        let json = serde_json::to_string(&q).unwrap();
        assert!(
            !json.contains("repeat") && !json.contains("last") && !json.contains("utc_offset"),
            "{json}"
        );
        let back: Queue = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sessions[0].repeat, None);
    }

    /// A repeating message stays on the queue, its time moved on each time it
    /// is said; a one-off comes off.
    #[test]
    fn a_repeating_message_moves_on_and_a_one_off_comes_off() {
        let mut q = Queue::default();
        let daily = Repeat::Interval { minutes: 24 * 60 };
        let repeating = q.add_repeating_message("s", "rebase on main", Some(100), Some(daily), 0);
        let once = q.add_message("s", "and push", None, 0);
        q.said(repeating, 100);
        q.said(once, 100);
        assert_eq!(q.messages.len(), 1);
        assert_eq!(q.messages[0].at, Some(100 + 86_400));
        assert_eq!(q.messages[0].runs, 1);
        // Said late, after the server was away: the next is after now.
        q.said(repeating, 1_000_000);
        assert_eq!(q.messages[0].at, Some(1_000_000 + 86_400));
    }
}
