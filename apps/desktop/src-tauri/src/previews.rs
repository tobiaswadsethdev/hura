//! Previews: a port inside a sandbox, on this machine's loopback.
//!
//! For each preview this binds a port on `127.0.0.1` and turns every
//! connection it accepts into a port channel on the window's one streaming
//! connection -- see `Channel::Port` in `hura_proto::stream`. `hurad` does the
//! other half: one `openshell forward service` per port, shared by every
//! connection to it.
//!
//! **Through the paired connection, not around it.** The alternative was
//! binding the forward on the server and dialling it directly, which only
//! works where that port is reachable -- not across a VPN, and not into WSL on
//! NAT networking -- and puts an unauthenticated dev server on whatever
//! interface it binds. This way it is reachable from this machine alone, over
//! a connection that is already pinned and authenticated.
//!
//! The same port number is asked for first, so `localhost:5173` in the window
//! is `localhost:5173` in the sandbox and a dev server's own idea of its URL --
//! in a redirect, in a CORS rule, in an OAuth callback -- is right. Another
//! number is taken only when this machine already uses that one.
//!
//! Plain threads, like the rest of this side of the bridge: a preview is a
//! listener and two threads per connection, and a browser opens a handful of
//! connections per page.

use std::collections::{HashMap, HashSet};
use std::io::{ErrorKind, Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hura_core::ports::{Loopback, PortsView};
use hura_proto::stream::{Channel, ChannelId, ClientFrame, ServerFrame, bytes};
use hura_proto::{Reply, Request};
use serde::Serialize;
use tauri::Manager as _;

use crate::{Failed, failed};

/// Where this side's channel ids start. The webview counts up from 1 for its
/// panes, so the two never meet; `ChannelId::MAX` is the connection's own.
const FIRST_ID: ChannelId = 0x8000_0000;

/// The connection a tunnel's frames travel on. The window's own in the
/// application; a bare `hura_client` stream in the test below, which is what
/// lets this file be tested against a real server without a window.
trait Link: Send + Sync + 'static {
    fn open(&self, server: &str) -> Result<(), Failed>;
    fn send(&self, frame: ClientFrame) -> Result<(), Failed>;
    fn previews(&self) -> &Previews;
}

/// The application's link: the window's streaming connection.
struct Window(tauri::AppHandle);

impl Link for Window {
    fn open(&self, server: &str) -> Result<(), Failed> {
        crate::connected(&self.0, server)
    }
    fn send(&self, frame: ClientFrame) -> Result<(), Failed> {
        crate::send(&self.0, frame)
    }
    fn previews(&self) -> &Previews {
        self.0.state::<Previews>().inner()
    }
}

/// What a tunnel's writer is told.
enum Inbound {
    Data(Vec<u8>),
    Closed,
}

#[derive(Default)]
pub struct Previews {
    next: AtomicU32,
    /// One entry per live connection, by the channel id that carries it.
    tunnels: Mutex<HashMap<ChannelId, Sender<Inbound>>>,
    open: Mutex<Vec<Preview>>,
}

struct Preview {
    server: String,
    session: String,
    port: u16,
    local: u16,
    stop: Arc<AtomicBool>,
    /// The channels carrying this preview's connections right now, so that
    /// stopping it ends them too. Without this, stopping a preview only
    /// stopped it *accepting*: a tab with a live-reload socket open kept
    /// talking to the sandbox for as long as it stayed open.
    live: Live,
}

type Live = Arc<Mutex<HashSet<ChannelId>>>;

/// A preview, as the window shows it.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewView {
    session: String,
    port: u16,
    local: u16,
    url: String,
    /// Connections open through it from this machine.
    connections: usize,
}

impl Preview {
    fn view(&self) -> PreviewView {
        PreviewView {
            session: self.session.clone(),
            port: self.port,
            local: self.local,
            url: format!("http://localhost:{}/", self.local),
            connections: self.live.lock().map_or(0, |l| l.len()),
        }
    }
}

impl Previews {
    fn id(&self) -> ChannelId {
        FIRST_ID + self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// Whether a frame belongs to a tunnel, and if so, hand it over. Called by
    /// the thread reading the connection, before anything goes to the window.
    pub fn route(&self, frame: &ServerFrame) -> bool {
        let id = frame.id();
        if !(FIRST_ID..ChannelId::MAX).contains(&id) {
            return false;
        }
        let Ok(mut tunnels) = self.tunnels.lock() else {
            return true;
        };
        match frame {
            ServerFrame::Output { data, .. } => {
                if let (Some(tx), Some(raw)) = (tunnels.get(&id), bytes::decode(data)) {
                    let _ = tx.send(Inbound::Data(raw));
                }
            }
            ServerFrame::Closed { .. } => {
                if let Some(tx) = tunnels.remove(&id) {
                    let _ = tx.send(Inbound::Closed);
                }
            }
            _ => {}
        }
        true
    }

    /// The connection has gone, and every connection through it with it. The
    /// listeners stay: the next connection accepted reconnects.
    pub fn connection_ended(&self) {
        if let Ok(mut tunnels) = self.tunnels.lock() {
            for (_, tx) in tunnels.drain() {
                let _ = tx.send(Inbound::Closed);
            }
        }
    }

    /// The window has switched to another server, so previews of this one's
    /// sessions would be sent down a connection to the wrong machine.
    pub fn keep_only(&self, server: &str) {
        if let Ok(mut open) = self.open.lock() {
            open.retain(|p| {
                let keep = p.server == server;
                if !keep {
                    p.stop.store(true, Ordering::Relaxed);
                }
                keep
            });
        }
    }

    /// Stop one preview: no more connections accepted, and every open one
    /// ended. The frames telling the server go through `send`; the server's
    /// half of each channel ends with them.
    fn close(&self, server: &str, session: &str, port: u16, send: impl Fn(ClientFrame)) {
        let removed: Vec<Preview> = match self.open.lock() {
            Ok(mut open) => {
                let (gone, kept) = std::mem::take(&mut *open)
                    .into_iter()
                    .partition(|p| p.server == server && p.session == session && p.port == port);
                *open = kept;
                gone
            }
            Err(_) => return,
        };
        for preview in removed {
            preview.stop.store(true, Ordering::Relaxed);
            let ids: Vec<ChannelId> = preview
                .live
                .lock()
                .map(|mut l| l.drain().collect())
                .unwrap_or_default();
            for id in ids {
                send(ClientFrame::Close { id });
                if let Ok(mut tunnels) = self.tunnels.lock()
                    && let Some(tx) = tunnels.remove(&id)
                {
                    let _ = tx.send(Inbound::Closed);
                }
            }
        }
    }
}

#[tauri::command(async)]
pub fn previews(app: tauri::AppHandle, server: String) -> Result<Vec<PreviewView>, Failed> {
    let state = app.state::<Previews>();
    let open = state.open.lock().map_err(failed)?;
    Ok(open
        .iter()
        .filter(|p| p.server == server)
        .map(Preview::view)
        .collect())
}

/// Start previewing a port, or answer with the preview already running.
#[tauri::command(async)]
pub fn preview_open(
    app: tauri::AppHandle,
    server: String,
    session: String,
    port: u16,
    host: Loopback,
) -> Result<PreviewView, Failed> {
    let state = app.state::<Previews>();
    let mut open = state.open.lock().map_err(failed)?;
    if let Some(p) = open
        .iter()
        .find(|p| p.server == server && p.session == session && p.port == port)
    {
        return Ok(p.view());
    }

    let listener = TcpListener::bind(("127.0.0.1", port))
        .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
        .map_err(|e| failed(format!("could not listen on this machine: {e}")))?;
    let local = listener.local_addr().map_err(failed)?.port();
    // Non-blocking, so the accept loop can notice it has been stopped: a
    // blocking `accept` would hold the port until somebody connected to it.
    listener.set_nonblocking(true).map_err(failed)?;

    let preview = Preview {
        server,
        session,
        port,
        local,
        stop: Arc::new(AtomicBool::new(false)),
        live: Live::default(),
    };
    let view = preview.view();
    let target = Target {
        server: preview.server.clone(),
        session: preview.session.clone(),
        port,
        host,
        live: preview.live.clone(),
    };
    let stop = preview.stop.clone();
    open.push(preview);
    drop(open);

    let link: Arc<dyn Link> = Arc::new(Window(app));
    std::thread::spawn(move || accept(link, listener, target, stop));
    Ok(view)
}

/// Stop a preview on this machine, and the server's forward behind it.
///
/// Both, because "stop" in the pane means the thing is no longer reachable --
/// and the forward would otherwise sit on the server for its idle minutes,
/// still serving any other client that had it open.
#[tauri::command(async)]
pub fn preview_stop(
    app: tauri::AppHandle,
    server: String,
    session: String,
    port: u16,
) -> Result<PortsView, Failed> {
    close_here(&app, &server, &session, port);
    ask(
        &server,
        Request::StopForward {
            name: session,
            port,
        },
    )
}

/// Stop the process listening on the port inside the sandbox, and every
/// preview of it.
#[tauri::command(async)]
pub fn kill_port(
    app: tauri::AppHandle,
    server: String,
    session: String,
    port: u16,
) -> Result<PortsView, Failed> {
    close_here(&app, &server, &session, port);
    ask(
        &server,
        Request::KillPort {
            name: session,
            port,
        },
    )
}

/// Stop one process in the sandbox, by pid: for a dev server whose port
/// cannot be traced to it, which inside the sandbox is most of them.
#[tauri::command(async)]
pub fn kill_process(server: String, session: String, pid: u32) -> Result<PortsView, Failed> {
    ask(&server, Request::KillProcess { name: session, pid })
}

/// A session's ports as the server sees them: what is listening and who
/// holds it, and the forwards it is running.
#[tauri::command(async)]
pub fn ports(server: String, session: String) -> Result<PortsView, Failed> {
    ask(&server, Request::Ports { name: session })
}

fn close_here(app: &tauri::AppHandle, server: &str, session: &str, port: u16) {
    app.state::<Previews>()
        .close(server, session, port, |frame| {
            let _ = crate::send(app, frame);
        });
}

fn ask(server: &str, request: Request) -> Result<PortsView, Failed> {
    let reply = crate::remote(server)?
        .call(request)
        .map_err(crate::to_message)?;
    match reply {
        Reply::Ports(view) => Ok(view),
        _ => Err(failed("the server answered something other than its ports")),
    }
}

/// Where a preview's connections go.
#[derive(Clone)]
struct Target {
    server: String,
    session: String,
    port: u16,
    host: Loopback,
    live: Live,
}

fn accept(link: Arc<dyn Link>, listener: TcpListener, target: Target, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let link = link.clone();
                let target = target.clone();
                std::thread::spawn(move || tunnel(&*link, stream, target));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// One accepted connection, as one channel, until either end closes.
fn tunnel(link: &dyn Link, stream: TcpStream, target: Target) {
    // The listener is non-blocking and an accepted socket inherits that on
    // some platforms; the two threads below want to block.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_nodelay(true);
    let state = link.previews();
    let id = state.id();

    let (tx, rx) = channel::<Inbound>();
    if let Ok(mut tunnels) = state.tunnels.lock() {
        tunnels.insert(id, tx);
    }
    if let Ok(mut live) = target.live.lock() {
        live.insert(id);
    }

    let opened = link.open(&target.server).and_then(|()| {
        link.send(ClientFrame::Open {
            id,
            channel: Channel::Port {
                session: target.session.clone(),
                port: target.port,
                host: target.host,
            },
        })
    });
    if opened.is_err() {
        if let Ok(mut tunnels) = state.tunnels.lock() {
            tunnels.remove(&id);
        }
        if let Ok(mut live) = target.live.lock() {
            live.remove(&id);
        }
        let _ = stream.shutdown(Shutdown::Both);
        return;
    }

    // The service's answers, written back to whoever connected.
    let Ok(mut writer) = stream.try_clone() else {
        let _ = link.send(ClientFrame::Close { id });
        return;
    };
    let write = std::thread::spawn(move || {
        while let Ok(Inbound::Data(raw)) = rx.recv() {
            if writer.write_all(&raw).is_err() {
                break;
            }
        }
        let _ = writer.shutdown(Shutdown::Both);
    });

    // What they send, to the service.
    let mut reader = stream;
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let frame = ClientFrame::Input {
                    id,
                    data: bytes::encode(&buf[..n]),
                };
                if link.send(frame).is_err() {
                    break;
                }
            }
        }
    }

    let _ = link.send(ClientFrame::Close { id });
    if let Ok(mut tunnels) = state.tunnels.lock()
        && let Some(tx) = tunnels.remove(&id)
    {
        let _ = tx.send(Inbound::Closed);
    }
    if let Ok(mut live) = target.live.lock() {
        live.remove(&id);
    }
    let _ = write.join();
}

#[cfg(test)]
mod tests {
    use super::*;
    use hura_client::{Incoming, Remotes, Sink};

    /// A link straight to a paired server, with its frames routed the way the
    /// application's reader thread routes them.
    struct Direct {
        sink: Mutex<Sink>,
        previews: Arc<Previews>,
    }

    impl Link for Direct {
        fn open(&self, _: &str) -> Result<(), Failed> {
            Ok(())
        }
        fn send(&self, frame: ClientFrame) -> Result<(), Failed> {
            let sink = self.sink.lock().map_err(failed)?;
            sink.send(frame)
                .then_some(())
                .ok_or_else(|| failed("ended"))
        }
        fn previews(&self) -> &Previews {
            &self.previews
        }
    }

    /// The whole of this file against a real server: a browser-shaped client
    /// connects to the local port, several times at once, and each gets the
    /// sandbox's answer.
    ///
    /// ```text
    /// HURA_LIVE_SESSION=<session> HURA_LIVE_PORT=8000 \
    ///   cargo test -- --ignored a_preview
    /// ```
    #[test]
    #[ignore = "needs a paired server and a live session with a port open"]
    fn a_preview_carries_concurrent_connections() {
        let session = std::env::var("HURA_LIVE_SESSION").expect("HURA_LIVE_SESSION");
        let port: u16 = std::env::var("HURA_LIVE_PORT").map_or(8000, |p| p.parse().unwrap());
        let remotes = Remotes::load().unwrap();
        let (sink, frames) = remotes.select(None).unwrap().stream().unwrap().split();

        let previews = Arc::new(Previews::default());
        let routing = previews.clone();
        std::thread::spawn(move || {
            for message in frames {
                match message {
                    Incoming::Frame(frame) => {
                        routing.route(&frame);
                    }
                    Incoming::Ended(_) => {
                        routing.connection_ended();
                        break;
                    }
                }
            }
        });
        let link: Arc<dyn Link> = Arc::new(Direct {
            sink: Mutex::new(sink),
            previews,
        });

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let live = Live::default();
        link.previews().open.lock().unwrap().push(Preview {
            server: "live".into(),
            session: session.clone(),
            port,
            local,
            stop: stop.clone(),
            live: live.clone(),
        });
        let target = Target {
            server: "live".into(),
            session: session.clone(),
            port,
            host: Loopback::V4,
            live: live.clone(),
        };
        {
            let (link, stop) = (link.clone(), stop.clone());
            std::thread::spawn(move || accept(link, listener, target, stop));
        }

        let clients: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(move || {
                    let mut c = TcpStream::connect(("127.0.0.1", local)).unwrap();
                    c.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
                    c.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
                        .unwrap();
                    let mut buf = vec![0u8; 4096];
                    let n = c.read(&mut buf).unwrap();
                    String::from_utf8_lossy(&buf[..n]).into_owned()
                })
            })
            .collect();
        for client in clients {
            let reply = client.join().unwrap();
            assert!(reply.starts_with("HTTP/1."), "{reply}");
        }

        // A connection held open -- a live-reload socket, a keep-alive tab --
        // is ended by stopping the preview, not left talking to the sandbox.
        let mut held = TcpStream::connect(("127.0.0.1", local)).unwrap();
        held.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        held.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut buf = vec![0u8; 4096];
        assert!(held.read(&mut buf).unwrap() > 0, "an answer first");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            live.lock().unwrap().len(),
            1,
            "the held connection is tracked"
        );

        let sender = link.clone();
        link.previews().close("live", &session, port, |frame| {
            let _ = sender.send(frame);
        });
        let after = loop {
            match held.read(&mut buf) {
                Ok(0) => break Ok(()),
                Ok(_) => continue,
                Err(e) => break Err(e),
            }
        };
        assert!(
            after.is_ok(),
            "the held connection was closed, not timed out: {after:?}"
        );
        assert!(
            stop.load(Ordering::Relaxed),
            "and the listener was told to stop"
        );
        assert!(link.previews().open.lock().unwrap().is_empty());
    }
}
