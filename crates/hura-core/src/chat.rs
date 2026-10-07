//! A chat session: the agent driven through the Agent SDK instead of a terminal.
//!
//! **The SDK runs inside the sandbox.** `images/hura-base/hura-agent.mjs` is a
//! host process the image carries, and it is what owns the agent: one or more
//! conversations, each a `query()` running the image's own `claude`. Putting the
//! SDK on the server instead would have meant a Claude Code outside the policy,
//! which is the one arrangement this tool exists to prevent.
//!
//! What this module owns is the edge of that process as the rest of hura sees
//! it. The seeder starts it under tmux, as it starts a terminal agent, so it
//! outlives every connection. A client reaches it on [`PORT`], a loopback port
//! in the sandbox, through the forward a preview already uses; `hurad` speaks
//! for the client there, and the frames are the types below. Everything that
//! only needs an answer (which conversations exist, open one, close one, say
//! something to the agent) is an exec of `hura-agent` itself, like the shells.
//!
//! **The SDK's messages are carried, not modelled.** They are Claude Code's, and
//! they grow with every release; the transcript keeps them whole, and this side
//! types only the envelope around them. The same reasoning [`crate::usage`]
//! gives for the status line payload.
//!
//! The status poll needs nothing from here. The host writes `status.json` and
//! `usage.json` in the shapes the hooks and the status line write for a
//! terminal agent, so a chat session reads like any other in the list.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::backend::Backend;
use crate::seed::sh_quote;
use crate::session::Session;

/// The loopback port `hura-agent` listens on inside the sandbox. Kept in step
/// with `PORT` in `hura-agent.mjs`; a test reads that file to check.
pub const PORT: u16 = 47_681;

/// The conversation the seeder starts with the task, and the one that cannot be
/// closed: it is the agent, in the way the terminal agent's tmux session is.
pub const AGENT: &str = "agent";

/// The prefix of every other conversation's name, which the host chooses.
pub const PREFIX: &str = "chat-";

/// What a session's agent is.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interface {
    /// Claude Code's own terminal interface, in tmux. What every session was
    /// before chat sessions, and so what a record without the field is.
    #[default]
    Terminal,
    /// The Agent SDK behind `hura-agent`, drawn by the window.
    Chat,
}

impl Interface {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "terminal" => Some(Interface::Terminal),
            "chat" => Some(Interface::Chat),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Interface::Terminal => "terminal",
            Interface::Chat => "chat",
        }
    }
}

/// What `hurad` sends first on a connection to the host: which conversation it
/// is watching, and the last entry it already has. A reconnect asks for what
/// it missed rather than the whole transcript again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attach {
    pub op: &'static str,
    pub conv: String,
    pub since: u64,
}

impl Attach {
    pub fn new(conv: &str, since: u64) -> Self {
        Attach {
            op: "attach",
            conv: conv.to_string(),
            since,
        }
    }
}

/// Something a person does in a conversation.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum ChatCommand {
    /// A message. Queued behind the current turn when there is one, which is
    /// what the SDK does with a message that arrives mid-turn.
    Send { text: String },
    /// Stop the turn in progress.
    Interrupt,
    /// Answer a permission request, a question or a plan.
    Answer { id: String, decision: ChatDecision },
    /// The permission mode, by Claude Code's own name for it.
    Mode { mode: String },
    /// The model, or `None` for Claude Code's default.
    Model { model: Option<String> },
}

/// The answer to one [`ChatAsk`].
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatDecision {
    pub allow: bool,
    /// Do not ask again for calls like this one, as the SDK suggests the rule.
    #[serde(default)]
    pub always: bool,
    /// Said to the agent with a denial: what to do instead.
    #[serde(default)]
    pub message: Option<String>,
    /// For a question: the label chosen, keyed by the question it answers.
    #[serde(default)]
    pub answers: Option<BTreeMap<String, String>>,
    /// For a plan: the mode to carry on in once it is approved.
    #[serde(default)]
    pub mode: Option<String>,
}

/// What the host says on a watching connection.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "kebab-case")]
pub enum ChatFrame {
    /// The transcript after the entry asked for, once, on attaching.
    Replay {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        seq: u64,
        entries: Vec<ChatEntry>,
    },
    /// One more entry in the transcript.
    Entry { entry: ChatEntry },
    /// The reply as it is being written: an SDK `stream_event`. Never kept,
    /// since the finished message that follows supersedes it.
    Partial {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        ev: serde_json::Value,
    },
    /// A tool still running, and for how long: an SDK `tool_progress`.
    Progress {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        ev: serde_json::Value,
    },
    /// The conversation's state, whenever it changes.
    State { state: ChatState },
    /// A request on this connection could not be done.
    Error { message: String },
    /// Said by `hurad` rather than the host: why there is nothing yet. A
    /// session still cloning has no host to talk to.
    Notice { text: String },
}

/// One thing that happened in a conversation, as the transcript keeps it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatEntry {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub seq: u64,
    /// Epoch milliseconds, from the sandbox's clock.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub at: u64,
    pub ev: ChatEvent,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ChatEvent {
    /// A message from the SDK, whole.
    Sdk {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        msg: serde_json::Value,
    },
    /// What you said, as you said it.
    Prompt { text: String, uuid: String },
    /// How a permission request was answered.
    Answer {
        id: String,
        tool: String,
        allow: bool,
        always: bool,
        message: Option<String>,
        /// A question's choices, by the question they answer. Kept here
        /// because the tool call in the transcript is as the agent made it,
        /// before the answers went into it.
        #[serde(default)]
        answers: Option<BTreeMap<String, String>>,
    },
    /// The conversation failed, in the SDK's words.
    Error { message: String },
    /// A kind a newer host writes that this one has not heard of. Read as
    /// nothing rather than failing the whole transcript.
    #[serde(other)]
    Unknown,
}

/// Where a conversation is, as one value.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatActivity {
    #[default]
    Idle,
    Running,
    /// Something in [`ChatState::pending`] is waiting on you.
    Waiting,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatState {
    pub activity: ChatActivity,
    /// Permission requests, questions and plans not yet answered, oldest first.
    #[serde(default)]
    pub pending: Vec<ChatAsk>,
    /// The permission mode, by Claude Code's name for it.
    pub mode: Option<String>,
    pub model: Option<String>,
    /// The models this account can switch to, once Claude Code has said.
    #[serde(default)]
    pub models: Vec<ChatModel>,
    /// Slash commands Claude Code offers, without the slash.
    #[serde(default)]
    pub commands: Vec<String>,
    /// What Claude Code is doing that is not a turn: `compacting`.
    pub status: Option<String>,
    pub cost_usd: Option<f64>,
    pub context_percentage: Option<f64>,
    /// Whether Claude Code is running for this conversation. One that has not
    /// been spoken to since the host started is not, until it is.
    #[serde(default)]
    pub started: bool,
}

/// A permission request, a question, or a plan to approve.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatAsk {
    /// The tool call's id, which is also what the answer names.
    pub id: String,
    pub tool: String,
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub input: serde_json::Value,
    /// Claude Code's own sentence for the request, when it has one.
    pub title: Option<String>,
    pub description: Option<String>,
    pub display_name: Option<String>,
    /// Why it is asking rather than deciding.
    pub reason: Option<String>,
    pub blocked_path: Option<String>,
    /// Whether "don't ask again" is on offer: the SDK suggested a rule, and
    /// the request does not forbid one.
    #[serde(default)]
    pub can_remember: bool,
    /// Open on the refusal: approving this must not be one stray keystroke.
    #[serde(default)]
    pub default_to_no: bool,
    /// The subagent asking, when it is not the main thread.
    pub agent: Option<String>,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatModel {
    pub value: String,
    pub display_name: String,
    pub description: String,
}

/// The script that starts the host, for the seeder's agent step.
///
/// The same shape as the terminal agent's: a tmux session named for the agent,
/// left alone if it exists, with the command typed into a shell so the pane
/// survives it. What is typed is a loop rather than the host alone. The host
/// keeps everything it needs on disk, so starting it again is the whole of
/// recovering from a crash, and nothing else would.
pub fn start_script(backend: &dyn Backend, session: &Session) -> String {
    let paths = backend.paths(session);
    let serve = format!(
        "while :; do hura-agent serve --port {PORT} --cwd {repo} --task-file {task}; \
         echo 'hura-agent stopped; starting it again'; sleep 2; done",
        repo = sh_quote(&paths.repo),
        task = sh_quote(&paths.task()),
    );
    format!(
        r#"set -eu
export LANG=C.UTF-8 LC_ALL=C.UTF-8 COLORTERM=truecolor
if {tmux_bin} has-session -t {tmux} 2>/dev/null; then
  exit 0
fi
mkdir -p {hura}
printf '%s' {task} > {task_path}
{tmux_bin} new-session -d -s {tmux} -c {repo}
{tmux_bin} send-keys -t {tmux} {serve} Enter
"#,
        tmux_bin = backend.tmux(),
        tmux = sh_quote(&session.tmux),
        hura = sh_quote(&paths.hura),
        task = sh_quote(&session.task),
        task_path = sh_quote(&paths.task()),
        repo = sh_quote(&paths.repo),
        serve = sh_quote(&serve),
    )
}

/// Run `hura-agent` in the sandbox, and answer with what it printed.
fn run(backend: &dyn Backend, session: &Session, script: &str) -> Result<String, String> {
    let out = backend
        .exec(session, &["sh", "-c", script])
        .map_err(|e| e.to_string())?;
    if !out.ok() {
        // The host's own words, which say what was wrong; `hura-agent: ` in
        // front of them says who.
        let why = out.stderr.trim();
        return Err(if why.is_empty() {
            format!("hura-agent exited with {}", out.exit_code)
        } else {
            why.to_string()
        });
    }
    Ok(out.trimmed().to_string())
}

/// The conversations a chat session has, the agent's first.
///
/// Asked of the host rather than remembered here, for the reason the shells
/// are asked of tmux: it outlives the window, the server and a second client.
pub fn list(backend: &dyn Backend, session: &Session) -> Result<Vec<String>, String> {
    let text = run(backend, session, "hura-agent list")?;
    let mut names: Vec<String> =
        serde_json::from_str(&text).map_err(|e| format!("hura-agent answered `{text}`: {e}"))?;
    names.sort_by_key(|a| order(a));
    Ok(names)
}

/// The agent first, then the rest by number, so `chat-10` follows `chat-9`.
fn order(name: &str) -> (bool, u64, String) {
    let n = name
        .strip_prefix(PREFIX)
        .and_then(|n| n.parse().ok())
        .unwrap_or(u64::MAX);
    (name != AGENT, n, name.to_string())
}

/// Open another conversation, and answer with its name. Named by the host, for
/// the reason the shells are named by the server: two windows asking at once
/// would otherwise both pick `chat-2`.
pub fn open(backend: &dyn Backend, session: &Session) -> Result<String, String> {
    run(backend, session, "hura-agent new")
}

/// End a conversation and forget its transcript.
pub fn close(backend: &dyn Backend, session: &Session, name: &str) -> Result<(), String> {
    // Checked here as well as by the host, and before anything reaches a
    // shell: the agent is not a conversation to close, and a name is not
    // trusted because a client sent it.
    if !is_closable(name) {
        return Err(format!("`{name}` is not a conversation that can be closed"));
    }
    run(
        backend,
        session,
        &format!("hura-agent close {}", sh_quote(name)),
    )
    .map(drop)
}

fn is_closable(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Say something to the agent's conversation, as you.
///
/// What [`crate::ops::tell`] is for a chat session: a review goes in as one
/// message, the way a bracketed paste puts it into a terminal.
pub fn tell(backend: &dyn Backend, session: &Session, message: &str) -> Result<(), String> {
    run(
        backend,
        session,
        &format!(
            "printf '%s' {} | hura-agent send {}",
            sh_quote(message),
            sh_quote(AGENT)
        ),
    )
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::testing::sandboxed;

    const HOST: &str = include_str!("../../../images/hura-base/hura-agent.mjs");

    /// The port and the agent's name are said twice, once on each side of the
    /// forward. A mismatch is a window that waits for ever on a host listening
    /// somewhere else.
    #[test]
    fn the_host_agrees_about_the_port_and_the_agent() {
        assert!(
            HOST.contains(&format!("export const PORT = {PORT};")),
            "hura-agent.mjs listens somewhere else"
        );
        assert!(HOST.contains(&format!("export const AGENT = \"{AGENT}\";")));
        assert!(HOST.contains(&format!("`{PREFIX}${{n}}`")));
    }

    #[test]
    fn a_record_without_an_interface_is_a_terminal_session() {
        assert_eq!(Interface::default(), Interface::Terminal);
        for i in [Interface::Terminal, Interface::Chat] {
            assert_eq!(Interface::parse(i.as_str()), Some(i));
        }
        assert_eq!(Interface::parse("tui"), None);
    }

    /// The host runs in the agent's tmux session, so everything that waits for
    /// the agent waits for it, and it restarts itself rather than leaving a
    /// session with no agent after a crash.
    #[test]
    fn the_start_script_runs_the_host_in_a_loop_in_the_agents_session() {
        let mut s = Session::new("s".into(), "https://github.com/o/r".into(), "fix it".into());
        s.interface = Interface::Chat;
        let script = start_script(&sandboxed(), &s);
        assert!(script.contains("has-session -t 'agent'"), "{script}");
        assert!(script.contains("new-session -d -s 'agent'"), "{script}");
        assert!(
            script.contains("while :; do hura-agent serve --port 47681"),
            "{script}"
        );
        assert!(
            script.contains("--task-file '\\''/sandbox/.hura/task.txt'\\''"),
            "{script}"
        );
        assert!(script.contains("printf '%s' 'fix it' > '/sandbox/.hura/task.txt'"));
    }

    #[test]
    fn only_a_numbered_conversation_can_be_closed() {
        assert!(is_closable("chat-1"));
        assert!(is_closable("chat-12"));
        assert!(!is_closable(AGENT));
        assert!(!is_closable("chat-"));
        assert!(!is_closable("chat-1; rm -rf /"));
        assert!(!is_closable("shell-1"));
    }

    #[test]
    fn the_agent_comes_first_and_the_rest_by_number() {
        let mut names = vec!["chat-10", "chat-2", "agent", "chat-9"];
        names.sort_by_key(|a| order(a));
        assert_eq!(names, ["agent", "chat-2", "chat-9", "chat-10"]);
    }

    /// Captured from the host, so the two sides cannot drift on a field name
    /// without a test saying so.
    #[test]
    fn frames_from_the_host_read() {
        let replay = r#"{"t":"replay","seq":3,"entries":[
            {"seq":1,"at":1790000000000,"ev":{"kind":"prompt","text":"hi","uuid":"u1"}},
            {"seq":2,"at":1790000000100,"ev":{"kind":"sdk","msg":{"type":"assistant","message":{}}}},
            {"seq":3,"at":1790000000200,"ev":{"kind":"answer","id":"t1","tool":"Bash","allow":true,"always":false,"message":null}},
            {"seq":4,"at":1790000000300,"ev":{"kind":"from-the-future","x":1}}
        ]}"#;
        let ChatFrame::Replay { seq, entries } = serde_json::from_str(replay).unwrap() else {
            panic!("not a replay");
        };
        assert_eq!(seq, 3);
        assert_eq!(entries.len(), 4);
        assert!(matches!(entries[0].ev, ChatEvent::Prompt { ref text, .. } if text == "hi"));
        assert!(matches!(entries[3].ev, ChatEvent::Unknown));

        let state = r#"{"t":"state","state":{"activity":"waiting","pending":[{"id":"t1","tool":"Bash",
            "input":{"command":"ls"},"title":null,"description":null,"display_name":null,"reason":null,
            "blocked_path":null,"can_remember":true,"default_to_no":false,"agent":null}],
            "mode":"auto","model":"claude-opus-5-5","models":[],"commands":["compact"],"status":null,
            "cost_usd":0.1,"context_percentage":4,"started":true}}"#;
        let ChatFrame::State { state } = serde_json::from_str(state).unwrap() else {
            panic!("not a state");
        };
        assert_eq!(state.activity, ChatActivity::Waiting);
        assert_eq!(state.pending[0].tool, "Bash");
        assert!(state.pending[0].can_remember);
    }

    #[test]
    fn commands_go_to_the_host_in_its_own_words() {
        let answer = ChatCommand::Answer {
            id: "t1".into(),
            decision: ChatDecision {
                allow: false,
                message: Some("run the tests first".into()),
                ..ChatDecision::default()
            },
        };
        let v = serde_json::to_value(&answer).unwrap();
        assert_eq!(v["op"], "answer");
        assert_eq!(v["decision"]["allow"], false);
        assert_eq!(v["decision"]["message"], "run the tests first");
        assert_eq!(
            serde_json::to_value(ChatCommand::Interrupt).unwrap()["op"],
            "interrupt"
        );
        assert_eq!(
            serde_json::to_value(Attach::new("chat-1", 7)).unwrap(),
            serde_json::json!({"op": "attach", "conv": "chat-1", "since": 7})
        );
    }
}
