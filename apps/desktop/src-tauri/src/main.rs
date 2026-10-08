//! The desktop application's Rust half.
//!
//! Thin on purpose. Every command here is a paired server's [`Remote::call`]
//! and a `?`; the reasoning about what a session *is* stays in `hura-core`, and
//! the wire stays in `hura-proto`. A behaviour that existed only here would be
//! one the CLI cannot do, which is the drift the whole split exists to avoid.
//!
//! **The connection is made on this side, and it has to be.** The certificate
//! is pinned by fingerprint, and a webview cannot do that -- `fetch` has no say
//! in which certificate it will accept, and asking a user to click through a
//! warning is how a self-signed server becomes an unauthenticated one. So the
//! webview never speaks to `hurad` at all: it calls these, and `hura-client` --
//! the same client the CLI uses -- makes the connection.
//!
//! ## Every command is `(async)`, and it has to be
//!
//! **Tauri runs a synchronous command on the main thread** -- the one pumping
//! the window's messages -- and every command in this file is blocking I/O: a
//! TLS round trip to `hurad`, which may itself shell out to the gateway CLI. The
//! session list is re-read every three seconds and each read is an
//! `openshell sandbox list` on the other end, so the window spent a large part
//! of every second not pumping anything.
//!
//! Windows says so out loud: the title bar gains *(not responding)* and the
//! window stops repainting. WebKitGTK on Linux has no such watchdog, which is
//! the only reason this survived being built and run there for six increments.
//!
//! `#[tauri::command(async)]` on a synchronous function is Tauri's way of
//! saying "run this on a worker thread": nothing here touches the window
//! handle, and the frontend contract is unchanged -- `invoke` returns a promise
//! either way.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

mod previews;

use hura_client::trackers::Trackers;
use hura_client::{Incoming, Remote, Remotes, Sink};
use hura_core::chat::ChatCommand;
use hura_core::comments::{Comment, NewComment};
use hura_core::events::FeedEvent;
use hura_core::endpoints::Route;
use hura_core::files::{Dir, FileText};
use hura_core::git::{Against, FileDiff, Status as GitStatus};
use hura_core::integrations::View as IntegrationsView;
use hura_core::ops::{NewOptions, NewSession, Picked, Poll};
use hura_core::policy::View as PolicyView;
use hura_core::projects::{NewProject, Project};
use hura_core::repos::Listing;
use hura_core::session::Session;
use hura_core::settings::{Settings, SettingsView};
use hura_core::tracker::{Configured, Inbox, Source as TrackerSource};
use hura_proto::stream::{Channel, ChannelId, ClientFrame, ServerFrame};
use hura_proto::{FailureKind, GitOp, McpOp, Reply, Request};
use serde::Serialize;
use tauri::{Emitter as _, Manager as _};

/// What a failed command looks like in the webview.
///
/// It was a bare string, on the grounds that every one of these is shown to a
/// person rather than branched on, and that when something did need to branch
/// this would grow a field rather than have the UI parse English.
///
/// The `message` is still the server's own words. What the kind decides is how
/// they are drawn.
#[derive(Debug, Clone, Serialize)]
struct Failed {
    kind: FailureKind,
    message: String,
}

fn to_message(e: hura_client::Error) -> Failed {
    let message = e.to_string();
    Failed {
        // A transport error and a reply that was not a reply are both failures
        // of this request; only the server's own `Failure` carries a kind.
        kind: match e {
            hura_client::Error::Failed(f) => f.kind,
            _ => FailureKind::Failed,
        },
        message,
    }
}

/// The same shape for a failure this side produced: no pairing, no such server.
fn failed(message: impl std::fmt::Display) -> Failed {
    Failed {
        kind: FailureKind::Failed,
        message: message.to_string(),
    }
}

/// A paired server, as the picker shows it. No token: it is a credential, and
/// the webview has no use for one it can never present.
#[derive(Debug, Clone, Serialize)]
struct ServerSummary {
    name: String,
    address: String,
}

#[tauri::command(async)]
fn servers() -> Result<Vec<ServerSummary>, Failed> {
    let remotes = Remotes::load().map_err(failed)?;
    Ok(remotes
        .list()
        .iter()
        .map(|r| ServerSummary {
            name: r.name.clone(),
            address: r.address(),
        })
        .collect())
}

/// What pairing produced: the server just added, the list it is now in, and
/// the version of the `hurad` that answered.
///
/// The version is in here because it is the proof: a pairing string is a claim
/// about an address, and a version that came back over the pinned connection is
/// that claim answered. Hand-written like [`ServerSummary`], being the bridge's
/// own shape rather than a message on the wire.
#[derive(Debug, Clone, Serialize)]
struct Paired {
    server: ServerSummary,
    servers: Vec<ServerSummary>,
    version: String,
}

/// Pair with a server from the window, rather than from a terminal.
///
/// The whole of the checking is `hura_client::pair`, which is what `hura connect`
/// calls -- so a string this window accepts is one the CLI would accept, and a
/// server it refuses is refused for the same stated reason. This command exists
/// because the alternative on Windows is installing a CLI whose other half
/// cannot run there at all.
#[tauri::command(async)]
fn connect(pairing: String, name: Option<String>) -> Result<Paired, Failed> {
    let (remote, hello) = hura_client::pair(&pairing, name.as_deref()).map_err(failed)?;
    Ok(Paired {
        server: ServerSummary {
            name: remote.name.clone(),
            address: remote.address(),
        },
        servers: servers()?,
        version: hello.version,
    })
}

/// Forget one, which is `hura remotes --forget`.
///
/// It drops a token this machine holds and nothing on the server: the server
/// stops accepting one when `hurad revoke` says so, which is the half that
/// matters if the token has been somewhere it should not.
#[tauri::command(async)]
fn forget(name: String) -> Result<Vec<ServerSummary>, Failed> {
    let mut remotes = Remotes::load().map_err(failed)?;
    if !remotes.remove(&name) {
        return Err(failed(format!("no server named `{name}`")));
    }
    remotes.save().map_err(failed)?;
    servers()
}

/// What the about screen says: this window's version, and the server's.
///
/// The server half is asked fresh rather than remembered from pairing, because
/// `hurad` updates itself and the version it paired at is a fact about last
/// month. Unauthenticated -- it is `GET /version` -- so a revoked token still
/// gets an answer here, which is the moment somebody wants one.
#[derive(Debug, Clone, Serialize)]
struct About {
    desktop: String,
    /// Whether this build can replace itself. The plugin is compiled in on
    /// Windows only; see `Update.tsx`.
    updater: bool,
    server_version: Option<String>,
    server_error: Option<String>,
}

#[tauri::command(async)]
fn about(server: Option<String>) -> About {
    let hello = server.map(|name| remote(&name).and_then(|r| r.hello().map_err(to_message)));
    let (server_version, server_error) = match hello {
        Some(Ok(h)) => (Some(h.version), None),
        Some(Err(e)) => (None, Some(e.message)),
        None => (None, None),
    };
    About {
        desktop: env!("CARGO_PKG_VERSION").into(),
        updater: cfg!(windows),
        server_version,
        server_error,
    }
}

fn remote(name: &str) -> Result<Remote, Failed> {
    let remotes = Remotes::load().map_err(failed)?;
    remotes.select(Some(name)).cloned().map_err(failed)
}

/// The reply a request was supposed to produce, or a message saying it was not.
///
/// A server answering a `Diff` with a `Policy` is a bug rather than a state to
/// handle, but it has to fail as something the window can show rather than as a
/// panic that takes the process with it.
macro_rules! expect_reply {
    ($reply:expr, $pattern:pat => $value:expr, $what:literal) => {
        match $reply {
            $pattern => Ok($value),
            _ => Err(crate::failed(format!(
                "the server answered something other than {}",
                $what
            ))),
        }
    };
}

#[tauri::command(async)]
fn sessions(server: String) -> Result<Vec<Session>, Failed> {
    let reply = remote(&server)?.call(Request::Ls).map_err(to_message)?;
    expect_reply!(reply, Reply::Ls { sessions, .. } => sessions, "a session list")
}

/// End a session: its sandbox, and its record.
///
/// Answers with the refreshed list, so the window redraws from the server's
/// account rather than dropping the row it had. The confirmation belongs to the
/// window -- see `Tree.tsx` -- because this is not undoable and the agent
/// inside may be part-way through something.
#[tauri::command(async)]
fn destroy(server: String, name: String) -> Result<Vec<Session>, Failed> {
    let reply = remote(&server)?
        .call(Request::Destroy { name })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Ls { sessions, .. } => sessions, "a session list")
}

#[tauri::command(async)]
fn poll(server: String, name: String) -> Result<Poll, Failed> {
    let reply = remote(&server)?
        .call(Request::Poll { name })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Poll(poll) => poll, "a poll")
}

#[tauri::command(async)]
fn policy(server: String, name: String) -> Result<PolicyView, Failed> {
    let reply = remote(&server)?
        .call(Request::Policy { name })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Policy(view) => view, "a policy")
}

/// The feed, each event with the endpoint it was about.
///
/// The target is derived here, on this side of the bridge, rather than sent by
/// the server: it is a reading of the subject, and reading it with this build's
/// parser means an older `hurad` still gets the allow and block buttons.
#[tauri::command(async)]
fn events(server: String, name: String) -> Result<Vec<FeedEvent>, Failed> {
    let reply = remote(&server)?
        .call(Request::Events { name })
        .map_err(to_message)?;
    let events = expect_reply!(reply, Reply::Events { events } => events, "an event feed")?;
    Ok(events.into_iter().map(FeedEvent::from).collect())
}

/// Open an endpoint to a session, and to every new one when `everywhere`.
/// Answers with the policy re-read.
#[tauri::command(async)]
fn allow(
    server: String,
    name: String,
    endpoint: String,
    everywhere: bool,
) -> Result<PolicyView, Failed> {
    let reply = remote(&server)?
        .call(Request::Allow {
            name,
            endpoint,
            everywhere,
        })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Policy(view) => view, "a policy")
}

/// Open only some methods and paths of an endpoint to a session, and to every
/// new one when `everywhere`. Answers with the policy re-read.
#[tauri::command(async)]
fn allow_paths(
    server: String,
    name: String,
    endpoint: String,
    routes: Vec<Route>,
    everywhere: bool,
) -> Result<PolicyView, Failed> {
    let reply = remote(&server)?
        .call(Request::AllowPaths {
            name,
            endpoint,
            routes,
            everywhere,
        })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Policy(view) => view, "a policy")
}

/// Close an endpoint to a session, and to every new one when `everywhere`.
#[tauri::command(async)]
fn block(
    server: String,
    name: String,
    endpoint: String,
    everywhere: bool,
) -> Result<PolicyView, Failed> {
    let reply = remote(&server)?
        .call(Request::Block {
            name,
            endpoint,
            everywhere,
        })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Policy(view) => view, "a policy")
}

/// Take an endpoint off the global lists. Answers with this session's policy.
#[tauri::command(async)]
fn unlist(server: String, name: String, endpoint: String) -> Result<PolicyView, Failed> {
    let reply = remote(&server)?
        .call(Request::Unlist { name, endpoint })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Policy(view) => view, "a policy")
}

/// Remove one of a session's own rules. Answers with its policy re-read.
#[tauri::command(async)]
fn remove_rule(server: String, name: String, id: String) -> Result<PolicyView, Failed> {
    let reply = remote(&server)?
        .call(Request::RemoveRule { name, id })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Policy(view) => view, "a policy")
}

#[tauri::command(async)]
fn diff(server: String, name: String) -> Result<String, Failed> {
    let reply = remote(&server)?
        .call(Request::Diff { name })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Diff { body } => body, "a diff")
}

/// The working copy as git describes it, and the result of doing something to
/// it. Both answer the same way, so the window has one shape to handle.
#[derive(Serialize)]
struct GitAnswer {
    said: String,
    status: GitStatus,
}

#[tauri::command(async)]
fn git_status(server: String, name: String) -> Result<GitAnswer, Failed> {
    let reply = remote(&server)?
        .call(Request::GitStatus { name })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Git { said, status } => GitAnswer { said, status }, "a git status")
}

#[tauri::command(async)]
fn git(server: String, name: String, action: GitOp) -> Result<GitAnswer, Failed> {
    let reply = remote(&server)?
        .call(Request::Git { name, action })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Git { said, status } => GitAnswer { said, status }, "a git status")
}

#[tauri::command(async)]
fn git_diff(
    server: String,
    name: String,
    path: String,
    against: Against,
) -> Result<FileDiff, Failed> {
    let reply = remote(&server)?
        .call(Request::GitDiff {
            name,
            path,
            against,
        })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::GitDiff(diff) => diff, "a file diff")
}

/// One directory of a worktree's working copy.
#[tauri::command(async)]
fn files(server: String, name: String, path: String) -> Result<Dir, Failed> {
    let reply = remote(&server)?
        .call(Request::Files { name, path })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Files(dir) => dir, "a directory")
}

#[tauri::command(async)]
fn file(server: String, name: String, path: String) -> Result<FileText, Failed> {
    let reply = remote(&server)?
        .call(Request::File { name, path })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::File(text) => text, "a file")
}

/// The shells open beside a worktree's agent.
#[tauri::command(async)]
fn shells(server: String, name: String) -> Result<Vec<String>, Failed> {
    let reply = remote(&server)?
        .call(Request::Shells { name })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Shells { shells } => shells, "a shell list")
}

#[tauri::command(async)]
fn new_shell(server: String, name: String) -> Result<Vec<String>, Failed> {
    let reply = remote(&server)?
        .call(Request::NewShell { name })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Shells { shells } => shells, "a shell list")
}

#[tauri::command(async)]
fn kill_shell(server: String, name: String, tmux: String) -> Result<Vec<String>, Failed> {
    let reply = remote(&server)?
        .call(Request::KillShell { name, tmux })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Shells { shells } => shells, "a shell list")
}

/// A chat session's conversations, and the one just opened when that is what
/// was asked. The webview's shape for `Reply::Chats`.
#[derive(Serialize)]
struct Chats {
    chats: Vec<String>,
    opened: Option<String>,
}

fn chats_of(reply: Reply) -> Result<Chats, Failed> {
    expect_reply!(reply, Reply::Chats { chats, opened } => Chats { chats, opened }, "a conversation list")
}

#[tauri::command(async)]
fn chats(server: String, name: String) -> Result<Chats, Failed> {
    chats_of(remote(&server)?.call(Request::Chats { name }).map_err(to_message)?)
}

#[tauri::command(async)]
fn new_chat(server: String, name: String) -> Result<Chats, Failed> {
    chats_of(remote(&server)?.call(Request::NewChat { name }).map_err(to_message)?)
}

#[tauri::command(async)]
fn close_chat(server: String, name: String, conv: String) -> Result<Chats, Failed> {
    chats_of(
        remote(&server)?
            .call(Request::CloseChat { name, conv })
            .map_err(to_message)?,
    )
}

#[tauri::command(async)]
fn comments(server: String, name: String) -> Result<Vec<Comment>, Failed> {
    let reply = remote(&server)?
        .call(Request::Comments { name })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Comments { comments } => comments, "a review")
}

#[tauri::command(async)]
fn comment(server: String, name: String, comment: NewComment) -> Result<Vec<Comment>, Failed> {
    let reply = remote(&server)?
        .call(Request::Comment { name, comment })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Comments { comments } => comments, "a review")
}

#[tauri::command(async)]
fn uncomment(server: String, name: String, id: u64) -> Result<Vec<Comment>, Failed> {
    let reply = remote(&server)?
        .call(Request::Uncomment { name, id })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Comments { comments } => comments, "a review")
}

/// Send the review to the agent. Answers with the message it sent.
#[tauri::command(async)]
fn send_comments(server: String, name: String) -> Result<String, Failed> {
    let reply = remote(&server)?
        .call(Request::SendComments { name })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Told { message } => message, "a delivered review")
}

/// The projects on the server: what the tree is grouped under.
#[tauri::command(async)]
fn projects(server: String) -> Result<Vec<Project>, Failed> {
    let reply = remote(&server)?
        .call(Request::Projects)
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Projects { projects } => projects, "a project list")
}

#[tauri::command(async)]
fn new_project(server: String, project: NewProject) -> Result<Vec<Project>, Failed> {
    let reply = remote(&server)?
        .call(Request::NewProject(project))
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Projects { projects } => projects, "a project list")
}

#[tauri::command(async)]
fn forget_project(server: String, name: String) -> Result<Vec<Project>, Failed> {
    let reply = remote(&server)?
        .call(Request::ForgetProject { name })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Projects { projects } => projects, "a project list")
}

/// The repositories the *server* can see, for the picker.
///
/// The server's and not this machine's: a checkout is only a way of naming a
/// remote, but which checkouts exist is a fact about the machine that will do
/// the cloning, and `repo_roots` is configured there.
#[tauri::command(async)]
fn repos(server: String) -> Result<Listing, Failed> {
    let reply = remote(&server)?.call(Request::Repos).map_err(to_message)?;
    expect_reply!(reply, Reply::Repos(listing) => listing, "a repository list")
}

#[tauri::command(async)]
fn inspect(server: String, path: String, branch: Option<String>) -> Result<Picked, Failed> {
    let reply = remote(&server)?
        .call(Request::Inspect { path, branch })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Inspect(picked) => picked, "repository facts")
}

#[tauri::command(async)]
fn new_options(server: String) -> Result<NewOptions, Failed> {
    let reply = remote(&server)?
        .call(Request::NewOptions)
        .map_err(to_message)?;
    expect_reply!(reply, Reply::NewOptions(options) => options, "the create options")
}

/// Ask for a session. Answers as soon as the server has accepted the request,
/// which is before the session exists: it appears in the list a moment later,
/// in `creating`.
#[tauri::command(async)]
fn create(server: String, session: NewSession) -> Result<String, Failed> {
    let reply = remote(&server)?
        .call(Request::Create(Box::new(session)))
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Created { name } => name, "a created session")
}

/// What the server holds on a session's behalf: the MCP catalog and what each
/// managed container is doing, the secret names, and the uploaded skills.
#[tauri::command(async)]
fn integrations(server: String) -> Result<IntegrationsView, Failed> {
    let reply = remote(&server)?
        .call(Request::Integrations)
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Integrations(view) => view, "the integrations view")
}

#[tauri::command(async)]
fn mcp(server: String, name: String, action: McpOp) -> Result<IntegrationsView, Failed> {
    let reply = remote(&server)?
        .call(Request::Mcp { name, action })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Integrations(view) => view, "the integrations view")
}

/// Store a secret on the server, or forget it.
///
/// The value goes one way. There is no command here that reads one back and no
/// reply that carries one, so a token typed into this window is in this
/// process's memory for the length of one request and in the server's store
/// afterwards -- and never in the webview at all.
#[tauri::command(async)]
fn secret(server: String, name: String, value: Option<String>) -> Result<IntegrationsView, Failed> {
    let reply = remote(&server)?
        .call(Request::Secret { name, value })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Integrations(view) => view, "the integrations view")
}

/// The trackers this machine reads, as the window may see them: whether each
/// has a token, never the token.
///
/// **Kept on this machine, not the server's.** See `hura_client::trackers`:
/// the tokens are logins to somebody's tickets and a server has no use for
/// them, so a tracker is set up, stored and read entirely from here.
#[tauri::command(async)]
fn trackers() -> Result<Vec<Configured>, Failed> {
    Ok(load_trackers()?.views())
}

#[tauri::command(async)]
fn add_tracker(tracker: TrackerSource, token: Option<String>) -> Result<Vec<Configured>, Failed> {
    change_trackers(|t| t.add(&tracker, token.as_deref()))
}

/// Replace a tracker by its current name, which is how its filters are edited.
/// Its token stays.
#[tauri::command(async)]
fn update_tracker(name: String, tracker: TrackerSource) -> Result<Vec<Configured>, Failed> {
    change_trackers(|t| t.update(&name, &tracker))
}

#[tauri::command(async)]
fn set_tracker_token(name: String, token: String) -> Result<Vec<Configured>, Failed> {
    change_trackers(|t| t.set_token(&name, &token))
}

#[tauri::command(async)]
fn forget_tracker(name: String) -> Result<Vec<Configured>, Failed> {
    change_trackers(|t| t.forget(&name))
}

/// One change to the trackers file, under a lock: commands run on a pool, and
/// two load-modify-saves racing would lose whichever finished first.
fn change_trackers(
    f: impl FnOnce(&mut Trackers) -> Result<(), String>,
) -> Result<Vec<Configured>, Failed> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut t = load_trackers()?;
    f(&mut t).map_err(failed)?;
    t.save()
        .map_err(|e| failed(format!("could not save the trackers: {e}")))?;
    Ok(t.views())
}

fn load_trackers() -> Result<Trackers, Failed> {
    Trackers::load().map_err(|e| failed(format!("could not read the trackers: {e}")))
}

/// Push this machine's own skills to the server.
///
/// **The reading and the packing happen on this side of the bridge**, which is
/// the whole reason this is a Tauri command and not something the webview does:
/// `~/.claude/skills` is on *this* machine, a webview cannot read it, and the
/// packing is `hura_core::skills::payload` -- the same tar the seeder carries, so
/// there is one definition of what a packed skill is.
///
/// Every skill the agent would load, not a selection: a list to maintain here
/// would be a list that goes stale the first time you add a skill and forget.
#[tauri::command(async)]
fn upload_skills(server: String) -> Result<IntegrationsView, Failed> {
    let mine = hura_core::skills::local();
    if mine.is_empty() {
        return Err(failed(format!(
            "no skills in {} to upload",
            hura_core::skills::host_skills_dir().display()
        )));
    }
    let mut uploads = Vec::new();
    let mut problems = Vec::new();
    for skill in &mine {
        match hura_core::skills::payload(&skill.source) {
            Ok(tar) => uploads.push(hura_core::skills::Upload {
                name: skill.name.clone(),
                origin: skill.source.display().to_string(),
                tar,
            }),
            // One skill that has grown a virtualenv should not stop the rest.
            Err(e) => problems.push(format!("{}: {e}", skill.name)),
        }
    }
    if uploads.is_empty() {
        return Err(failed(format!(
            "none of the skills could be packed: {}",
            problems.join("; ")
        )));
    }
    let reply = remote(&server)?
        .call(Request::UploadSkills { skills: uploads })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Integrations(view) => view, "the integrations view")
}

#[tauri::command(async)]
fn forget_skill(server: String, name: String) -> Result<IntegrationsView, Failed> {
    let reply = remote(&server)?
        .call(Request::ForgetSkill { name })
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Integrations(view) => view, "the integrations view")
}

/// What this machine has to upload, for a screen that wants to say so before
/// anything is sent.
#[tauri::command(async)]
fn my_skills() -> Vec<String> {
    hura_core::skills::local()
        .into_iter()
        .map(|s| s.name)
        .collect()
}

/// Your tickets: what each tracker's filters match, read from this machine.
///
/// The server is asked one thing, and only for the branch names: its
/// `branch_prefix`, so a ticket suggests the branch a session started from it
/// will actually get. A server that cannot be reached costs that and nothing
/// else -- the tickets are still read, under the built-in prefix, and the
/// warning says so.
#[tauri::command(async)]
fn tickets(server: Option<String>) -> Result<Inbox, Failed> {
    let trackers = load_trackers()?;
    let (prefix, warning) = branch_prefix(server.as_deref());
    let mut inbox = hura_core::tracker::inbox(trackers.list(), &prefix);
    inbox.warnings.extend(warning);
    Ok(inbox)
}

/// The server's `branch_prefix`, or the built-in one and why.
fn branch_prefix(server: Option<&str>) -> (String, Option<String>) {
    match server.map(|s| remote(s).and_then(|r| r.call(Request::Settings).map_err(to_message))) {
        Some(Ok(Reply::Settings(view))) => (
            view.settings
                .branch_prefix
                .unwrap_or(view.default_branch_prefix),
            None,
        ),
        Some(Err(e)) => (
            hura_core::session::DEFAULT_BRANCH_PREFIX.to_string(),
            Some(format!(
                "branch names use the default prefix: the server did not answer ({})",
                e.message
            )),
        ),
        _ => (hura_core::session::DEFAULT_BRANCH_PREFIX.to_string(), None),
    }
}

/// The tracker a ticket came from, by the name the screen knows it by.
fn tracker_named(
    trackers: &hura_client::trackers::Trackers,
    name: &str,
) -> Result<hura_core::tracker::Stored, Failed> {
    trackers
        .list()
        .iter()
        .find(|t| t.source.name == name)
        .cloned()
        .ok_or_else(|| failed(format!("no tracker called `{name}`")))
}

// Changing a ticket. Each is one button somebody pressed: nothing here runs
// on a timer, and each answers with nothing but whether it worked -- the
// window reads the ticket again to see what it now says.

/// A ticket's edit screen: the fields Jira lets this account change, as the
/// control for each, with their values.
#[tauri::command(async)]
fn edit_form(tracker: String, key: String) -> Result<hura_core::tracker::EditForm, Failed> {
    let stored = tracker_named(&load_trackers()?, &tracker)?;
    hura_core::tracker::edit_form(&stored, &key).map_err(failed)
}

#[tauri::command(async)]
fn save_ticket(
    tracker: String,
    key: String,
    changes: Vec<hura_core::tracker::FieldChange>,
) -> Result<(), Failed> {
    let stored = tracker_named(&load_trackers()?, &tracker)?;
    hura_core::tracker::save(&stored, &key, &changes).map_err(failed)
}

/// Where the ticket can move from its current status.
#[tauri::command(async)]
fn transitions(
    tracker: String,
    key: String,
) -> Result<Vec<hura_core::tracker::Transition>, Failed> {
    let stored = tracker_named(&load_trackers()?, &tracker)?;
    hura_core::tracker::transitions(&stored, &key).map_err(failed)
}

#[tauri::command(async)]
fn transition(tracker: String, key: String, id: String) -> Result<(), Failed> {
    let stored = tracker_named(&load_trackers()?, &tracker)?;
    hura_core::tracker::transition(&stored, &key, &id).map_err(failed)
}

#[tauri::command(async)]
fn add_comment(tracker: String, key: String, markdown: String) -> Result<(), Failed> {
    let stored = tracker_named(&load_trackers()?, &tracker)?;
    hura_core::tracker::comment(&stored, &key, &markdown).map_err(failed)
}

#[tauri::command(async)]
fn edit_comment(tracker: String, key: String, id: String, markdown: String) -> Result<(), Failed> {
    let stored = tracker_named(&load_trackers()?, &tracker)?;
    hura_core::tracker::edit_comment(&stored, &key, &id, &markdown).map_err(failed)
}

#[tauri::command(async)]
fn delete_comment(tracker: String, key: String, id: String) -> Result<(), Failed> {
    let stored = tracker_named(&load_trackers()?, &tracker)?;
    hura_core::tracker::delete_comment(&stored, &key, &id).map_err(failed)
}

/// People to put in a person field, by the start of their name.
#[tauri::command(async)]
fn users(
    tracker: String,
    key: String,
    query: String,
    assignable: bool,
) -> Result<Vec<hura_core::tracker::UserChoice>, Failed> {
    let stored = tracker_named(&load_trackers()?, &tracker)?;
    hura_core::tracker::users(&stored, &key, &query, assignable).map_err(failed)
}

/// Every board the Jira trackers' accounts can see: the board view's picker.
#[tauri::command(async)]
fn boards() -> Result<hura_core::tracker::Boards, Failed> {
    Ok(hura_core::tracker::boards(load_trackers()?.list()))
}

/// One board, its columns and the cards in each, read from this machine like
/// the filters -- and like them, the server is asked only for the branch
/// prefix its cards suggest.
#[tauri::command(async)]
fn board(
    server: Option<String>,
    tracker: String,
    id: String,
) -> Result<hura_core::tracker::BoardView, Failed> {
    let trackers = load_trackers()?;
    let stored = trackers
        .list()
        .iter()
        .find(|t| t.source.name == tracker)
        .ok_or_else(|| failed(format!("no tracker called `{tracker}`")))?;
    // A board with a wrong prefix on its branches is still a board; the
    // filters say why when the server is not answering, and saying it twice
    // is noise.
    let (prefix, _) = branch_prefix(server.as_deref());
    hura_core::tracker::board(stored, &id, &prefix).map_err(failed)
}

/// One ticket in full -- description, comments, the people on it -- read
/// from this machine with the tracker's stored token, like the board.
#[tauri::command(async)]
fn ticket(tracker: String, key: String) -> Result<hura_core::tracker::Issue, Failed> {
    let trackers = load_trackers()?;
    let stored = trackers
        .list()
        .iter()
        .find(|t| t.source.name == tracker)
        .ok_or_else(|| failed(format!("no tracker called `{tracker}`")))?;
    hura_core::tracker::issue(stored, &key).map_err(failed)
}

/// The editable defaults in the server's config file.
///
/// The server's own file, not a preference of this window's, and the split is
/// deliberate: `branch_prefix` names the work branch of every session on that
/// machine including the ones started from a terminal, so a window holding its
/// own copy would be a second convention that disagrees with the first. What
/// *is* this window's -- how wide the sidebars are, how often the list is
/// re-read -- never leaves it. See `prefs.ts`.
#[tauri::command(async)]
fn settings(server: String) -> Result<SettingsView, Failed> {
    let reply = remote(&server)?
        .call(Request::Settings)
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Settings(view) => view, "the settings view")
}

/// Write them back, and answer with the file as it now reads.
///
/// Re-read rather than echoed, for the reason the integrations screen gives:
/// a cleared field comes back out as an absent key and an absent key reads as
/// the built-in default, so what was saved is not what was sent.
#[tauri::command(async)]
fn set_settings(server: String, settings: Settings) -> Result<SettingsView, Failed> {
    let reply = remote(&server)?
        .call(Request::SetSettings(settings))
        .map_err(to_message)?;
    expect_reply!(reply, Reply::Settings(view) => view, "the settings view")
}

/// The one streaming connection, and which server it is to.
///
/// One per window rather than one per pane: the protocol multiplexes, so four
/// terminals and four feeds share a socket, a token check and a reconnect. See
/// [`hura_proto::stream`].
#[derive(Default)]
struct Streaming {
    open: Mutex<Option<Open>>,
}

struct Open {
    server: String,
    sink: Sink,
}

/// Every frame the server sends, as a window event.
///
/// One event name for all of them rather than one per channel: the frame
/// already carries its channel id, and a listener per channel would mean the
/// frontend unsubscribing correctly every time a pane closes -- which it would
/// eventually not.
const FRAME: &str = "hura://frame";

/// Connect if the window is not already connected to this server.
///
/// Switching servers replaces the connection, which also ends every channel on
/// it -- correct, since the channels named sessions on the old one.
fn connected(app: &tauri::AppHandle, server: &str) -> Result<(), Failed> {
    let state = app.state::<Streaming>();
    let mut open = state.open.lock().map_err(failed)?;

    if open.as_ref().is_some_and(|o| o.server == server) {
        return Ok(());
    }

    let (sink, frames) = remote(server)?.stream().map_err(to_message)?.split();
    // Previews of another server's sessions would go down this connection to
    // the wrong machine.
    app.state::<previews::Previews>().keep_only(server);

    let handle = app.clone();
    std::thread::spawn(move || {
        for message in frames {
            match message {
                // A preview's connections are this side's to carry, not the
                // window's: they are bytes for a socket, not something to draw.
                Incoming::Frame(frame) if handle.state::<previews::Previews>().route(&frame) => {}
                Incoming::Frame(frame) => {
                    let _ = handle.emit(FRAME, *frame);
                }
                // The connection has gone. Every open channel is closed by it,
                // so each is told rather than left waiting for output that will
                // not come.
                Incoming::Ended(reason) => {
                    handle.state::<previews::Previews>().connection_ended();
                    let _ = handle.emit(
                        FRAME,
                        ServerFrame::Closed {
                            id: ALL_CHANNELS,
                            reason: reason.or_else(|| Some("the connection ended".into())),
                        },
                    );
                    break;
                }
            }
        }
    });

    *open = Some(Open {
        server: server.to_string(),
        sink,
    });
    Ok(())
}

/// The id a `Closed` carries when it is about the whole connection rather than
/// one channel. Not a real channel id: no client allocates it.
const ALL_CHANNELS: ChannelId = ChannelId::MAX;

fn send(app: &tauri::AppHandle, frame: ClientFrame) -> Result<(), Failed> {
    let state = app.state::<Streaming>();
    let open = state.open.lock().map_err(failed)?;
    let Some(open) = open.as_ref() else {
        return Err(failed("not connected"));
    };
    open.sink
        .send(frame)
        .then_some(())
        .ok_or_else(|| failed("the connection has ended"))
}

#[tauri::command(async)]
fn watch(
    app: tauri::AppHandle,
    server: String,
    id: ChannelId,
    channel: Channel,
) -> Result<(), Failed> {
    connected(&app, &server)?;
    send(&app, ClientFrame::Open { id, channel })
}

#[tauri::command(async)]
fn unwatch(app: tauri::AppHandle, id: ChannelId) -> Result<(), Failed> {
    send(&app, ClientFrame::Close { id })
}

#[tauri::command(async)]
fn terminal_input(app: tauri::AppHandle, id: ChannelId, data: String) -> Result<(), Failed> {
    send(&app, ClientFrame::Input { id, data })
}

/// Something done in a conversation, on its chat channel.
#[tauri::command(async)]
fn chat_command(app: tauri::AppHandle, id: ChannelId, command: ChatCommand) -> Result<(), Failed> {
    send(&app, ClientFrame::Chat { id, command })
}

#[tauri::command(async)]
fn terminal_resize(
    app: tauri::AppHandle,
    id: ChannelId,
    cols: u16,
    rows: u16,
) -> Result<(), Failed> {
    send(&app, ClientFrame::Resize { id, cols, rows })
}

fn main() {
    // Wayland is left alone: WSLg is a Wayland compositor and the window is a
    // Wayland client there. `GDK_BACKEND=x11` is a way to make X11 screenshot
    // tooling see the surface, not a way to run.
    let builder = tauri::Builder::default()
        .manage(Streaming::default())
        .manage(previews::Previews::default())
        // An OS notification when a session starts waiting on a permission
        // prompt is the single largest quality-of-life gain this window has
        // over the terminal: watching four agents is exactly the case where a
        // terminal loses, and a `waiting` badge in a list you are not looking
        // at is a badge nobody sees. The window decides *when* -- see
        // `App.tsx` -- because it is the thing that knows which states it has
        // already seen.
        .plugin(tauri_plugin_notification::init())
        // Links. A webview's `target="_blank"` goes nowhere in a Tauri window:
        // there is no browser behind it to hand a new tab to. This is what
        // gives one to the system's -- see `src/open.ts`, and the capability,
        // which lets it open web and mail addresses and nothing else.
        .plugin(tauri_plugin_opener::init());

    // Replacing itself, on the one platform that ships an installer to replace.
    // Checking and asking are the window's job -- see `src/Update.tsx`; these
    // two only make it possible. `capabilities/updater.json` is scoped to the
    // same platform, because a permission naming a plugin that was not compiled
    // in fails the build rather than being ignored.
    //
    // Rebound rather than chained, because the two arms of a `cfg` inside a
    // builder chain have different types and inference has nothing to go on.
    #[cfg(windows)]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init());

    builder
        .setup(|app| {
            // Debug builds open the inspector. There is no other way to see a
            // console message from inside this window.
            #[cfg(debug_assertions)]
            {
                use tauri::Manager as _;
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            servers,
            about,
            previews::previews,
            previews::preview_open,
            previews::preview_stop,
            previews::kill_port,
            previews::kill_process,
            previews::ports,
            connect,
            forget,
            sessions,
            destroy,
            poll,
            policy,
            events,
            allow,
            allow_paths,
            block,
            unlist,
            remove_rule,
            diff,
            git_status,
            git,
            git_diff,
            files,
            file,
            shells,
            new_shell,
            kill_shell,
            chats,
            new_chat,
            close_chat,
            chat_command,
            comments,
            comment,
            uncomment,
            send_comments,
            projects,
            new_project,
            forget_project,
            repos,
            inspect,
            new_options,
            create,
            integrations,
            mcp,
            secret,
            trackers,
            add_tracker,
            update_tracker,
            set_tracker_token,
            forget_tracker,
            upload_skills,
            forget_skill,
            my_skills,
            tickets,
            ticket,
            boards,
            board,
            edit_form,
            save_ticket,
            transitions,
            transition,
            add_comment,
            edit_comment,
            delete_comment,
            users,
            settings,
            set_settings,
            watch,
            unwatch,
            terminal_input,
            terminal_resize
        ])
        .run(tauri::generate_context!())
        .expect("the window could not be created");
}
