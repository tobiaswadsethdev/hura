//! The websocket: one connection, several channels.
//!
//! Each channel is a task producing [`ServerFrame`]s into one queue, and a
//! single writer drains that queue onto the socket. That shape is what keeps
//! the terminal responsive while an events poll is waiting on the runtime: the
//! slow channel blocks itself and nothing else.
//!
//! **Polling lives here, not in the client.** A client that asked `/rpc` for
//! events every second would spend a TLS handshake per session per second to be
//! told nothing had changed. The server is next to the sandbox runtime, so it
//! does the asking and sends only what is new, which is also the only way a
//! second client watching the same session does not double the load on it.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use hura_core::chat::{self, ChatCommand, ChatFrame};
use hura_core::events::Event;
use hura_core::ops;
use hura_core::ports::Loopback;
use hura_core::seed;
use hura_core::session::{Session, State};
use hura_core::store::Store;
use hura_proto::stream::{Channel, ChannelId, ClientFrame, ServerFrame, bytes};
use tokio::sync::mpsc;

/// How often a feed or a status channel asks the sandbox runtime.
///
/// Slower than the terminal interface's own second, deliberately. Each of these
/// is an exec against the sandbox, about a third of a second each, and a
/// server may be answering several clients about several sessions at once,
/// where the TUI was one process watching one machine.
const POLL: Duration = Duration::from_secs(2);

/// How much output may queue for a client that has stopped reading.
///
/// A terminal that produces faster than the socket drains -- `yes`, a build --
/// must not grow a queue until the server runs out of memory. When this fills,
/// the channel closes and says so, which is recoverable; the alternative is not.
const BACKLOG: usize = 256;

// Checked where they are written rather than in a test, because both are
// judgements about a number and a test that folds to `assert!(true)` proves
// nothing at run time.
const _: () = assert!(
    BACKLOG > 0 && BACKLOG <= 1024,
    "a backlog this large is not a bound"
);
const _: () = assert!(
    POLL.as_secs() >= 1 && POLL.as_secs() <= 5,
    "a server polls for every client at once; a TUI's own second is too fast here"
);

/// What a channel accepts while it is running: bytes and a size for a terminal
/// or a port, commands for a chat.
enum ToChannel {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Chat(ChatCommand),
}

pub async fn run(socket: WebSocket) {
    let (mut sink, mut source) = {
        use futures_util::StreamExt as _;
        socket.split()
    };

    let (out, mut queued) = mpsc::channel::<ServerFrame>(BACKLOG);

    // One writer. Every channel produces into `out`, so nothing else ever
    // touches the socket and frames cannot interleave halfway.
    let writer = tokio::spawn(async move {
        use futures_util::SinkExt as _;
        while let Some(frame) = queued.recv().await {
            let Ok(text) = serde_json::to_string(&frame) else {
                continue;
            };
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    let mut channels: HashMap<ChannelId, ChannelHandle> = HashMap::new();

    use futures_util::StreamExt as _;
    while let Some(Ok(message)) = source.next().await {
        let Message::Text(text) = message else {
            // Binary and ping/pong are not part of this protocol; axum answers
            // pings itself.
            continue;
        };
        let Ok(frame) = serde_json::from_str::<ClientFrame>(&text) else {
            continue;
        };

        match frame {
            ClientFrame::Open { id, channel } => {
                // Re-opening an id closes what was there. A client that has
                // lost track is better served by the newer intent than by a
                // silent refusal.
                if let Some(old) = channels.remove(&id) {
                    old.shutdown().await;
                }
                match open(id, channel, out.clone()).await {
                    Ok(handle) => {
                        channels.insert(id, handle);
                    }
                    Err(reason) => {
                        let _ = out
                            .send(ServerFrame::Closed {
                                id,
                                reason: Some(reason),
                            })
                            .await;
                    }
                }
            }
            ClientFrame::Close { id } => {
                if let Some(handle) = channels.remove(&id) {
                    handle.shutdown().await;
                    let _ = out.send(ServerFrame::Closed { id, reason: None }).await;
                }
            }
            ClientFrame::Input { id, data } => {
                if let (Some(handle), Some(raw)) = (channels.get(&id), bytes::decode(&data)) {
                    handle.send(ToChannel::Input(raw)).await;
                }
            }
            ClientFrame::Resize { id, cols, rows } => {
                if let Some(handle) = channels.get(&id) {
                    handle.send(ToChannel::Resize { cols, rows }).await;
                }
            }
            ClientFrame::Chat { id, command } => {
                if let Some(handle) = channels.get(&id) {
                    handle.send(ToChannel::Chat(command)).await;
                }
            }
        }
    }

    // The socket has gone. Every channel goes with it, and a terminal detaches
    // rather than being killed -- see `terminal`.
    for (_, handle) in channels.drain() {
        handle.shutdown().await;
    }
    drop(out);
    let _ = writer.await;
}

struct ChannelHandle {
    task: tokio::task::JoinHandle<()>,
    to_terminal: Option<mpsc::Sender<ToChannel>>,
    /// Whether ending this channel means typing tmux's detach into it first.
    /// A terminal's does; a port's must not, since those two bytes would be
    /// written into somebody's HTTP request.
    detach: bool,
}

impl ChannelHandle {
    async fn send(&self, message: ToChannel) {
        if let Some(tx) = &self.to_terminal {
            let _ = tx.send(message).await;
        }
    }

    /// End the channel.
    ///
    /// A terminal is asked to detach and given a moment to do it; everything
    /// else is simply dropped. See `terminal` for why the difference matters.
    async fn shutdown(self) {
        if let Some(tx) = self.to_terminal.as_ref().filter(|_| self.detach) {
            let _ = tx.send(ToChannel::Input(DETACH.to_vec())).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        self.task.abort();
    }
}

/// `Ctrl-b d`: what a person types to leave a tmux session.
///
/// The alternative would be killing the exec. Under OpenShell that wedged the
/// exec path for the sandbox until it was recreated, so leaving the way a
/// person does became the rule. The sandbox runtime does not wedge, measured,
/// so it is now only the gentler of the two.
const DETACH: &[u8] = b"\x02d";

async fn open(
    id: ChannelId,
    channel: Channel,
    out: mpsc::Sender<ServerFrame>,
) -> Result<ChannelHandle, String> {
    let name = channel.session().to_string();
    let session = tokio::task::spawn_blocking(move || {
        Store::load()
            .map_err(|e| format!("could not read the session cache: {e}"))?
            .get(&name)
            .cloned()
            .ok_or_else(|| format!("no session named `{name}`"))
    })
    .await
    .map_err(|e| e.to_string())??;

    let _ = out.send(ServerFrame::Opened { id }).await;

    Ok(match channel {
        Channel::Events { .. } => ChannelHandle {
            task: tokio::spawn(events(id, session, out)),
            to_terminal: None,
            detach: false,
        },
        Channel::Status { .. } => ChannelHandle {
            task: tokio::spawn(status(id, session, out)),
            to_terminal: None,
            detach: false,
        },
        Channel::Terminal { tmux, .. } => {
            let (tx, rx) = mpsc::channel(64);
            // Defaulted here rather than in the worker, so everything below
            // this point has one name for its target and no opinion about
            // which tmux session is special.
            let target = tmux.clone().unwrap_or_else(|| session.tmux.clone());
            ChannelHandle {
                task: tokio::spawn(terminal(id, session, target, out, rx)),
                to_terminal: Some(tx),
                detach: true,
            }
        }
        Channel::Port { port, host, .. } => {
            let (tx, rx) = mpsc::channel(BACKLOG);
            ChannelHandle {
                task: tokio::spawn(port_channel(id, session, port, host, out, rx)),
                to_terminal: Some(tx),
                detach: false,
            }
        }
        Channel::Chat {
            conv,
            since,
            fetch_pictures,
            ..
        } => {
            let (tx, rx) = mpsc::channel(64);
            let attach = chat::Attach::new(&conv, since, fetch_pictures);
            ChannelHandle {
                task: tokio::spawn(chat_channel(id, session, attach, out, rx)),
                to_terminal: Some(tx),
                detach: false,
            }
        }
    })
}

/// How long a chat channel waits for the host once the session is past
/// seeding. The host starts in well under a second and restarts itself in two,
/// so a minute without one is something wrong rather than something slow.
const HOST_WAIT_LIMIT: Duration = Duration::from_secs(60);
const HOST_WAIT_EVERY: Duration = Duration::from_secs(1);

/// One conversation in a chat session, from the host inside the sandbox.
///
/// Reached through the same forward a preview uses: the host listens on a
/// loopback port in there, and a forward is a `hurad relay` to that. A forward
/// is safe to stop, so nothing here has to be careful about how the channel
/// ends.
///
/// The host can be missing for a while, and the channel waits it out rather
/// than closing: through seeding, since the seeder is what starts it, and
/// through a restart, after which the channel attaches again from the last
/// entry it passed on. The client sees one channel throughout, with a notice
/// while there is nothing to show.
async fn chat_channel(
    id: ChannelId,
    session: Session,
    attach: chat::Attach,
    out: mpsc::Sender<ServerFrame>,
    mut input: mpsc::Receiver<ToChannel>,
) {
    let reason = chat(id, &session, attach, &out, &mut input).await;
    let _ = out.send(ServerFrame::Closed { id, reason }).await;
}

async fn chat(
    id: ChannelId,
    session: &Session,
    mut attach: chat::Attach,
    out: &mpsc::Sender<ServerFrame>,
    input: &mut mpsc::Receiver<ToChannel>,
) -> Option<String> {
    if session.interface != chat::Interface::Chat {
        return Some(format!(
            "`{}` has a terminal agent, not a chat",
            session.name
        ));
    }
    let notice = |text: &str| ServerFrame::Chat {
        id,
        frame: ChatFrame::Notice {
            text: text.to_string(),
        },
    };

    let mut waited = Duration::ZERO;
    let mut said = None;
    let mut held: Vec<ChatCommand> = Vec::new();
    loop {
        // Seeding is the seeder's to finish, and the host is the last thing
        // it starts. Waited out on its own limit, which is the clone's.
        let s = session.clone();
        let still_seeding = tokio::task::spawn_blocking(move || {
            let backends = crate::rpc::backends();
            seeding(backends.for_session(&s), &s)
        })
        .await
        .unwrap_or(false);

        if !still_seeding {
            let lease =
                match crate::forward::acquire(session, chat::port(session), Loopback::V4).await {
                    Ok(lease) => lease,
                    Err(reason) => return Some(reason),
                };
            match lease.connect().await {
                Ok(stream) => {
                    match converse(id, stream, &mut attach, &mut held, out, input).await {
                        Conversed::ClientGone => return None,
                        // Talking, then not: the host stopped, and is starting
                        // again. The wait begins afresh.
                        Conversed::HostGone { heard: true } => {
                            waited = Duration::ZERO;
                            said = None;
                        }
                        Conversed::HostGone { heard: false } => {}
                        Conversed::Refused(reason) => return Some(reason),
                    }
                }
                Err(e) => return Some(format!("could not reach the forward: {e}")),
            }
        }

        let (text, limit) = if still_seeding {
            ("cloning, and starting the agent", SEED_WAIT_LIMIT)
        } else {
            ("waiting for the agent to start", HOST_WAIT_LIMIT)
        };
        if said != Some(text) {
            said = Some(text);
            if out.send(notice(text)).await.is_err() {
                return None;
            }
        }
        if waited >= limit {
            return Some(format!(
                "the agent did not start in {}s; its log is in the sandbox, where `hurad attach {}` shows it",
                limit.as_secs(),
                session.name
            ));
        }
        // The client closing the tab ends the wait, as it ends a terminal's.
        // Anything said in the meantime is held for the host rather than
        // dropped: a message typed into a session that is still cloning is
        // the first thing its agent should hear.
        tokio::select! {
            _ = tokio::time::sleep(HOST_WAIT_EVERY) => waited += HOST_WAIT_EVERY,
            message = input.recv() => match message {
                None => return None,
                Some(ToChannel::Chat(command)) => held.push(command),
                Some(_) => {}
            },
        }
    }
}

enum Conversed {
    /// The tab was closed, or the client went.
    ClientGone,
    /// The connection to the host ended. `heard` is whether it ever said
    /// anything: a forward to a port nobody is listening on accepts and then
    /// closes, which is what a host that has not started yet looks like.
    HostGone { heard: bool },
    /// The host refused what was asked of it, and will go on refusing.
    Refused(String),
}

/// Attach to one conversation and carry frames both ways until one end goes.
/// `attach.since` follows what has been passed on, so attaching again after
/// the host restarts asks for what came after that.
async fn converse(
    id: ChannelId,
    stream: tokio::net::TcpStream,
    attach: &mut chat::Attach,
    held: &mut Vec<ChatCommand>,
    out: &mpsc::Sender<ServerFrame>,
    input: &mut mpsc::Receiver<ToChannel>,
) -> Conversed {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

    let _ = stream.set_nodelay(true);
    let (read, mut write) = stream.into_split();
    let mut lines = tokio::io::BufReader::new(read).lines();

    let mut hello = match serde_json::to_string(&*attach) {
        Ok(text) => text,
        Err(e) => return Conversed::Refused(e.to_string()),
    };
    hello.push('\n');
    if write.write_all(hello.as_bytes()).await.is_err() {
        return Conversed::HostGone { heard: false };
    }
    // What was said while there was no host, in the order it was said. Kept
    // until the write has gone, so a host that drops now still gets it from
    // the next attach.
    while let Some(command) = held.first() {
        let Ok(mut text) = serde_json::to_string(command) else {
            held.remove(0);
            continue;
        };
        text.push('\n');
        if write.write_all(text.as_bytes()).await.is_err() {
            return Conversed::HostGone { heard: false };
        }
        held.remove(0);
    }

    let mut heard = false;
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    // A frame this server does not know is skipped rather than
                    // ending the channel: the host and the server ship
                    // together, but a transcript outlives both.
                    let Ok(frame) = serde_json::from_str::<ChatFrame>(&line) else {
                        continue;
                    };
                    // Refused on attaching is for good: there is no such
                    // conversation, and asking again will not make one.
                    if let ChatFrame::Error { message } = &frame && !heard {
                        return Conversed::Refused(message.clone());
                    }
                    heard = true;
                    match &frame {
                        ChatFrame::Replay { seq, .. } => attach.since = attach.since.max(*seq),
                        ChatFrame::Entry { entry } => attach.since = attach.since.max(entry.seq),
                        _ => {}
                    }
                    // Waited for rather than dropped: a replay is one large
                    // frame, and a client still drawing the last one should
                    // slow the host down, not lose the conversation.
                    if out.send(ServerFrame::Chat { id, frame }).await.is_err() {
                        return Conversed::ClientGone;
                    }
                }
                Ok(None) | Err(_) => return Conversed::HostGone { heard },
            },
            message = input.recv() => match message {
                None => return Conversed::ClientGone,
                Some(ToChannel::Chat(command)) => {
                    let Ok(mut text) = serde_json::to_string(&command) else {
                        continue;
                    };
                    text.push('\n');
                    if write.write_all(text.as_bytes()).await.is_err() {
                        return Conversed::HostGone { heard };
                    }
                }
                // Keystrokes and sizes are a terminal's.
                Some(_) => {}
            },
        }
    }
}

/// One connection to a port in the sandbox, through the forward for it.
///
/// What arrives as `Input` is written to the service and what the service
/// answers goes back as `Output`, until either side closes. Input sent before
/// the forward is up waits in the channel rather than being lost: the client
/// writes the request as soon as it has opened, and the first connection to a
/// port is the one that starts its forward.
async fn port_channel(
    id: ChannelId,
    session: Session,
    port: u16,
    host: hura_core::ports::Loopback,
    out: mpsc::Sender<ServerFrame>,
    mut input: mpsc::Receiver<ToChannel>,
) {
    let mut seen = Seen::default();
    let mut https = None;
    let reason = match crate::forward::acquire(&session, port, host).await {
        Err(reason) => Some(reason),
        Ok(lease) => {
            let reason = pipe(id, &lease, &out, &mut input, &mut seen).await;
            if seen.unanswered
                && let Some(location) = https_location(&seen.head)
                && speaks_tls(lease.connect()).await
            {
                https = Some(location);
            }
            reason
        }
    };
    // Closing the client's connection says nothing to a browser but "the
    // network connection was lost", so one that has had no answer is told why
    // in words it shows -- or, when what it asked in plain HTTP is a server
    // that only speaks TLS, sent there.
    let answer = match (&https, reason.as_deref()) {
        (Some(location), _) => Some(redirect(location)),
        (None, Some(why)) if !seen.answered => Some(unreachable(port, why)),
        _ => None,
    };
    if let Some(answer) = answer {
        let frame = ServerFrame::Output {
            id,
            data: bytes::encode(&answer),
        };
        let _ = out.send(frame).await;
    }
    let _ = out.send(ServerFrame::Closed { id, reason }).await;
}

/// What a port channel's connection did, for what to say when it ends.
#[derive(Default)]
struct Seen {
    /// The start of what the client sent, enough for a request line and its
    /// `Host`.
    head: Vec<u8>,
    /// Whether the service sent anything back.
    answered: bool,
    /// Whether the service closed with the client's question unanswered.
    unanswered: bool,
}

/// How much of a request is kept to read its line and `Host` from.
const HEAD: usize = 8 * 1024;

/// Where a plain HTTP request would be over TLS: `https://`, its `Host`, its
/// path. `None` for anything that is not an HTTP request in origin form.
fn https_location(head: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let mut line = lines.next()?.split(' ');
    let (method, target, version) = (line.next()?, line.next()?, line.next()?);
    if !version.starts_with("HTTP/1.") || !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    if !target.starts_with('/') || !target.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    let host = lines
        .take_while(|l| !l.is_empty())
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("host").then(|| value.trim())
        })
        .filter(|h| {
            !h.is_empty()
                && h.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
        })?;
    Some(format!("https://{host}{target}"))
}

/// Whether the service answers a TLS `ClientHello` as TLS does: with a
/// handshake record, or an alert. A dev server set up for HTTPS -- webpack's
/// `server: 'https'` -- closes a plain HTTP request without a word, which is
/// all a browser opening `http://` would ever see of it.
async fn speaks_tls(
    connect: impl std::future::Future<Output = std::io::Result<tokio::net::TcpStream>>,
) -> bool {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let Some(hello) = client_hello() else {
        return false;
    };
    let probe = async {
        let mut stream = connect.await.ok()?;
        stream.write_all(&hello).await.ok()?;
        let mut first = [0u8; 1];
        stream.read_exact(&mut first).await.ok()?;
        Some(first[0])
    };
    matches!(
        tokio::time::timeout(Duration::from_secs(3), probe).await,
        Ok(Some(0x16 | 0x15))
    )
}

/// A real `ClientHello`, from rustls. Nothing after it is read past the
/// first byte, so no certificate is ever looked at.
fn client_hello() -> Option<Vec<u8>> {
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .ok()?
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from("localhost").ok()?;
    let mut conn = rustls::ClientConnection::new(std::sync::Arc::new(config), name).ok()?;
    let mut hello = Vec::new();
    conn.write_tls(&mut hello).ok()?;
    Some(hello)
}

/// The answer sending a plain HTTP request to the same place over TLS.
/// Temporary, so a browser asks again next time rather than remembering a
/// server that may serve plain HTTP tomorrow.
fn redirect(location: &str) -> Vec<u8> {
    let body = format!("This server speaks HTTPS: {location}\n");
    format!(
        "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\n\
         Content-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// The answer to a connection that could not be carried to the service.
fn unreachable(port: u16, why: &str) -> Vec<u8> {
    let body = format!("hura could not reach port {port} in the sandbox: {why}\n");
    format!(
        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

async fn pipe(
    id: ChannelId,
    lease: &crate::forward::Lease,
    out: &mpsc::Sender<ServerFrame>,
    input: &mut mpsc::Receiver<ToChannel>,
    seen: &mut Seen,
) -> Option<String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let stream = match lease.connect().await {
        Ok(stream) => stream,
        Err(e) => return Some(format!("could not reach the forward: {e}")),
    };
    let _ = stream.set_nodelay(true);
    let (mut from_service, mut to_service) = stream.into_split();
    let mut buf = vec![0u8; 32 * 1024];
    let mut asked = false;

    loop {
        tokio::select! {
            message = input.recv() => match message {
                Some(ToChannel::Input(raw)) => {
                    asked = true;
                    let room = HEAD.saturating_sub(seen.head.len());
                    seen.head.extend_from_slice(&raw[..raw.len().min(room)]);
                    if let Err(e) = to_service.write_all(&raw).await {
                        return Some(e.to_string());
                    }
                }
                Some(ToChannel::Resize { .. } | ToChannel::Chat(_)) => {}
                // The client closed its end.
                None => return None,
            },
            read = from_service.read(&mut buf) => match read {
                // The relay in the sandbox accepts and then closes when it
                // cannot reach the port, so an end with a question unanswered
                // is that, not a service that had nothing to say.
                Ok(0) if asked && !seen.answered => {
                    seen.unanswered = true;
                    return Some("nothing answered on it; is the server still running?".into());
                }
                Ok(0) => return None,
                Ok(n) => {
                    seen.answered = true;
                    let frame = ServerFrame::Output { id, data: bytes::encode(&buf[..n]) };
                    if out.send(frame).await.is_err() {
                        return None;
                    }
                }
                Err(e) => return Some(e.to_string()),
            },
        }
    }
}

/// The allow/deny feed, as decisions are made.
///
/// The first frame carries the recent log and every frame after it carries the
/// difference, keyed on what the core already considers the same event. The
/// runtime counts on in one row, so an event is sent again when its count or
/// its last sighting moves, and a client keeps the newest it was sent for a key.
async fn events(id: ChannelId, session: Session, out: mpsc::Sender<ServerFrame>) {
    let mut seen: HashMap<(u64, String, String), (u64, u64)> = HashMap::new();

    loop {
        let s = session.clone();
        let fetched = tokio::task::spawn_blocking(move || {
            let backends = crate::rpc::backends();
            ops::events(backends.for_session(&s), &s)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));

        match fetched {
            Ok(all) => {
                let fresh: Vec<Event> = all
                    .into_iter()
                    .filter(|e| seen.insert(e.key(), (e.count, e.last)) != Some((e.count, e.last)))
                    .collect();
                if !fresh.is_empty()
                    && out
                        .send(ServerFrame::Events { id, events: fresh })
                        .await
                        .is_err()
                {
                    return;
                }
            }
            // A log that cannot be read is usually a sandbox that has just gone.
            // The channel says so and stops rather than retrying into nothing.
            Err(reason) => {
                let _ = out
                    .send(ServerFrame::Closed {
                        id,
                        reason: Some(reason),
                    })
                    .await;
                return;
            }
        }

        tokio::time::sleep(POLL).await;
    }
}

/// What the agent is doing, sent when it changes.
///
/// Only on change, which is what makes this cheaper than the client asking:
/// most polls of a session that is thinking return the same answer.
async fn status(id: ChannelId, mut session: Session, out: mpsc::Sender<ServerFrame>) {
    let mut last: Option<ops::Poll> = None;

    loop {
        let s = session.clone();
        let Ok(poll) = tokio::task::spawn_blocking(move || {
            let backends = crate::rpc::backends();
            ops::poll(backends.for_session(&s), &s)
        })
        .await
        else {
            return;
        };
        // `ops::poll` has written a moved branch to the record; this copy
        // follows, or every poll after would see the same move and write it.
        if let Some(branch) = &poll.branch {
            session.work_branch.clone_from(branch);
        }

        if last.as_ref() != Some(&poll) {
            last = Some(poll.clone());
            if out
                .send(ServerFrame::Status {
                    id,
                    poll: Box::new(poll),
                })
                .await
                .is_err()
            {
                return;
            }
        }

        tokio::time::sleep(POLL).await;
    }
}

/// The agent's terminal.
///
/// The same `exec --tty` and the same shell that `hura attach` runs, from one
/// definition in `ops::attach_script`, so an embedded terminal and a terminal
/// on the machine itself cannot end up attaching differently.
///
/// **It runs under a pty on *this* side, and it has to.** An interactive exec
/// allocates a pty at the sandbox end, which is enough for the process in there
/// to have a terminal, but the CLI that carries it wants a terminal here too.
/// Measured against OpenShell 0.0.110, whose CLI would not proxy through plain
/// pipes: with stdin an open pipe it wrote nothing at all. The channel opened,
/// the child ran, and no byte ever arrived.
///
/// So the pty is local as well, and the child is spawned into it exactly as a
/// terminal emulator would. That is what `interactive_exec_argv` is shaped for:
/// spawning under a pty needs the program and its arguments apart.
///
/// Resizing is then the pty's own, rather than an exec running
/// `tmux resize-window`. Better in two ways: the client's size reaches tmux the
/// way any terminal's does, and there is no second exec, a third of a second
/// each, for every change of size.
async fn terminal(
    id: ChannelId,
    session: Session,
    tmux: String,
    out: mpsc::Sender<ServerFrame>,
    mut input: mpsc::Receiver<ToChannel>,
) {
    let (from_pty, mut output) = mpsc::channel::<Vec<u8>>(BACKLOG);
    let (to_pty, pty_input) = std::sync::mpsc::channel::<ToChannel>();

    let worker = {
        let session = session.clone();
        std::thread::spawn(move || pty_worker(session, tmux, from_pty, pty_input))
    };

    let mut reason = None;
    loop {
        tokio::select! {
            chunk = output.recv() => match chunk {
                // The worker has finished: the child exited, or the pty failed.
                None => break,
                Some(raw) => {
                    let frame = ServerFrame::Output { id, data: bytes::encode(&raw) };
                    // A full queue is a client that has stopped draining.
                    // Ending the channel is recoverable; growing is not.
                    if out.try_send(frame).is_err() {
                        reason = Some("the client stopped reading its terminal".to_string());
                        break;
                    }
                }
            },
            message = input.recv() => match message {
                None => break,
                Some(message) => {
                    if to_pty.send(message).is_err() {
                        break;
                    }
                }
            },
        }
    }

    // Dropped rather than killed. Closing this end hangs up the pty, the child
    // sees it and exits; nothing sends it a signal. See `DETACH`.
    drop(to_pty);
    let _ = tokio::task::spawn_blocking(move || worker.join()).await;
    let _ = out.send(ServerFrame::Closed { id, reason }).await;
}

/// How long a terminal waits for its sandbox before giving up on it.
///
/// Shorter than the create's own wait: by the time a tab is opened the sandbox
/// is normally long since up, so a wait this side means something is wrong
/// rather than something is slow, and an empty pane that says nothing for five
/// minutes is worse than one that admits it.
const ATTACH_WAIT_LIMIT: Duration = Duration::from_secs(60);
const ATTACH_WAIT_EVERY: Duration = Duration::from_millis(500);
/// How long the agent's terminal waits for seeding to finish. As long as the
/// create's own watch, because this is waiting on the same clone.
const SEED_WAIT_LIMIT: Duration = Duration::from_secs(15 * 60);

/// Whether a session's seeder is still working, and so has not started the
/// agent yet.
fn seeding(backend: &dyn hura_core::backend::Backend, session: &Session) -> bool {
    let recorded = Store::load()
        .ok()
        .and_then(|store| store.get(&session.name).map(|s| s.state));
    if !matches!(recorded, Some(State::Creating | State::Seeding)) {
        return false;
    }
    // `Unknown` is a seeder that has not written its first line yet. A dead one
    // has nothing more to start, and the attach is the way to see what it left.
    matches!(
        seed::seed_state(backend, session),
        seed::SeedState::Unknown | seed::SeedState::Running { alive: true, .. }
    )
}

/// The blocking half: a pty, a child in it, and the two directions of traffic.
///
/// Its own thread rather than `spawn_blocking`, because it outlives a single
/// call and holds a reader that blocks until there is something to read.
fn pty_worker(
    session: Session,
    tmux: String,
    out: mpsc::Sender<Vec<u8>>,
    input: std::sync::mpsc::Receiver<ToChannel>,
) {
    use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

    // The backend decides what attaching *is*: an `sbx exec --interactive
    // --tty` into the sandbox's tmux, spawned under the pty below exactly as a
    // terminal emulator would.
    let backends = crate::rpc::backends();
    let backend = backends.for_session(&session);
    let Ok(argv) = ops::attach_argv(backend, &session, &tmux) else {
        return;
    };

    // A sandbox that cannot run the exec refuses it, and the refusal goes to
    // the pty, where it was drawn into the pane as though the agent had said
    // it before the terminal closed. `ops::create` does not leave a session in
    // that state, but the runtime can, and a tab is opened by clicking rather
    // than by creating.
    //
    // One `true` is the whole test: it is the same exec the attach is about to
    // do, so nothing can be ready for this and not for that. A sandbox that is
    // up answers in one round trip, which is what this costs in the normal case.
    let ready = || matches!(backend.exec(&session, &["true"]), Ok(out) if out.ok());
    let mut waited = Duration::ZERO;
    while !ready() {
        if waited.is_zero() {
            let _ = out.blocking_send(b"waiting for the sandbox...\r\n".to_vec());
        }
        if waited >= ATTACH_WAIT_LIMIT {
            let _ = out.blocking_send(
                b"the sandbox has not come up; close this tab and open it again\r\n".to_vec(),
            );
            return;
        }
        std::thread::sleep(ATTACH_WAIT_EVERY);
        waited += ATTACH_WAIT_EVERY;
    }

    // **The agent's own tmux session is the seeder's to create**, and a
    // reachable sandbox is not one that has got that far. The attach script
    // falls back to `new-session` when there is nothing to attach to, so a tab
    // opened mid-clone made the agent's session itself, as a bare shell -- and
    // the seeder's agent step, which leaves a session that already exists
    // alone, then never started the agent at all. A window that selects a
    // session the moment it is asked for opens exactly that tab.
    //
    // Waited out on the record as well as on the seeder: the record is a file
    // read, and once it says anything but creating or seeding there is nothing
    // left to wait for. Shells are not held back -- the seeder never makes one.
    if tmux == session.tmux {
        let mut waited = Duration::ZERO;
        while seeding(backend, &session) {
            if waited.is_zero() {
                let _ = out.blocking_send(b"waiting for the agent to start...\r\n".to_vec());
            }
            // The tab was closed. A clone can take minutes, and asking the
            // sandbox twice a second on behalf of nobody is a leak.
            if out.is_closed() {
                return;
            }
            if waited >= SEED_WAIT_LIMIT {
                let _ = out.blocking_send(
                    b"the session is still being prepared; close this tab and open it again\r\n"
                        .to_vec(),
                );
                return;
            }
            std::thread::sleep(ATTACH_WAIT_EVERY);
            waited += ATTACH_WAIT_EVERY;
        }
    }

    // A size to start with. The client sends its own as soon as it has one, and
    // until then this is what tmux draws for -- so it is the session's own
    // scrape size rather than an 80x24 the pane would have to reflow out of.
    let (cols, rows) = hura_core::session::SCRAPE_SIZE;
    let pty = NativePtySystem::default();
    let Ok(pair) = pty.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }) else {
        return;
    };

    let mut command = CommandBuilder::new(&argv[0]);
    command.args(&argv[1..]);
    // Said here rather than left to the image or the runtime, since tmux
    // needs to know it is talking to something. `ops::attach_script` sets the
    // locale inside; this is the outside half.
    command.env("TERM", "xterm-256color");
    // Else the tab stays open through the CLI's analytics wait after the
    // exec ends (see `NO_TELEMETRY`). Not for a worktree, whose attach can
    // start a tmux server that every later shell would inherit it from.
    if session.backend == hura_core::session::Kind::Sandbox {
        command.env(sbx_client::NO_TELEMETRY.0, sbx_client::NO_TELEMETRY.1);
    }

    let Ok(mut child) = pair.slave.spawn_command(command) else {
        return;
    };
    // The slave is the child's now. Holding a copy would keep the pty from ever
    // reporting end of file, so the reader below would block for ever.
    drop(pair.slave);

    let Ok(mut reader) = pair.master.try_clone_reader() else {
        return;
    };
    let Ok(mut writer) = pair.master.take_writer() else {
        return;
    };
    let master = std::sync::Arc::new(std::sync::Mutex::new(Some(pair.master)));

    // Waited on from the start, so the tab can close when the command ends.
    // On Unix the reader sees end of file then and nothing more is needed. A
    // Windows pseudo-console keeps its output open until it is closed itself,
    // so there the reader would block until the client left; closing it once
    // the child has exited is what ends the read.
    let waiting = {
        #[cfg(windows)]
        let master = master.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            #[cfg(windows)]
            if let Ok(mut master) = master.lock() {
                master.take();
            }
        })
    };

    let reading = std::thread::spawn(move || {
        let mut buf = vec![0u8; 8192];
        while let Ok(n) = std::io::Read::read(&mut reader, &mut buf) {
            if n == 0 || out.blocking_send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    while let Ok(message) = input.recv() {
        match message {
            ToChannel::Input(raw) => {
                if std::io::Write::write_all(&mut writer, &raw).is_err()
                    || std::io::Write::flush(&mut writer).is_err()
                {
                    break;
                }
            }
            // The client's window, as a real terminal's size change. tmux
            // resizes from it the way it would for any client.
            ToChannel::Resize { cols, rows } => {
                if let Ok(master) = master.lock()
                    && let Some(master) = master.as_ref()
                {
                    let _ = master.resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
            }
            ToChannel::Chat(_) => {}
        }
    }

    // The caller has gone. Hanging up the pty is what tells the child, and the
    // detach it was sent first is what lets it leave tmux cleanly.
    drop(writer);
    if let Ok(mut master) = master.lock() {
        master.take();
    }
    let _ = waiting.join();
    let _ = reading.join();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terminal channel leaves the way a person does rather than killing its
    /// exec, which wedged the whole sandbox under OpenShell. The detach
    /// sequence is what replaces the kill, and it is `Ctrl-b d`.
    #[test]
    fn the_detach_sequence_is_the_one_tmux_answers_to() {
        assert_eq!(DETACH, b"\x02d");
        assert_eq!(DETACH[0], 0x02, "Ctrl-b");
        assert_eq!(DETACH[1], b'd');
    }

    /// What a browser shows when a preview cannot reach its port: a whole
    /// response, whose length is the body's, saying why.
    #[test]
    fn an_unreachable_port_is_answered_in_words() {
        let raw = String::from_utf8(unreachable(8080, "connection refused")).unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 502 "), "{head}");
        assert!(
            head.contains(&format!("Content-Length: {}", body.len())),
            "{head}"
        );
        assert!(body.contains("port 8080") && body.contains("connection refused"));
    }

    #[test]
    fn a_plain_request_is_sent_to_the_same_place_over_tls() {
        let head =
            b"GET /app/index.html?x=1 HTTP/1.1\r\nhost: localhost:9001\r\nAccept: */*\r\n\r\n";
        assert_eq!(
            https_location(head).as_deref(),
            Some("https://localhost:9001/app/index.html?x=1")
        );
        let raw = String::from_utf8(redirect("https://localhost:9001/")).unwrap();
        assert!(raw.starts_with("HTTP/1.1 307 "), "{raw}");
        assert!(
            raw.contains("\r\nLocation: https://localhost:9001/\r\n"),
            "{raw}"
        );
    }

    #[test]
    fn only_an_http_request_with_a_sane_host_is_redirected() {
        for head in [
            &b"\x16\x03\x01\x00"[..],
            b"SSH-2.0-OpenSSH_9.6\r\n",
            b"GET / HTTP/1.1\r\n\r\n",
            b"GET http://elsewhere/ HTTP/1.1\r\nHost: localhost\r\n\r\n",
            b"GET / HTTP/1.1\r\nHost: evil.example/\r\n\r\n",
            b"GET / HTTP/1.1\r\n\r\nHost: localhost:1\r\n",
        ] {
            assert_eq!(
                https_location(head),
                None,
                "{}",
                String::from_utf8_lossy(head)
            );
        }
    }

    /// Against a real TLS server, and against one that closes a connection
    /// it does not understand, as a plain HTTP server given a `ClientHello`
    /// may.
    #[tokio::test]
    async fn a_tls_server_is_told_from_one_that_is_not() {
        use std::io::Read as _;

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into()),
        )
        .unwrap();
        let config = std::sync::Arc::new(config);
        let tls = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let tls_port = tls.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut sock, _) = tls.accept().unwrap();
            let mut conn = rustls::ServerConnection::new(config).unwrap();
            let _ = conn.complete_io(&mut sock);
        });
        let plain = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let plain_port = plain.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut sock, _) = plain.accept().unwrap();
            let _ = sock.read(&mut [0u8; 64]);
        });

        let at = |port| tokio::net::TcpStream::connect(("127.0.0.1", port));
        assert!(speaks_tls(at(tls_port)).await);
        assert!(!speaks_tls(at(plain_port)).await);
    }
}
