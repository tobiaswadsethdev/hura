//! One [`Request`] in, one [`Outcome`] out.
//!
//! The whole of what the server *does*, and deliberately thin: every arm is a
//! call into [`hura_core::ops`], which is the same function the CLI calls. A
//! behaviour that exists here and not there would be one the terminal cannot
//! do, which is how two front ends start disagreeing about what a session is.
//!
//! Nothing here is async. The core talks to the sandbox runtime by running a
//! subprocess, so every one of these blocks for a few hundred milliseconds;
//! [`crate::serve`] is what keeps that off the runtime's threads.

use std::path::Path;

use hura_core::backend::{Backend, Backends};
use hura_core::session::Session;
use hura_core::store::Store;
use hura_core::{chat, comments, config, files, git, image, ops, projects, repos, secrets, skills};
use hura_proto::{Failure, GitOp, McpOp, Outcome, Reply, Request};

/// Answer one request.
///
/// Every arm that is about one session asks [`Backends::for_session`] for its
/// backend and hands it to the matching op.
pub fn dispatch(backends: &Backends, request: Request) -> Outcome {
    match request {
        Request::Ls => ls(backends),
        Request::Poll { name } => {
            with_session(&name, |s| Ok(ops::poll(backends.for_session(s), s).into()))
        }
        Request::Diff { name } => with_session(&name, |s| {
            Ok(Reply::Diff {
                body: ops::repo_diff(backends.for_session(s), s),
            })
        }),
        Request::Policy { name } => with_session(&name, |s| policy(backends.for_session(s), s)),
        Request::Events { name } => with_session(&name, |s| events(backends.for_session(s), s)),
        Request::Allow {
            name,
            endpoint,
            everywhere,
        } => with_session(&name, |s| {
            ops::allow(backends.for_session(s), s, &endpoint, &[], everywhere)
                .map(Reply::Policy)
                .map_err(Failure::failed)
        }),
        Request::AllowPaths {
            name,
            endpoint,
            routes,
            everywhere,
        } => with_session(&name, |s| {
            // Never an allow of the whole host: that is `Allow`, and an empty
            // list here is a client that lost the paths on the way.
            if routes.is_empty() {
                return Err(Failure::failed(format!(
                    "no paths named to allow on {endpoint}"
                )));
            }
            ops::allow(backends.for_session(s), s, &endpoint, &routes, everywhere)
                .map(Reply::Policy)
                .map_err(Failure::failed)
        }),
        Request::Block {
            name,
            endpoint,
            everywhere,
        } => with_session(&name, |s| {
            ops::block(backends.for_session(s), s, &endpoint, everywhere)
                .map(Reply::Policy)
                .map_err(Failure::failed)
        }),
        Request::RemoveRule { name, id } => with_session(&name, |s| {
            ops::remove_rule(backends.for_session(s), s, &id)
                .map(Reply::Policy)
                .map_err(Failure::failed)
        }),
        Request::Unlist { name, endpoint } => with_session(&name, |s| {
            ops::unlist(&endpoint).map_err(Failure::failed)?;
            policy(backends.for_session(s), s)
        }),
        Request::GitStatus { name } => with_session(&name, |s| {
            git::status(backends.for_session(s), s)
                .map(|status| Reply::Git {
                    said: String::new(),
                    status,
                })
                .map_err(Failure::failed)
        }),
        Request::GitDiff {
            name,
            path,
            against,
        } => with_session(&name, |s| {
            git::file_diff(backends.for_session(s), s, &path, against)
                .map(Reply::GitDiff)
                .map_err(Failure::failed)
        }),
        Request::Git { name, action } => with_session(&name, |s| {
            let said = match action {
                GitOp::Stage { path } => {
                    git::stage(backends.for_session(s), s, &path).map(|_| String::new())
                }
                GitOp::Unstage { path } => {
                    git::unstage(backends.for_session(s), s, &path).map(|_| String::new())
                }
                GitOp::Discard { path } => {
                    git::discard(backends.for_session(s), s, &path).map(|_| String::new())
                }
                GitOp::Commit { message } => git::commit(backends.for_session(s), s, &message),
                GitOp::Push => git::push(backends.for_session(s), s),
                GitOp::Pull => git::pull(backends.for_session(s), s),
                GitOp::Fetch => git::fetch(backends.for_session(s), s),
            }
            .map_err(Failure::failed)?;
            // Re-read rather than assume: the agent is editing while this runs,
            // so the status after a stage is not the status before it plus one
            // entry.
            let status = git::status(backends.for_session(s), s).map_err(Failure::failed)?;
            Ok(Reply::Git { said, status })
        }),
        Request::Files { name, path } => with_session(&name, |s| {
            files::list(backends.for_session(s), s, &path)
                .map(Reply::Files)
                .map_err(Failure::failed)
        }),
        Request::File { name, path } => with_session(&name, |s| {
            files::read(backends.for_session(s), s, &path)
                .map(Reply::File)
                .map_err(Failure::failed)
        }),
        Request::Shells { name } => with_session(&name, |s| {
            ops::shells(backends.for_session(s), s)
                .map(|shells| Reply::Shells { shells })
                .map_err(Failure::gateway)
        }),
        Request::NewShell { name } => with_session(&name, |s| {
            ops::new_shell(backends.for_session(s), s).map_err(Failure::failed)?;
            ops::shells(backends.for_session(s), s)
                .map(|shells| Reply::Shells { shells })
                .map_err(Failure::gateway)
        }),
        Request::Ports { name } => with_session(&name, |s| ports(backends.for_session(s), s)),
        Request::StopForward { name, port } => with_session(&name, |s| {
            crate::forward::stop_now(&s.sandbox, port);
            ports(backends.for_session(s), s)
        }),
        // The forward goes too: it would only ever answer with a refusal now.
        Request::KillPort { name, port } => with_session(&name, |s| {
            let backend = backends.for_session(s);
            ops::kill_port(backend, s, port).map_err(Failure::failed)?;
            crate::forward::stop_now(&s.sandbox, port);
            ports(backend, s)
        }),
        Request::KillProcess { name, pid } => with_session(&name, |s| {
            let backend = backends.for_session(s);
            ops::kill_process(backend, s, pid).map_err(Failure::failed)?;
            ports(backend, s)
        }),
        Request::KillShell { name, tmux } => with_session(&name, |s| {
            ops::kill_shell(backends.for_session(s), s, &tmux).map_err(Failure::failed)?;
            ops::shells(backends.for_session(s), s)
                .map(|shells| Reply::Shells { shells })
                .map_err(Failure::gateway)
        }),
        Request::Chats { name } => with_session(&name, |s| {
            chat::list(backends.for_session(s), s)
                .map(|chats| Reply::Chats {
                    chats,
                    opened: None,
                })
                .map_err(Failure::gateway)
        }),
        Request::NewChat { name } => with_session(&name, |s| {
            let backend = backends.for_session(s);
            let opened = chat::open(backend, s).map_err(Failure::failed)?;
            chat::list(backend, s)
                .map(|chats| Reply::Chats {
                    chats,
                    opened: Some(opened),
                })
                .map_err(Failure::gateway)
        }),
        Request::CloseChat { name, conv } => with_session(&name, |s| {
            let backend = backends.for_session(s);
            chat::close(backend, s, &conv).map_err(Failure::failed)?;
            chat::list(backend, s)
                .map(|chats| Reply::Chats {
                    chats,
                    opened: None,
                })
                .map_err(Failure::gateway)
        }),
        Request::Comments { name } => with_session(&name, |s| Ok(review(comments::list(&s.name)))),
        Request::Comment { name, comment } => with_session(&name, |s| {
            comments::add(&s.name, comment)
                .map(review)
                .map_err(Failure::failed)
        }),
        Request::Uncomment { name, id } => with_session(&name, |s| {
            comments::remove(&s.name, id)
                .map(review)
                .map_err(Failure::failed)
        }),
        Request::SendComments { name } => with_session(&name, |s| {
            ops::send_comments(backends.for_session(s), s)
                .map(|message| Reply::Told { message })
                .map_err(Failure::failed)
        }),
        Request::Projects => Reply::Projects {
            projects: projects::list(),
        }
        .into(),
        Request::NewProject(new) => match projects::add(new) {
            Ok(projects) => Reply::Projects { projects }.into(),
            Err(e) => Failure::failed(e).into(),
        },
        Request::ForgetProject { name } => match projects::remove(&name) {
            Ok(projects) => Reply::Projects { projects }.into(),
            Err(e) => Failure::failed(e).into(),
        },
        Request::Repos => repo_list(),
        Request::Inspect { path, branch } => inspect(backends, &path, branch.as_deref()),
        Request::NewOptions => match config::Config::load() {
            Ok(cfg) => Reply::NewOptions(ops::new_options(backends, &cfg)).into(),
            Err(e) => Failure::failed(format!("could not read the config file: {e}")).into(),
        },
        Request::Create(new) => create(*new),

        // The list, and not an acknowledgement: `destroy` answers whether it
        // deleted a sandbox or only a record, and neither is what a client
        // needs -- what it needs is the row gone. Reconciliation runs inside
        // `ls`, so anything else that changed underneath comes back too.
        Request::Destroy { name } => match ops::destroy(backends, &name) {
            Ok(_) => ls(backends),
            Err(e) => Failure::failed(e).into(),
        },

        // The integrations screen. Every one of these answers with the whole
        // view rather than an acknowledgement, for the reason the git view does
        // the same: they explain each other, and a client adjusting the list it
        // had would be inventing an answer.
        Request::Integrations => integrations(),
        Request::Mcp { name, action } => match mcp_action(&name, action) {
            Ok(()) => integrations(),
            Err(e) => Failure::failed(e).into(),
        },
        Request::Secret { name, value } => {
            let done = match value {
                Some(v) => secrets::set(&name, &v),
                None => secrets::forget(&name),
            };
            match done {
                Ok(()) => integrations(),
                Err(e) => Failure::failed(e).into(),
            }
        }
        Request::UploadSkills { skills } => upload_skills(skills),
        Request::ForgetSkill { name } => match skills::forget(&skills::library_dir(), &name) {
            Ok(()) => integrations(),
            Err(e) => Failure::failed(e).into(),
        },
        // The config file's editable defaults. Read and written here rather
        // than through `ops`, because there is no session in it: this is the
        // machine's own answer to what a *new* session starts with, which is
        // exactly what `hurad config` prints.
        Request::Settings => settings(),
        Request::SetSettings(wanted) => {
            match hura_core::settings::save(&config::Config::default_path(), &wanted) {
                // The reply is built from the config that came back out of the
                // file rather than from `wanted`, so a client is shown what was
                // saved and not what it asked for.
                Ok(cfg) => Reply::Settings(hura_core::settings::view(&cfg)).into(),
                Err(e) => Failure::failed(e.to_string()).into(),
            }
        }
    }
}

/// The config file as a settings screen should draw it.
///
/// A file that will not parse is a failure here rather than a screen full of
/// defaults: every command except `hura doctor` already refuses to run against
/// one, and a settings screen that showed the built-ins would invite somebody
/// to save over a file whose real contents it never managed to read.
fn settings() -> Outcome {
    match config::Config::load() {
        Ok(cfg) => Reply::Settings(hura_core::settings::view(&cfg)).into(),
        Err(e) => Failure::failed(format!("could not read the config file: {e}")).into(),
    }
}

/// The MCP catalog, the secret names and the skill library, as the server sees
/// them now.
fn integrations() -> Outcome {
    match config::Config::load() {
        Ok(cfg) => Reply::Integrations(hura_core::integrations::view(&cfg)).into(),
        Err(e) => Failure::failed(format!("could not read the config file: {e}")).into(),
    }
}

/// Start, restart or stop one managed server.
///
/// The catalog is the config file's, so a name a client sends is looked up
/// rather than trusted: `stop` takes a container name, and one derived from an
/// arbitrary string would let a client stop any container on the host whose name
/// begins with the prefix.
fn mcp_action(name: &str, action: McpOp) -> Result<(), String> {
    let cfg = config::Config::load().map_err(|e| format!("could not read the config file: {e}"))?;
    let entry = cfg
        .mcp()
        .iter()
        .find(|e| e.name() == name)
        .ok_or_else(|| format!("no mcp server named `{name}` in the config file"))?;
    if !entry.is_managed() {
        return Err(format!(
            "`{name}` is a url this server does not run, so there is nothing here to {}",
            match action {
                McpOp::Stop => "stop",
                _ => "start",
            }
        ));
    }
    match action {
        McpOp::Start => hura_core::mcp::ensure(std::slice::from_ref(entry))
            .into_iter()
            .next()
            .map_or(Ok(()), Err),
        McpOp::Restart => hura_core::mcp::start(entry),
        McpOp::Stop => hura_core::mcp::stop(entry.name()),
    }
}

/// Take a client's skills into the library.
///
/// Every one is attempted: a client uploads its whole `~/.claude/skills` before
/// a create, and one skill that has grown a virtualenv should not stop the rest
/// arriving. What went wrong comes back as a failure only when *nothing*
/// landed, since a screen that re-reads the view can see for itself what is
/// there.
fn upload_skills(uploads: Vec<hura_core::skills::Upload>) -> Outcome {
    let dir = skills::library_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Failure::failed(format!("could not make {}: {e}", dir.display())).into();
    }
    let mut problems = Vec::new();
    let mut installed = 0;
    for upload in &uploads {
        match skills::install(&dir, upload) {
            Ok(_) => installed += 1,
            Err(e) => problems.push(e),
        }
    }
    if installed == 0 && !problems.is_empty() {
        return Failure::failed(problems.join("; ")).into();
    }
    if !problems.is_empty() {
        eprintln!(
            "hurad: some skills were not stored: {}",
            problems.join("; ")
        );
    }
    integrations()
}

fn review(comments: Vec<comments::Comment>) -> Reply {
    Reply::Comments { comments }
}

/// The repositories on this machine, and where it looked for them.
fn repo_list() -> Outcome {
    let cfg = config::Config::load().unwrap_or_default();
    let roots = repos::roots(cfg.repo_roots.as_deref());
    let repos = repos::discover_in(&roots);
    Reply::Repos(repos::Listing {
        roots: roots.iter().map(|r| r.path.display().to_string()).collect(),
        repos,
    })
    .into()
}

/// What is known about the repository a client has picked.
///
/// The provider half fails softly, like the list in [`ops::new_options`]: a
/// list that cannot be read leaves nothing ticked, which is a form you
/// can still fill in, rather than an error against a question that was mostly
/// about git.
fn inspect(backends: &Backends, path: &str, branch: Option<&str>) -> Outcome {
    let path = Path::new(path);
    let checkout = repos::read(path);
    // `None` means the checkout's own branch, which is what the request says it
    // means -- and `inspect` cannot work that out for itself, because it is
    // handed a path rather than the record `read` produces. Left unresolved it
    // reports every branch as missing from the remote, and a form built on that
    // silently falls back to the remote's default.
    let branch = branch
        .map(str::to_string)
        .or_else(|| checkout.branch.clone());
    let facts = repos::inspect(path, branch.as_deref());
    let cfg = config::Config::load().unwrap_or_default();

    // An explicit list in the config file replaces this rather than adding to
    // it, so there is nothing to work out when there is one.
    let providers = if !cfg.providers().is_empty() {
        Vec::new()
    } else {
        let origin = checkout.origin.clone();
        let choices: Vec<ops::ProviderChoice> =
            backends.sandboxed().providers().unwrap_or_default();
        let sessions: Result<Vec<Session>, _> =
            Store::load().map(|s| s.list().into_iter().cloned().collect());
        let used = match (&origin, &sessions) {
            (Some(url), Ok(list)) => ops::providers_used_for(url, list),
            _ => Vec::new(),
        };
        ops::preselect_providers(&choices, origin.as_deref(), &used)
    };

    Reply::Inspect(ops::Picked {
        facts,
        branch: checkout.branch,
        providers,
    })
    .into()
}

/// Start a session, and answer before it has finished starting.
///
/// Everything that can be judged from the request is judged here, so a name
/// with a slash in it or a toolchain nobody has heard of comes back as an error
/// against the request that caused it. What is left is seconds of sandbox
/// runtime and network, and that runs on a thread: the states it passes through
/// are on the session, and the session is already polled.
fn create(new: hura_core::ops::NewSession) -> Outcome {
    let cfg = match config::Config::load() {
        Ok(c) => c,
        Err(e) => return Failure::failed(format!("could not read the config file: {e}")).into(),
    };

    // Built here and not on the thread: naming, validating and resolving the
    // toolchains are everything that can fail on the client's account, and they
    // belong to the request that caused them.
    // A store that will not load leaves nothing to step around; `ops::create`
    // loads it again and is the one that reports why.
    let taken = Store::load().map(|s| s.names()).unwrap_or_default();
    let draft = match new.into_draft(&cfg, &taken) {
        Ok(d) => d,
        Err(e) => return Failure::failed(e).into(),
    };
    let name = draft.name.clone();

    std::thread::spawn(move || {
        // The CLI builds the image before creating, because the build streams
        // docker's output to a terminal. There is no terminal here, so it
        // happens on this thread with the session sitting in `creating` -- which
        // is what a client watching the list sees either way, only for longer
        // the first time a set of toolchains is used.
        let backends = backends();
        // Said on the record as well as here, because the window that asked
        // for this session reads the record and nothing else. `ops::create`
        // records its own failures, so for those this only prints; the ones it
        // cannot record are the image build below and its refusals before its
        // first write, which would otherwise leave no session at all.
        let failed = |why: String| {
            eprintln!("hurad: {}: {why}", draft.name);
            if let Err(e) = ops::record_failure(&draft, &why) {
                eprintln!("hurad: {}: could not record why: {e}", draft.name);
            }
        };
        let image = match draft.interface {
            chat::Interface::Terminal => image::ensure_for(&draft.toolchains),
            chat::Interface::Chat => image::ensure_chat(&draft.toolchains),
        };
        if let Err(e) = image {
            failed(format!("could not build the image: {e}"));
            return;
        }
        // The managed MCP containers, before the seeder registers them with the
        // agent. Here rather than in `ops::create` for the same reason the image
        // build is here: it is a side effect on the host with output of its own,
        // and `ops` is what both front ends share. A server that will not start
        // is a warning -- the session is still worth having, and the agent will
        // report the tool as unreachable, which the events pane explains.
        for warning in hura_core::mcp::ensure(cfg.mcp()) {
            eprintln!("hurad: {}: {warning}", draft.name);
        }
        // Progress is dropped rather than reported: the steps map onto states
        // the record already carries, and a channel per create would be a second
        // way to learn the same thing.
        if let Err(e) = ops::create(&backends, &draft, &mut |_| {}) {
            failed(format!("could not create the session: {e}"));
        }
    });

    Reply::Created { name }.into()
}

fn ls(backends: &Backends) -> Outcome {
    // `repair` is false: it costs one exec per session left mid-lifecycle, and
    // is the right thing for a tool that starts, prints and exits. A server
    // answering this every second for every connected client would spend an
    // exec a second on a question that only changes when a create dies. The
    // repair happens once, at startup, in `serve`.
    match ops::refresh_with(backends, false) {
        Ok(refreshed) => Reply::from(refreshed).into(),
        Err(e) => Failure::gateway(e.to_string()).into(),
    }
}

/// The policy pane's contents.
fn policy(backend: &dyn Backend, session: &Session) -> Result<Reply, Failure> {
    ops::policy(backend, session)
        .map(Reply::Policy)
        .map_err(Failure::gateway)
}

fn events(backend: &dyn Backend, session: &Session) -> Result<Reply, Failure> {
    let events = ops::events(backend, session).map_err(Failure::gateway)?;
    Ok(Reply::Events { events })
}

/// The backend, as this server holds it.
///
/// Built per use rather than kept in a `static`: a client is a path, and
/// building one costs a config read. What that buys is an `hurad` that picks up
/// an edited `config.toml` without a restart, which is the same promise every
/// other read here makes.
pub fn backends() -> Backends {
    Backends::from_config(&config::Config::load().unwrap_or_default())
}

/// Look a session up by name, and answer for it.
///
/// The lookup is against the cache rather than the runtime, which is what
/// `require_session` in the CLI does too: the cache is reconciled by `Ls`, and
/// a name that is not in it is a client asking about something that has gone.
fn with_session(name: &str, f: impl FnOnce(&Session) -> Result<Reply, Failure>) -> Outcome {
    let store = match Store::load() {
        Ok(s) => s,
        Err(e) => return Failure::failed(format!("could not read the session cache: {e}")).into(),
    };
    let Some(session) = store.get(name) else {
        return Failure::no_such_session(name).into();
    };
    match f(session) {
        Ok(reply) => reply.into(),
        Err(failure) => failure.into(),
    }
}

/// A sandbox's ports: what is listening and who holds it, from the sandbox;
/// what is being forwarded into it, from this process.
fn ports(backend: &dyn Backend, s: &Session) -> Result<Reply, Failure> {
    let found = ops::sandbox_ports(backend, s).map_err(Failure::failed)?;
    Ok(Reply::Ports(hura_core::ports::PortsView {
        listening: found.listening,
        processes: found.processes,
        forwards: crate::forward::list(&s.sandbox),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cache is a real file in a real home directory, so the only arm that
    /// can be exercised without one is the miss -- which is also the one a
    /// client hits most, since a session it knew about can go at any time.
    #[test]
    fn a_request_for_a_session_that_is_not_there_says_so_by_kind() {
        let out = with_session("definitely-not-a-session-4f2a", |_| {
            panic!("should not have been called")
        });
        let err = out.into_result().unwrap_err();
        assert_eq!(err.kind, hura_proto::FailureKind::NoSuchSession);
        assert!(err.message.contains("definitely-not-a-session-4f2a"));
    }

    /// Every request names the op it failed on, so a client can tell an
    /// unsupported request from a broken one.
    #[test]
    fn an_unsupported_op_names_itself_and_the_protocol() {
        let f = Failure::unsupported("attach");
        assert_eq!(f.kind, hura_proto::FailureKind::Unsupported);
        assert!(f.message.contains("attach"), "{}", f.message);
        assert!(
            f.message.contains(&hura_proto::VERSION.to_string()),
            "{}",
            f.message
        );
    }
}
