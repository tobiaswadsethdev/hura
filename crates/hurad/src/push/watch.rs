//! What is worth a notification, decided on the server.
//!
//! The desktop application decides this for itself, from the status channel
//! it already has open for every session (`notify.ts`). A phone has nothing
//! open while it is in a pocket, so the server has to watch instead: every
//! few seconds, each live session's state, read the same way the status
//! channel reads it ([`ops::poll`]), so the two cannot disagree about what
//! "waiting" means.
//!
//! **On the transition, not on the state**, for the reason `notify.ts`
//! gives: a session sits in `waiting` until somebody answers it, and a
//! notification every few seconds for as long as it waits would be a phone
//! nobody keeps notifications on for. And not on the first look either: a
//! server starting, or push being switched on, with three sessions already
//! waiting has learnt something rather than seen it happen.
//!
//! **Only while somebody is listening.** A poll is an exec into the sandbox,
//! and a server with no routes has no reason to pay for one.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use hura_core::session::{Session, State};
use hura_core::{ops, store::Store};

use super::{
    CLAUDE_FAILED, CLAUDE_NEEDS_INPUT, CLAUDE_PERMISSION, CLAUDE_TURN_FINISHED, Notification,
    Pusher,
};
use crate::serve::Server;

/// How often every live session is looked at. Slower than the status
/// channel's poll, because this is one exec per session whether or not a
/// window is open, and a notification a few seconds late is still on time.
const EVERY: Duration = Duration::from_secs(5);

/// One session as it was seen on one look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub name: String,
    /// The record's state: `failed` is the one that matters here.
    pub record: State,
    /// What the agent is doing, when the poll could tell.
    pub agent: Option<State>,
    /// The tool, or Claude Code's own sentence, while it waits.
    pub detail: Option<String>,
    /// The id of the request it waits on, when its host says: chat sessions
    /// only.
    pub ask: Option<String>,
    /// What that request asks, in one line: `Bash: npm test`.
    pub prompt: Option<String>,
}

/// What a notification says, before it has an id or a subtitle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    pub kind: &'static str,
    pub session: String,
    pub body: String,
    /// The request a [`CLAUDE_PERMISSION`] notification can answer.
    pub ask: Option<String>,
}

/// Tools that ask a question rather than for permission. Answering one takes
/// a choice or a mode, which an Allow on a lock screen cannot carry, so a wait
/// on one is said as plain input needed.
const QUESTIONS: [&str; 2] = ["AskUserQuestion", "ExitPlanMode"];

/// One session as last seen.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Last {
    record: State,
    agent: Option<State>,
    ask: Option<String>,
}

/// The states last seen, which is what tells a transition from a state.
#[derive(Debug, Default)]
pub struct Watch {
    seen: Option<HashMap<String, Last>>,
}

impl Watch {
    /// Forget everything, so the next look is a first look again.
    pub fn reset(&mut self) {
        self.seen = None;
    }

    /// Take one look, and say what has just happened.
    pub fn look(&mut self, now: &[Seen]) -> Vec<Said> {
        let first = self.seen.is_none();
        let before = self.seen.take().unwrap_or_default();
        let mut said = Vec::new();
        let mut after = HashMap::new();

        for s in now {
            let was = before.get(&s.name);
            // A poll that could not tell -- a sandbox that did not answer this
            // once -- is not a change. Treating it as one would make every
            // hiccup in a long wait a fresh `waiting` on the way back.
            let (agent, ask) = match s.agent {
                Some(a) => (Some(a), s.ask.clone()),
                None => (was.and_then(|w| w.agent), was.and_then(|w| w.ask.clone())),
            };
            after.insert(
                s.name.clone(),
                Last {
                    record: s.record,
                    agent,
                    ask,
                },
            );
            if first {
                continue;
            }
            if let Some(it) = said_about(s, was) {
                said.push(it);
            }
        }
        self.seen = Some(after);
        said
    }
}

/// What one session's change, if it is one, is worth saying.
fn said_about(s: &Seen, was: Option<&Last>) -> Option<Said> {
    let said = |kind, body: String, ask| Said {
        kind,
        session: s.name.clone(),
        body,
        ask,
    };
    if s.record == State::Failed && was.map(|w| w.record) != Some(State::Failed) {
        return Some(said(CLAUDE_FAILED, "The session failed".into(), None));
    }
    let was_agent = was.and_then(|w| w.agent);
    match s.agent? {
        State::Waiting => {
            // Newly waiting -- and a session created a moment ago counts if
            // it already is: it did just start. Or still waiting, but on a
            // request other than the one already said: the last one was
            // answered, perhaps from the phone, and the next came straight
            // after it.
            let fresh = was_agent != Some(State::Waiting)
                || matches!((was.and_then(|w| w.ask.as_deref()), s.ask.as_deref()), (Some(a), Some(b)) if a != b);
            if !fresh {
                return None;
            }
            let tool = s.detail.as_deref().unwrap_or_default();
            Some(match &s.ask {
                // What is being allowed, when the host said, since a lock
                // screen Allow is only an answer if the screen says that.
                Some(id) if !QUESTIONS.contains(&tool) => said(
                    CLAUDE_PERMISSION,
                    s.prompt
                        .clone()
                        .unwrap_or_else(|| asking(s.detail.as_deref())),
                    Some(id.clone()),
                ),
                _ => said(CLAUDE_NEEDS_INPUT, asking(s.detail.as_deref()), None),
            })
        }
        State::Idle if matches!(was_agent, Some(State::Running | State::Waiting)) => Some(said(
            CLAUDE_TURN_FINISHED,
            "Claude finished and is waiting for you".into(),
            None,
        )),
        _ => None,
    }
}

/// The body of a `waiting` notification.
///
/// A chat session's detail is the tool it is asking about; a terminal
/// session's is that, or Claude Code's own sentence when the hook had one
/// ("Claude needs your permission to use Bash"), which needs no help.
fn asking(detail: Option<&str>) -> String {
    match detail.map(str::trim).filter(|d| !d.is_empty()) {
        Some(tool) if !tool.contains(char::is_whitespace) => {
            format!("Claude is asking to use {tool}")
        }
        Some(sentence) => sentence.to_string(),
        None => "Claude needs your input".into(),
    }
}

/// One look at every session the store knows.
fn observe() -> Vec<Seen> {
    let Ok(store) = Store::load() else {
        return Vec::new();
    };
    let backends = crate::rpc::backends();
    store
        .list()
        .into_iter()
        .map(|s: &Session| {
            // Nothing to ask while it is being made or once it is gone, and
            // a failed one has said all it is going to.
            let report = match s.state {
                State::Creating | State::Seeding | State::Failed | State::Dead => None,
                _ => ops::poll(backends.for_session(s), s).status,
            };
            Seen {
                name: s.name.clone(),
                record: s.state,
                agent: report.as_ref().map(|r| r.state),
                ask: report.as_ref().and_then(|r| r.ask.clone()),
                prompt: report.as_ref().and_then(|r| r.prompt.clone()),
                detail: report.and_then(|r| r.detail),
            }
        })
        .collect()
}

/// Watch until the process ends.
pub async fn run(server: Arc<Server>, pusher: Arc<Pusher>) {
    let mut watch = Watch::default();
    // Under each session's name, which machine it is on.
    let host = crate::default_host();
    loop {
        tokio::time::sleep(EVERY).await;

        // A revoked pairing takes its route with it before anything else is
        // said to that phone.
        if let Some(live) = server.token_hashes() {
            pusher.prune(&live);
        }
        if !pusher.has_routes() {
            watch.reset();
            continue;
        }

        let Ok(seen) = tokio::task::spawn_blocking(observe).await else {
            continue;
        };
        for said in watch.look(&seen) {
            pusher.enqueue(
                Notification {
                    event_id: pusher.event_id(),
                    kind: said.kind.into(),
                    title: said.session.clone(),
                    subtitle: host.clone(),
                    body: said.body,
                    session: Some(said.session),
                    ask_id: said.ask,
                },
                None,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(name: &str, record: State, agent: Option<State>) -> Seen {
        Seen {
            name: name.into(),
            record,
            agent,
            detail: None,
            ask: None,
            prompt: None,
        }
    }

    fn asking_for(name: &str, tool: &str, id: &str) -> Seen {
        Seen {
            detail: Some(tool.into()),
            ask: Some(id.into()),
            ..seen(name, State::Running, Some(State::Waiting))
        }
    }

    fn kinds(said: &[Said]) -> Vec<(&str, &str)> {
        said.iter().map(|s| (s.kind, s.session.as_str())).collect()
    }

    #[test]
    fn the_first_look_only_learns() {
        let mut w = Watch::default();
        let said = w.look(&[seen("a", State::Running, Some(State::Waiting))]);
        assert!(said.is_empty());
    }

    #[test]
    fn starting_to_wait_is_said_once() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        let mut waiting = seen("a", State::Running, Some(State::Waiting));
        waiting.detail = Some("Bash".into());
        let said = w.look(std::slice::from_ref(&waiting));
        assert_eq!(kinds(&said), [(CLAUDE_NEEDS_INPUT, "a")]);
        assert_eq!(said[0].body, "Claude is asking to use Bash");
        assert!(w.look(&[waiting]).is_empty(), "still waiting is not news");
    }

    #[test]
    fn a_new_session_already_waiting_counts() {
        let mut w = Watch::default();
        w.look(&[]);
        let said = w.look(&[seen("new", State::Running, Some(State::Waiting))]);
        assert_eq!(kinds(&said), [(CLAUDE_NEEDS_INPUT, "new")]);
    }

    #[test]
    fn a_finished_turn_is_said() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        let said = w.look(&[seen("a", State::Running, Some(State::Idle))]);
        assert_eq!(kinds(&said), [(CLAUDE_TURN_FINISHED, "a")]);
        // Idle to idle is nothing; nor is a session first seen idle.
        assert!(
            w.look(&[
                seen("a", State::Running, Some(State::Idle)),
                seen("b", State::Running, Some(State::Idle))
            ])
            .is_empty()
        );
    }

    #[test]
    fn a_poll_that_could_not_tell_changes_nothing() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Waiting))]);
        assert!(w.look(&[seen("a", State::Running, None)]).is_empty());
        assert!(
            w.look(&[seen("a", State::Running, Some(State::Waiting))])
                .is_empty(),
            "the wait carried on through the gap"
        );
    }

    #[test]
    fn failing_is_said_once() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Seeding, None)]);
        let said = w.look(&[seen("a", State::Failed, None)]);
        assert_eq!(kinds(&said), [(CLAUDE_FAILED, "a")]);
        assert!(w.look(&[seen("a", State::Failed, None)]).is_empty());
    }

    #[test]
    fn a_chat_sessions_permission_request_can_be_answered() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        let said = w.look(&[asking_for("a", "Bash", "toolu_1")]);
        assert_eq!(kinds(&said), [(CLAUDE_PERMISSION, "a")]);
        assert_eq!(said[0].ask.as_deref(), Some("toolu_1"));
        assert_eq!(said[0].body, "Claude is asking to use Bash");

        // With the host's line for it, that is what the phone shows.
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        let said = w.look(&[Seen {
            prompt: Some("Bash: npm test".into()),
            ..asking_for("a", "Bash", "toolu_1")
        }]);
        assert_eq!(said[0].body, "Bash: npm test");
    }

    #[test]
    fn the_next_request_straight_after_is_said_too() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        w.look(&[asking_for("a", "Bash", "toolu_1")]);
        assert!(w.look(&[asking_for("a", "Bash", "toolu_1")]).is_empty());
        let said = w.look(&[asking_for("a", "Edit", "toolu_2")]);
        assert_eq!(said[0].ask.as_deref(), Some("toolu_2"));
    }

    #[test]
    fn a_question_is_not_offered_as_a_permission() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        let said = w.look(&[asking_for("a", "AskUserQuestion", "toolu_1")]);
        assert_eq!(kinds(&said), [(CLAUDE_NEEDS_INPUT, "a")]);
        assert_eq!(said[0].ask, None);
    }

    #[test]
    fn a_request_seen_through_a_failed_poll_is_not_said_again() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        w.look(&[asking_for("a", "Bash", "toolu_1")]);
        assert!(w.look(&[seen("a", State::Running, None)]).is_empty());
        assert!(w.look(&[asking_for("a", "Bash", "toolu_1")]).is_empty());
    }

    #[test]
    fn a_reset_forgets() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        w.reset();
        assert!(
            w.look(&[seen("a", State::Running, Some(State::Waiting))])
                .is_empty()
        );
    }

    #[test]
    fn the_body_reads_either_kind_of_detail() {
        assert_eq!(asking(Some("Edit")), "Claude is asking to use Edit");
        assert_eq!(
            asking(Some("Claude needs your permission to use Bash")),
            "Claude needs your permission to use Bash"
        );
        assert_eq!(asking(Some("  ")), "Claude needs your input");
        assert_eq!(asking(None), "Claude needs your input");
    }
}
