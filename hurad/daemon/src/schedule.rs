//! Starting what the queue says to start, and saying what it says to say.
//!
//! The queue itself is `hura_core::queue`, on disk; this is what acts on it,
//! every few seconds for as long as the server runs. Nothing about it is held
//! here that a restart would lose except when each session was last told
//! something, which only spaces messages out.
//!
//! **Only looks when there is something to look for.** Deciding whether the
//! queue has room is an exec into every live session, the same poll the
//! status channel makes; a server with nothing queued pays for none, and one
//! with only messages queued polls only the sessions they are for.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use hura_core::queue::{self, Seen};
use hura_core::session::{Session, State};
use hura_core::store::Store;
use hura_core::{config, ops, session};

/// How often the queue is looked at. A scheduled session starts within this
/// of its time.
const EVERY: Duration = Duration::from_secs(10);

/// How long after a message a session is left alone, whatever it reads as.
/// An agent just told something can still read as idle for a moment before
/// its turn shows, and a second message in that moment would land with the
/// first rather than after it.
const SETTLE: Duration = Duration::from_secs(30);

/// When each session was last told something.
#[derive(Default)]
pub struct Memory {
    told: HashMap<String, Instant>,
}

/// Look at the queue until the process ends.
pub async fn run() {
    // A server that stopped part-way through starting something finds out
    // what became of it before it starts anything else.
    let _ = tokio::task::spawn_blocking(recover).await;
    let mut memory = Some(Memory::default());
    loop {
        tokio::time::sleep(EVERY).await;
        let Some(mut m) = memory.take() else { break };
        match tokio::task::spawn_blocking(move || {
            tick(&mut m);
            m
        })
        .await
        {
            Ok(m) => memory = Some(m),
            Err(e) => {
                eprintln!("hurad: the queue was not looked at this time: {e}");
                memory = Some(Memory::default());
            }
        }
    }
}

fn recover() {
    let Ok(store) = Store::load() else { return };
    match queue::update(|q| queue::recover(q, &store.names())) {
        Ok(back) => {
            for name in back {
                println!(
                    "queue: `{name}` was not started before the server stopped; it waits again"
                );
            }
        }
        Err(e) => eprintln!("hurad: could not read the queue: {e}"),
    }
}

/// One look: start what is due, say what can be said.
fn tick(memory: &mut Memory) {
    let q = match queue::load() {
        Ok(q) => q,
        Err(e) => {
            eprintln!("hurad: could not read the queue: {e}");
            return;
        }
    };
    if q.sessions.is_empty() && q.messages.is_empty() {
        return;
    }
    let Ok(store) = Store::load() else { return };

    // Messages for a session that has gone are never going to be said.
    let gone: Vec<String> = q
        .told()
        .into_iter()
        .filter(|name| !store.contains(name))
        .map(str::to_string)
        .collect();
    if !gone.is_empty() {
        let _ = queue::update(|q| gone.iter().for_each(|n| q.forget_session(n)));
    }

    let now = queue::now();
    let wanted = q.due_told(now);
    let room = q.wants_room();
    let backends = crate::rpc::backends();
    let seen: Vec<Seen> = store
        .list()
        .into_iter()
        .filter(|s| room || wanted.contains(s.name.as_str()))
        .map(|s| look(&backends, s))
        .collect();

    let limit = config::Config::load()
        .map(|c| c.queue_limit())
        .unwrap_or(queue::DEFAULT_LIMIT);
    for id in q.due(now, &seen, limit) {
        launch(id, &store);
    }

    for m in q.next_messages(now, &seen) {
        if memory
            .told
            .get(&m.session)
            .is_some_and(|at| at.elapsed() < SETTLE)
        {
            continue;
        }
        let Some(s) = store.get(&m.session) else {
            continue;
        };
        match ops::tell(backends.for_session(s), s, &m.text) {
            Ok(()) => {
                memory.told.insert(m.session.clone(), Instant::now());
                let id = m.id;
                if let Err(e) = queue::update(|q| q.remove(id)) {
                    eprintln!(
                        "hurad: {}: told, but could not take it off the queue: {e}",
                        m.session
                    );
                }
            }
            // Left on the queue for the next look: a sandbox that did not
            // answer this once is the usual reason, and it will.
            Err(e) => eprintln!("hurad: {}: a queued message was not said: {e}", m.session),
        }
    }
}

/// One session, as far as the queue cares.
fn look(backends: &hura_core::backend::Backends, s: &Session) -> Seen {
    let (busy, idle) = match s.state {
        State::Creating | State::Seeding => (true, false),
        State::Failed | State::Dead => (false, false),
        _ => match ops::poll(backends.for_session(s), s)
            .status
            .map(|r| r.state)
        {
            Some(State::Running) => (true, false),
            Some(State::Idle) => (false, true),
            // Waiting on a person, or a poll that could not tell: neither
            // takes a place nor hears a message.
            _ => (false, false),
        },
    };
    Seen {
        name: s.name.clone(),
        busy,
        idle,
    }
}

/// Start one queued session on a thread of its own, marked as started until
/// its create has finished.
fn launch(id: u64, store: &Store) {
    let taken = store.names();
    let marked = queue::update(|q| {
        let others: Vec<String> = q
            .sessions
            .iter()
            .filter(|s| s.id != id)
            .map(|s| s.name().to_string())
            .collect();
        let s = q.sessions.iter_mut().find(|s| s.id == id && !s.started)?;
        // Somebody took the name while it waited: a counter, as a derived
        // name gets, rather than a create refused for a clash nobody made.
        if taken.iter().any(|n| n == s.name()) {
            let all: Vec<String> = taken.iter().chain(&others).cloned().collect();
            s.new.name = Some(session::unique_name(s.name(), &all));
        }
        s.started = true;
        Some(s.new.clone())
    });
    let new = match marked {
        Ok(Some(new)) => new,
        Ok(None) => return,
        Err(e) => {
            eprintln!("hurad: could not read the queue: {e}");
            return;
        }
    };

    std::thread::spawn(move || {
        let name = new.name.clone().unwrap_or_default();
        println!("queue: starting `{name}`");
        let cfg = config::Config::load().unwrap_or_default();
        let done = match new.into_draft(&cfg, &taken) {
            Ok(draft) => {
                crate::rpc::start(&draft, &cfg);
                queue::update(|q| q.sessions.retain(|s| s.id != id))
            }
            // Refused now though it was accepted when queued: the config file
            // changed under it. Nothing would make it start later either, so
            // it stays, saying why, until someone deals with it.
            Err(why) => {
                eprintln!("hurad: queue: `{name}` could not start: {why}");
                queue::update(|q| {
                    if let Some(s) = q.sessions.iter_mut().find(|s| s.id == id) {
                        s.started = false;
                        s.failed = Some(why);
                    }
                })
            }
        };
        if let Err(e) = done {
            eprintln!("hurad: queue: `{name}` could not be updated on the queue: {e}");
        }
    });
}
