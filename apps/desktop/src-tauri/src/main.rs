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
//! **Tauri runs a synchronous command on the main thread**, the one pumping
//! the window's messages, and every command in this file is blocking I/O: a
//! TLS round trip to `hurad`, which may itself shell out to the sandbox
//! runtime's CLI. The session list is re-read every three seconds and each
//! read is an `sbx ls` on the other end, so the window spent a large part of
//! every second not pumping anything.
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
use hura_core::cloud::{Account as CloudAccount, Device as CloudDevice, Status as CloudStatus};
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
use hura_core::tracker::changes::{Change as InboxChange, Seen};
use hura_core::tracker::{Configured, Inbox, Source as TrackerSource};
use hura_proto::stream::{Channel, ChannelId, ClientFrame, ServerFrame};
use hura_proto::{FailureKind, GitOp, McpOp, Reply, Request};
use serde::Serialize;
use serde_json::json;
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

/// Have the runtime read each request to an endpoint, so a refused one is
/// said by its path. Lets nothing through.
#[tauri::command(async)]
fn watch_requests(server: String, name: String, endpoint: String) -> Result<PolicyView, Failed> {
    let reply = remote(&server)?
        .call(Request::Watch { name, endpoint })
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
    // A session from a pull request is a request of its own, so a server that
    // predates it refuses it rather than making a new branch with the pull
    // request's name. See `Request::CreateFromPull`.
    let from_pull = session.pull.is_some();
    let request = if from_pull {
        Request::CreateFromPull(Box::new(session))
    } else {
        Request::Create(Box::new(session))
    };
    let reply = remote(&server)?.call(request).map_err(|e| {
        let said = to_message(e);
        if from_pull && said.message.contains("`create-from-pull`") {
            failed(
                "this server is from before sessions could start from a pull request; \
                 `hurad update` on it brings it up to date",
            )
        } else {
            said
        }
    })?;
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
/// **Kept on this machine, not the hura server's.** See
/// `hura_client::trackers`: the tokens are logins to somebody's tickets and a
/// server that hosts agents has no use for them, so a tracker is set up,
/// stored and read from here. **Signed in to hura cloud**, the account's
/// trackers instead, kept and read there, the same on every device.
#[tauri::command(async)]
fn trackers() -> Result<Vec<Configured>, Failed> {
    if cloud().signed_in() {
        return cloud().trackers().map_err(cloud_failed);
    }
    Ok(load_trackers()?.views())
}

#[tauri::command(async)]
fn add_tracker(tracker: TrackerSource, token: Option<String>) -> Result<Vec<Configured>, Failed> {
    if cloud().signed_in() {
        let source = checked_source(&tracker)?;
        let token = token.as_deref().map(str::trim).filter(|t| !t.is_empty());
        return cloud().add_tracker(&source, token).map_err(cloud_failed);
    }
    change_trackers(|t| t.add(&tracker, token.as_deref()))
}

#[tauri::command(async)]
fn set_tracker_token(name: String, token: String) -> Result<Vec<Configured>, Failed> {
    if cloud().signed_in() {
        return cloud().set_token(&name, token.trim()).map_err(cloud_failed);
    }
    change_trackers(|t| t.set_token(&name, &token))
}

/// Change a tracker's filters, or whether it hides what you created. Its
/// token stays where it is and is not sent back for this.
#[tauri::command(async)]
fn edit_tracker(name: String, tracker: TrackerSource) -> Result<Vec<Configured>, Failed> {
    if cloud().signed_in() {
        let source = checked_source(&tracker)?;
        return cloud().edit_tracker(&name, &source).map_err(cloud_failed);
    }
    change_trackers(|t| t.edit(&name, &tracker))
}

#[tauri::command(async)]
fn forget_tracker(name: String) -> Result<Vec<Configured>, Failed> {
    if cloud().signed_in() {
        return cloud().forget_tracker(&name).map_err(cloud_failed);
    }
    change_trackers(|t| t.forget(&name))
}

/// Trimmed, named and able to work, or why not: the check
/// `hura_client::trackers` makes, made here before the service is asked so
/// the answer reads the same.
fn checked_source(source: &TrackerSource) -> Result<TrackerSource, Failed> {
    let source = source.normalized();
    match source.problem() {
        Some(problem) => Err(failed(problem)),
        None => Ok(source),
    }
}

// hura cloud: signing this machine in, the account and its devices, and
// moving this machine's trackers to it. Optional: signed out, nothing above
// asks the service anything.

fn cloud() -> &'static hura_client::cloud::Cloud {
    static CLOUD: std::sync::OnceLock<hura_client::cloud::Cloud> = std::sync::OnceLock::new();
    CLOUD.get_or_init(hura_client::cloud::Cloud::new)
}

fn cloud_failed(e: hura_client::cloud::Failure) -> Failed {
    failed(e.message)
}

/// Who this machine is signed in as, or nothing: read from the session file,
/// without asking the service.
#[tauri::command(async)]
fn cloud_status() -> Option<CloudStatus> {
    cloud().session().map(|s| CloudStatus { email: s.email })
}

#[tauri::command(async)]
fn cloud_send_code(email: String) -> Result<(), Failed> {
    cloud().send_code(email.trim()).map_err(cloud_failed)
}

/// Sign this machine in with the emailed code, under its computer name.
#[tauri::command(async)]
fn cloud_verify(email: String, code: String) -> Result<CloudStatus, Failed> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let session = cloud()
        .verify(email.trim(), &code, &computer_name(), std::env::consts::OS)
        .map_err(cloud_failed)?;
    Ok(CloudStatus {
        email: session.email,
    })
}

#[tauri::command(async)]
fn cloud_sign_out() {
    cloud().sign_out();
}

#[tauri::command(async)]
fn cloud_account() -> Result<CloudAccount, Failed> {
    cloud().account().map_err(cloud_failed)
}

#[tauri::command(async)]
fn cloud_devices() -> Result<Vec<CloudDevice>, Failed> {
    cloud().devices().map_err(cloud_failed)
}

#[tauri::command(async)]
fn cloud_sign_out_device(id: String) -> Result<Vec<CloudDevice>, Failed> {
    cloud().sign_out_device(&id).map_err(cloud_failed)?;
    cloud().devices().map_err(cloud_failed)
}

/// The trackers hura cloud can connect through their own sign-in.
#[tauri::command(async)]
fn cloud_connectable() -> Vec<String> {
    if !cloud().signed_in() {
        return Vec::new();
    }
    cloud().connectable().unwrap_or_default()
}

/// Where to send the browser to connect `provider` to the account.
#[tauri::command(async)]
fn cloud_connect(provider: String) -> Result<String, Failed> {
    cloud().connect_url(&provider).map_err(cloud_failed)
}

/// While signed in, the trackers still kept on this machine: not read, and
/// offered to be moved to the account.
#[tauri::command(async)]
fn local_trackers() -> Result<Vec<Configured>, Failed> {
    if !cloud().signed_in() {
        return Ok(Vec::new());
    }
    Ok(load_trackers()?.views())
}

/// Move this machine's trackers, tokens and all, to the account, and forget
/// them here. One whose name the account already has stays, and the answer
/// is what stayed.
#[tauri::command(async)]
fn move_trackers_to_cloud() -> Result<Vec<Configured>, Failed> {
    let mut left = Vec::new();
    for t in load_trackers()?.list() {
        match cloud().add_tracker(&t.source, t.token.as_deref()) {
            Ok(_) => {}
            Err(e) if e.status == 409 => left.push(t.source.name.clone()),
            Err(e) => return Err(cloud_failed(e)),
        }
    }
    change_trackers(|t| {
        for name in t
            .list()
            .iter()
            .map(|s| s.source.name.clone())
            .collect::<Vec<_>>()
        {
            if !left.contains(&name) {
                t.forget(&name)?;
            }
        }
        Ok(())
    })
}

/// Whether hura cloud's stream of tracker changes is open, so the window
/// leaves announcing them to it.
static LIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What the window is sent for each batch of changes hura cloud finds.
const TRACKER_CHANGES: &str = "tracker-changes";

/// Whether hura cloud is telling this computer what changed on the trackers,
/// over its open stream. While it is, the window does not announce what it
/// finds itself, which would say everything twice.
#[tauri::command]
fn cloud_live() -> bool {
    LIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Keeps hura cloud's stream of the account's tracker changes open while
/// this computer is signed in, for as long as the app runs, and hands each
/// batch to the window. A stream that drops is opened again, sooner while
/// they last and later while they fail.
fn listen(app: tauri::AppHandle) {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        let mut wait = std::time::Duration::from_secs(5);
        loop {
            if !cloud().signed_in() {
                LIVE.store(false, Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_secs(5));
                continue;
            }
            let started = std::time::Instant::now();
            let ended = cloud().events(
                &mut || LIVE.store(true, Ordering::Relaxed),
                &mut |changes| {
                    let _ = app.emit(TRACKER_CHANGES, changes);
                },
            );
            LIVE.store(false, Ordering::Relaxed);
            // A stream that stayed up a while was a good one: start over.
            if started.elapsed() > std::time::Duration::from_secs(120) {
                wait = std::time::Duration::from_secs(5);
            }
            if let Err(e) = ended {
                eprintln!("hura cloud's stream: {e}");
            }
            std::thread::sleep(wait);
            wait = (wait * 2).min(std::time::Duration::from_secs(120));
        }
    });
}

/// What a device is called in the account's list: the name this computer
/// goes by.
fn computer_name() -> String {
    let ask = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    if cfg!(target_os = "macos")
        && let Some(name) = ask("scutil", &["--get", "ComputerName"])
    {
        return name;
    }
    ask("hostname", &[]).unwrap_or_else(|| "computer".into())
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

/// What is yours on every tracker: the tickets assigned to you or created by
/// you, and the pull requests you opened or that wait on you, read from this
/// machine.
///
/// The server is asked one thing, and only for the branch names: its
/// `branch_prefix`, so a ticket suggests the branch a session started from it
/// will actually get. A server that cannot be reached costs that and nothing
/// else -- the tickets are still read, under the built-in prefix, and the
/// warning says so.
#[tauri::command(async)]
fn tickets(server: Option<String>) -> Result<Inbox, Failed> {
    let (prefix, warning) = branch_prefix(server.as_deref());
    let mut inbox = if cloud().signed_in() {
        let answer = cloud()
            .tracker(&serde_json::json!({ "op": "inbox", "prefix": prefix }))
            .map_err(cloud_failed)?;
        serde_json::from_value(answer).map_err(failed)?
    } else {
        hura_core::tracker::inbox(load_trackers()?.list(), &prefix)
    };
    inbox.warnings.extend(warning);
    Ok(inbox)
}

/// A read recorded against what was seen at the last one.
#[derive(Serialize)]
struct Recorded {
    seen: Seen,
    changes: Vec<InboxChange>,
}

/// What changed since the last read of the trackers, and what is seen now.
///
/// Keeps nothing: the window holds what was seen and hands it back, so this
/// is the same call the phone makes. Nothing seen, or something that will not
/// parse, makes this read the baseline, which costs one quiet poll.
#[tauri::command(async)]
fn inbox_changes(inbox: Inbox, seen: Option<serde_json::Value>) -> Recorded {
    let seen: Option<Seen> = seen.and_then(|v| serde_json::from_value(v).ok());
    let (seen, changes) = hura_core::tracker::changes::record(seen.as_ref(), &inbox);
    Recorded { seen, changes }
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

/// One of hura-core's tracker requests (`hura_core::tracker::request`) on the
/// tracker the screen names: on hura cloud when signed in, which reads the
/// account's tracker by that name; otherwise here, with this machine's tracker
/// and its token. The same request and the same answer either way.
fn on_tracker<T: serde::de::DeserializeOwned>(
    op: &str,
    tracker: &str,
    mut args: serde_json::Value,
) -> Result<T, Failed> {
    args["op"] = json!(op);
    let answer = if cloud().signed_in() {
        args["tracker"] = json!(tracker);
        cloud().tracker(&args).map_err(cloud_failed)?
    } else {
        let stored = load_trackers()?
            .list()
            .iter()
            .find(|t| t.source.name == tracker)
            .cloned()
            .ok_or_else(|| failed(format!("no tracker called `{tracker}`")))?;
        args["tracker"] = serde_json::to_value(stored).map_err(failed)?;
        hura_core::tracker::request::handle(&args).map_err(failed)?
    };
    serde_json::from_value(answer).map_err(failed)
}

// Changing a ticket. Each is one button somebody pressed: nothing here runs
// on a timer, and each answers with nothing but whether it worked -- the
// window reads the ticket again to see what it now says.

/// A ticket's edit screen: the fields Jira lets this account change, as the
/// control for each, with their values.
#[tauri::command(async)]
fn edit_form(tracker: String, key: String) -> Result<hura_core::tracker::EditForm, Failed> {
    on_tracker("edit_form", &tracker, json!({ "key": key }))
}

#[tauri::command(async)]
fn save_ticket(
    tracker: String,
    key: String,
    changes: Vec<hura_core::tracker::FieldChange>,
) -> Result<(), Failed> {
    on_tracker("save", &tracker, json!({ "key": key, "changes": changes }))
}

/// Where the ticket can move from its current status.
#[tauri::command(async)]
fn transitions(
    tracker: String,
    key: String,
) -> Result<Vec<hura_core::tracker::Transition>, Failed> {
    on_tracker("transitions", &tracker, json!({ "key": key }))
}

#[tauri::command(async)]
fn transition(tracker: String, key: String, id: String) -> Result<(), Failed> {
    on_tracker("transition", &tracker, json!({ "key": key, "id": id }))
}

#[tauri::command(async)]
fn add_comment(tracker: String, key: String, markdown: String) -> Result<(), Failed> {
    on_tracker(
        "comment",
        &tracker,
        json!({ "key": key, "markdown": markdown }),
    )
}

#[tauri::command(async)]
fn edit_comment(tracker: String, key: String, id: String, markdown: String) -> Result<(), Failed> {
    on_tracker(
        "edit_comment",
        &tracker,
        json!({ "key": key, "id": id, "markdown": markdown }),
    )
}

#[tauri::command(async)]
fn delete_comment(tracker: String, key: String, id: String) -> Result<(), Failed> {
    on_tracker("delete_comment", &tracker, json!({ "key": key, "id": id }))
}

/// People to put in a person field, by the start of their name.
#[tauri::command(async)]
fn users(
    tracker: String,
    key: String,
    query: String,
    assignable: bool,
) -> Result<Vec<hura_core::tracker::UserChoice>, Failed> {
    on_tracker(
        "users",
        &tracker,
        json!({ "key": key, "query": query, "assignable": assignable }),
    )
}

// A pull request, read in full and reviewed from here. Read and written from
// this machine with the tracker's own token, like the tickets: the server is
// not involved.

/// One pull request: its checks, its reviewers and its conversation.
#[tauri::command(async)]
fn pull(tracker: String, repo: String, id: String) -> Result<hura_core::tracker::PullDetail, Failed> {
    on_tracker("pull", &tracker, json!({ "repo": repo, "id": id }))
}

/// The files it changes, and the two commits it is measured between.
#[tauri::command(async)]
fn pull_files(
    tracker: String,
    repo: String,
    id: String,
) -> Result<hura_core::tracker::PullFiles, Failed> {
    on_tracker("pull_files", &tracker, json!({ "repo": repo, "id": id }))
}

/// Both sides of one of its files, for the diff editor. `repo`, `base` and
/// `head` are what `pull_files` answered with.
#[tauri::command(async)]
fn pull_file(
    tracker: String,
    repo: String,
    base: String,
    head: String,
    file: hura_core::tracker::PullFile,
) -> Result<hura_core::git::FileDiff, Failed> {
    on_tracker(
        "pull_file",
        &tracker,
        json!({ "repo": repo, "base": base, "head": head, "file": file }),
    )
}

/// A comment on it, or a reply to one of its threads.
#[tauri::command(async)]
fn pull_comment(
    tracker: String,
    repo: String,
    id: String,
    body: String,
    thread: Option<String>,
) -> Result<(), Failed> {
    on_tracker(
        "pull_comment",
        &tracker,
        json!({ "repo": repo, "id": id, "body": body, "thread": thread }),
    )
}

/// A review: approve, ask for changes, or one of Azure DevOps' votes.
#[tauri::command(async)]
fn pull_vote(
    tracker: String,
    repo: String,
    id: String,
    verdict: hura_core::tracker::PullVerdict,
    body: String,
) -> Result<(), Failed> {
    on_tracker(
        "pull_vote",
        &tracker,
        json!({ "repo": repo, "id": id, "verdict": verdict, "body": body }),
    )
}

/// Merge it now (Azure DevOps: complete it). `head` is the commit the window
/// showed, so that something pushed since is refused rather than merged.
#[tauri::command(async)]
fn pull_merge(
    tracker: String,
    repo: String,
    id: String,
    how: hura_core::tracker::PullMerge,
    head: Option<String>,
    delete_branch: bool,
) -> Result<(), Failed> {
    on_tracker(
        "pull_merge",
        &tracker,
        json!({ "repo": repo, "id": id, "how": how, "head": head, "delete_branch": delete_branch }),
    )
}

/// Set it to merge itself once it can, or with `how` null, take that back:
/// GitHub's auto-merge, Azure DevOps' auto-complete.
#[tauri::command(async)]
fn pull_auto_merge(
    tracker: String,
    repo: String,
    id: String,
    how: Option<hura_core::tracker::PullMerge>,
    delete_branch: bool,
) -> Result<(), Failed> {
    on_tracker(
        "pull_auto_merge",
        &tracker,
        json!({ "repo": repo, "id": id, "how": how, "delete_branch": delete_branch }),
    )
}

/// One ticket in full (description, comments, the people on it), read from
/// this machine with the tracker's stored token, like the sidebar.
#[tauri::command(async)]
fn ticket(tracker: String, key: String) -> Result<hura_core::tracker::Issue, Failed> {
    on_tracker("issue", &tracker, json!({ "key": key }))
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
        .plugin(tauri_plugin_opener::init())
        // The terminal's copy and paste. A webview's own clipboard API wants a
        // user gesture, and a copy made in tmux arrives as OSC 52 a round trip
        // after the mouse let go -- too late to count as one. Reading is for
        // Ctrl+Shift+V, which no webview treats as a paste.
        .plugin(tauri_plugin_clipboard_manager::init());

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
            listen(app.handle().clone());
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
            watch_requests,
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
            set_tracker_token,
            edit_tracker,
            forget_tracker,
            upload_skills,
            forget_skill,
            my_skills,
            tickets,
            inbox_changes,
            cloud_status,
            cloud_send_code,
            cloud_verify,
            cloud_sign_out,
            cloud_account,
            cloud_devices,
            cloud_sign_out_device,
            local_trackers,
            move_trackers_to_cloud,
            cloud_live,
            cloud_connectable,
            cloud_connect,
            ticket,
            pull,
            pull_files,
            pull_file,
            pull_comment,
            pull_vote,
            pull_merge,
            pull_auto_merge,
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
