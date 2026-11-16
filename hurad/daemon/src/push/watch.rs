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
//! A session's Live Activity is told the same way, and also only on a
//! change: what the session is doing now, in one word ([`Phase`]), whenever
//! that word changes. The relay decides from it whether the phone's activity
//! starts, moves on or ends.
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
    Pusher, SESSION_ACTIVITY,
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
    /// What a person calls it, which is what a notification is titled with;
    /// the name is for typing and for the phone to open it by.
    pub title: Option<String>,
    /// The project it is filed under, which is what a notification is
    /// subtitled with: a person knows their projects, not their host names.
    pub project: Option<String>,
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
    /// The request a [`CLAUDE_PERMISSION`] notification can answer, or a
    /// Live Activity's Allow and Deny.
    pub ask: Option<String>,
    /// What a [`SESSION_ACTIVITY`] says the session is doing; nothing for a
    /// notification.
    pub phase: Option<Phase>,
    /// When the turn a [`SESSION_ACTIVITY`] shows began, in Unix seconds.
    pub since: Option<u64>,
}

/// What a session is doing, as its Live Activity shows it. The words are the
/// relay's (`docs/relay-api.md`, "Live Activities").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Working,
    /// Waiting on a permission request, which Allow and Deny can answer.
    Permission,
    /// Waiting on anything else: a question, a plan, a terminal's prompt.
    Input,
    Done,
    Failed,
    /// The session is gone.
    Ended,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Working => "working",
            Phase::Permission => "permission",
            Phase::Input => "input",
            Phase::Done => "done",
            Phase::Failed => "failed",
            Phase::Ended => "ended",
        }
    }

    /// A turn under way, which is what a phone has an activity showing for.
    fn live(self) -> bool {
        matches!(self, Phase::Working | Phase::Permission | Phase::Input)
    }
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
    /// What its activity was last told, if anything.
    phase: Option<Phase>,
    /// When the turn its activity shows began: the first live phase, kept
    /// through a wait and the work after it, and gone once the turn is.
    since: Option<u64>,
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
        self.look_at(now, super::now())
    }

    /// [`Watch::look`], with the time it is: `at`, in Unix seconds.
    pub fn look_at(&mut self, now: &[Seen], at: u64) -> Vec<Said> {
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
            let was_phase = was.and_then(|w| w.phase);
            let phase = phase_of(s, was_phase);
            let since = match phase {
                Some(p) if p.live() => was.and_then(|w| w.since).or(Some(at)),
                _ => None,
            };
            after.insert(
                s.name.clone(),
                Last {
                    record: s.record,
                    agent,
                    ask: ask.clone(),
                    phase,
                    since,
                },
            );
            if first {
                continue;
            }
            if let Some(it) = said_about(s, was) {
                said.push(it);
            }
            // A new phase, or a permission request answered and the next
            // one straight after it, whose Allow is for another request.
            let moved = phase != was_phase
                || (phase == Some(Phase::Permission)
                    && was.and_then(|w| w.ask.as_ref()) != ask.as_ref());
            if let Some(p) = phase.filter(|_| moved) {
                // The turn that is over still began when it began.
                said.push(activity(s, p, since.or(was.and_then(|w| w.since))));
            }
        }
        // A session that has gone while its activity showed a turn: the
        // phone would otherwise show it working until the activity expires.
        if !first {
            for (name, w) in &before {
                if !after.contains_key(name) && w.phase.is_some_and(Phase::live) {
                    said.push(Said {
                        kind: SESSION_ACTIVITY,
                        session: name.clone(),
                        body: "The session ended".into(),
                        ask: None,
                        phase: Some(Phase::Ended),
                        since: w.since,
                    });
                }
            }
        }
        self.seen = Some(after);
        said
    }
}

/// What a session's activity shows now: from its record and what the agent
/// is doing, and `was`, what it showed before -- which is also what it shows
/// through a poll that could not tell, as with notifications. Finishing is
/// only `done` after a turn the activity showed, and stopping only `ended`
/// then: an idle session the phone never heard about has nothing to finish.
fn phase_of(s: &Seen, was: Option<Phase>) -> Option<Phase> {
    let live = was.is_some_and(Phase::live);
    match s.record {
        State::Failed => return Some(Phase::Failed),
        State::Dead => return if live { Some(Phase::Ended) } else { was },
        _ => {}
    }
    match s.agent {
        Some(State::Running) => Some(Phase::Working),
        Some(State::Waiting) => Some(match waiting(s) {
            (CLAUDE_PERMISSION, ..) => Phase::Permission,
            _ => Phase::Input,
        }),
        Some(State::Idle) if live => Some(Phase::Done),
        _ => was,
    }
}

/// What a session's activity is told on moving into `phase`.
fn activity(s: &Seen, phase: Phase, since: Option<u64>) -> Said {
    let (body, ask) = match phase {
        Phase::Working => ("Claude is working".into(), None),
        Phase::Permission | Phase::Input => {
            let (_, body, ask) = waiting(s);
            (body, ask)
        }
        Phase::Done => ("Claude finished and is waiting for you".into(), None),
        Phase::Failed => ("The session failed".into(), None),
        Phase::Ended => ("The session ended".into(), None),
    };
    Said {
        kind: SESSION_ACTIVITY,
        session: s.name.clone(),
        body,
        ask,
        phase: Some(phase),
        since,
    }
}

/// What a wait is: a permission request the phone can answer, with what it
/// asks and its id, or input needed and what for.
fn waiting(s: &Seen) -> (&'static str, String, Option<String>) {
    let tool = s.detail.as_deref().unwrap_or_default();
    match &s.ask {
        // What is being allowed, when the host said, since a lock screen
        // Allow is only an answer if the screen says that.
        Some(id) if !QUESTIONS.contains(&tool) => (
            CLAUDE_PERMISSION,
            s.prompt
                .clone()
                .unwrap_or_else(|| asking(s.detail.as_deref())),
            Some(id.clone()),
        ),
        // A question in its own words, when the host said them. An older
        // host's line for one is only the tool's name, which says nothing.
        Some(_) => (
            CLAUDE_NEEDS_INPUT,
            s.prompt
                .clone()
                .filter(|p| !p.trim().is_empty() && p.trim() != tool)
                .unwrap_or_else(|| asking(s.detail.as_deref())),
            None,
        ),
        None => (CLAUDE_NEEDS_INPUT, asking(s.detail.as_deref()), None),
    }
}

/// What one session's change, if it is one, is worth saying.
fn said_about(s: &Seen, was: Option<&Last>) -> Option<Said> {
    let said = |kind, body: String, ask| Said {
        kind,
        session: s.name.clone(),
        body,
        ask,
        phase: None,
        since: None,
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
            let (kind, body, ask) = waiting(s);
            Some(said(kind, body, ask))
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
/// ("Claude needs your permission to use Bash"), which needs no help. A
/// tool's name is for a program: the two that ask rather than act are said
/// as what they are, and an MCP server's tool by its own name and its
/// server's.
fn asking(detail: Option<&str>) -> String {
    match detail.map(str::trim).filter(|d| !d.is_empty()) {
        Some("AskUserQuestion") => "Claude has a question for you".into(),
        Some("ExitPlanMode") => "Claude has a plan for you to review".into(),
        Some(tool) if !tool.contains(char::is_whitespace) => {
            let tool = match tool.strip_prefix("mcp__").and_then(|t| t.split_once("__")) {
                Some((server, name)) if !server.is_empty() && !name.is_empty() => {
                    format!("{name} ({server})")
                }
                _ => tool.to_string(),
            };
            format!("Claude is asking to use {tool}")
        }
        Some(sentence) => sentence.to_string(),
        None => "Claude needs your input".into(),
    }
}

/// One look at every session the store knows, or nothing when the store
/// could not be read: that is not every session having gone, which would end
/// every activity on the phone.
fn observe() -> Option<Vec<Seen>> {
    let store = Store::load().ok()?;
    let backends = crate::rpc::backends();
    let seen = store
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
                title: s.title.clone(),
                project: s.project.clone(),
                record: s.state,
                agent: report.as_ref().map(|r| r.state),
                ask: report.as_ref().and_then(|r| r.ask.clone()),
                prompt: report.as_ref().and_then(|r| r.prompt.clone()),
                detail: report.and_then(|r| r.detail),
            }
        })
        .collect();
    Some(seen)
}

/// What a session's notification is titled with: its title, or its name for
/// one that has none, as the lists show it.
fn title(seen: &[Seen], session: &str) -> String {
    seen.iter()
        .find(|s| s.name == session)
        .and_then(|s| s.title.clone())
        .unwrap_or_else(|| session.to_string())
}

/// What a session's notification is subtitled with: its project, or the
/// machine for one that has none.
fn subtitle(seen: &[Seen], session: &str, host: &str) -> String {
    seen.iter()
        .find(|s| s.name == session)
        .and_then(|s| s.project.clone())
        .unwrap_or_else(|| host.to_string())
}

/// Watch until the process ends.
pub async fn run(server: Arc<Server>, pusher: Arc<Pusher>) {
    let mut watch = Watch::default();
    // Under each session's name, its project, or for one without, which
    // machine it is on. The phone puts its own name for the server first.
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

        let Ok(Some(seen)) = tokio::task::spawn_blocking(observe).await else {
            continue;
        };
        let at = super::now();
        for said in watch.look(&seen) {
            let title = title(&seen, &said.session);
            let subtitle = subtitle(&seen, &said.session, &host);
            pusher.enqueue(
                Notification {
                    event_id: pusher.event_id(),
                    kind: said.kind.into(),
                    title,
                    subtitle,
                    body: said.body,
                    session: Some(said.session),
                    ask_id: said.ask,
                    phase: said.phase.map(|p| p.as_str().into()),
                    at: said.phase.map(|_| at),
                    since: said.since,
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
            title: None,
            project: None,
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

    /// The notifications said, leaving out the activities.
    fn kinds(said: &[Said]) -> Vec<(&str, &str)> {
        said.iter()
            .filter(|s| s.phase.is_none())
            .map(|s| (s.kind, s.session.as_str()))
            .collect()
    }

    /// The activities said, leaving out the notifications.
    fn phases(said: &[Said]) -> Vec<(Phase, &str)> {
        said.iter()
            .filter_map(|s| Some((s.phase?, s.session.as_str())))
            .collect()
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
        assert_eq!(phases(&said), [(Phase::Input, "a")]);
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
    fn a_notification_is_titled_with_the_sessions_title_or_else_its_name() {
        let seen = [
            Seen {
                title: Some("Fix the flaky login test".into()),
                ..seen("fix-flaky-login-test", State::Running, None)
            },
            seen("b", State::Running, None),
        ];
        assert_eq!(
            title(&seen, "fix-flaky-login-test"),
            "Fix the flaky login test"
        );
        assert_eq!(title(&seen, "b"), "b");
        assert_eq!(title(&seen, "gone"), "gone");
    }

    #[test]
    fn a_notification_is_subtitled_with_its_project_or_else_the_machine() {
        let seen = [
            Seen {
                project: Some("scope-creep".into()),
                ..seen("a", State::Running, None)
            },
            seen("b", State::Running, None),
        ];
        assert_eq!(subtitle(&seen, "a", "box"), "scope-creep");
        assert_eq!(subtitle(&seen, "b", "box"), "box");
        assert_eq!(subtitle(&seen, "gone", "box"), "box");
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
    fn a_question_is_said_in_its_own_words() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        let said = w.look(&[Seen {
            prompt: Some("Which database should we use?".into()),
            ..asking_for("a", "AskUserQuestion", "toolu_1")
        }]);
        assert_eq!(said[0].body, "Which database should we use?");

        // An older host's line is only the tool's name.
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        let said = w.look(&[Seen {
            prompt: Some("AskUserQuestion".into()),
            ..asking_for("a", "AskUserQuestion", "toolu_1")
        }]);
        assert_eq!(said[0].body, "Claude has a question for you");
    }

    #[test]
    fn an_activity_follows_a_turn() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Idle))]);
        let said = w.look(&[seen("a", State::Running, Some(State::Running))]);
        assert_eq!(phases(&said), [(Phase::Working, "a")]);
        assert_eq!(said[0].body, "Claude is working");
        assert!(
            w.look(&[seen("a", State::Running, Some(State::Running))])
                .is_empty(),
            "still working is not news"
        );

        let said = w.look(&[Seen {
            prompt: Some("Bash: npm test".into()),
            ..asking_for("a", "Bash", "toolu_1")
        }]);
        let activity = said.iter().find(|s| s.phase.is_some()).unwrap();
        assert_eq!(activity.phase, Some(Phase::Permission));
        assert_eq!(activity.kind, SESSION_ACTIVITY);
        assert_eq!(activity.ask.as_deref(), Some("toolu_1"));
        assert_eq!(activity.body, "Bash: npm test");

        // The next request straight after is another Allow.
        let said = w.look(&[asking_for("a", "Edit", "toolu_2")]);
        assert_eq!(phases(&said), [(Phase::Permission, "a")]);

        let said = w.look(&[seen("a", State::Running, Some(State::Running))]);
        assert_eq!(phases(&said), [(Phase::Working, "a")]);
        let said = w.look(&[seen("a", State::Running, Some(State::Idle))]);
        assert_eq!(phases(&said), [(Phase::Done, "a")]);
        assert!(
            w.look(&[seen("a", State::Running, Some(State::Idle))])
                .is_empty()
        );
    }

    #[test]
    fn a_question_is_input_not_permission() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        let said = w.look(&[asking_for("a", "ExitPlanMode", "toolu_1")]);
        assert_eq!(phases(&said), [(Phase::Input, "a")]);
        let activity = said.iter().find(|s| s.phase.is_some()).unwrap();
        assert_eq!(activity.ask, None);
        assert_eq!(activity.body, "Claude has a plan for you to review");
    }

    #[test]
    fn an_idle_session_nobody_saw_working_has_no_activity() {
        let mut w = Watch::default();
        w.look(&[]);
        assert!(phases(&w.look(&[seen("a", State::Running, Some(State::Idle))])).is_empty());
        // Nor does it end one when it goes.
        assert!(w.look(&[]).is_empty());
    }

    #[test]
    fn a_turn_keeps_its_start_through_a_wait() {
        let since = |said: &[Said]| -> Vec<Option<u64>> {
            said.iter()
                .filter(|s| s.phase.is_some())
                .map(|s| s.since)
                .collect()
        };
        let mut w = Watch::default();
        w.look_at(&[seen("a", State::Running, Some(State::Idle))], 100);
        let said = w.look_at(&[seen("a", State::Running, Some(State::Running))], 105);
        assert_eq!(since(&said), [Some(105)]);
        let said = w.look_at(&[asking_for("a", "Bash", "toolu_1")], 200);
        assert_eq!(since(&said), [Some(105)], "the wait is the same turn");
        let said = w.look_at(&[seen("a", State::Running, Some(State::Running))], 230);
        assert_eq!(since(&said), [Some(105)], "and so is the work after it");
        let said = w.look_at(&[seen("a", State::Running, Some(State::Idle))], 300);
        assert_eq!(since(&said), [Some(105)], "done says when the turn began");
        let said = w.look_at(&[seen("a", State::Running, Some(State::Running))], 400);
        assert_eq!(
            since(&said),
            [Some(400)],
            "the next turn starts its own clock"
        );
    }

    #[test]
    fn a_session_that_goes_mid_turn_ends_its_activity() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        let said = w.look(&[]);
        assert_eq!(phases(&said), [(Phase::Ended, "a")]);
        assert_eq!(said[0].body, "The session ended");

        // And one stopped, which the store still lists as dead.
        let mut w = Watch::default();
        w.look(&[seen("b", State::Running, Some(State::Waiting))]);
        let said = w.look(&[seen("b", State::Dead, None)]);
        assert_eq!(phases(&said), [(Phase::Ended, "b")]);
        assert!(w.look(&[seen("b", State::Dead, None)]).is_empty());
        assert!(w.look(&[]).is_empty(), "already ended");
    }

    #[test]
    fn a_failure_shows_on_the_activity() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        let said = w.look(&[seen("a", State::Failed, None)]);
        assert_eq!(phases(&said), [(Phase::Failed, "a")]);
    }

    #[test]
    fn a_poll_that_could_not_tell_keeps_the_activity() {
        let mut w = Watch::default();
        w.look(&[seen("a", State::Running, Some(State::Running))]);
        assert!(w.look(&[seen("a", State::Running, None)]).is_empty());
        assert!(
            w.look(&[seen("a", State::Running, Some(State::Running))])
                .is_empty()
        );
    }

    #[test]
    fn the_body_reads_either_kind_of_detail() {
        assert_eq!(asking(Some("Edit")), "Claude is asking to use Edit");
        assert_eq!(
            asking(Some("AskUserQuestion")),
            "Claude has a question for you"
        );
        assert_eq!(
            asking(Some("ExitPlanMode")),
            "Claude has a plan for you to review"
        );
        assert_eq!(
            asking(Some("mcp__github__create_issue")),
            "Claude is asking to use create_issue (github)"
        );
        assert_eq!(
            asking(Some("Claude needs your permission to use Bash")),
            "Claude needs your permission to use Bash"
        );
        assert_eq!(asking(Some("  ")), "Claude needs your input");
        assert_eq!(asking(None), "Claude needs your input");
    }
}
