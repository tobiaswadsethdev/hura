//! Tests that need a paired server and a real session.
//!
//! `#[ignore]`d, like the gateway contract tests in `openshell-client`: the
//! suite stays hermetic for anyone without a gateway, and these are run by hand
//! when the streaming half changes.
//!
//! ```sh
//! hura new --repo <url> --task "..." --name live-stream
//! cargo test -p hura-client -- --ignored --test-threads=1
//! ```
//!
//! The session name is `live-stream` unless `HURA_LIVE_SESSION` says otherwise,
//! and the server is whichever one is paired.

use std::time::{Duration, Instant};

use hura_client::{Incoming, Remotes};
use hura_proto::stream::{Channel, ClientFrame, ServerFrame, bytes};

fn session() -> String {
    std::env::var("HURA_LIVE_SESSION").unwrap_or_else(|_| "live-stream".into())
}

fn stream() -> hura_client::Stream {
    let remotes = Remotes::load().expect("remotes");
    let remote = remotes.select(None).expect("one paired server");
    remote.stream().expect("the websocket opened")
}

/// Wait for a frame the predicate accepts, or give up.
fn wait_for<T>(
    stream: &hura_client::Stream,
    within: Duration,
    mut f: impl FnMut(&ServerFrame) -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        match stream.frames().recv_timeout(Duration::from_millis(500)) {
            Ok(Incoming::Frame(frame)) => {
                if let Some(v) = f(&frame) {
                    return Some(v);
                }
            }
            Ok(Incoming::Ended(reason)) => panic!("the connection ended: {reason:?}"),
            Err(_) => continue,
        }
    }
    None
}

/// The channel this whole increment is for: bytes out of the agent's tmux.
///
/// The pty is at the sandbox end, so nothing on this side needs a terminal --
/// which is the property that lets a server host it at all, and the one worth
/// checking against a real gateway rather than assuming.
#[test]
#[ignore = "needs a paired server and a live session"]
fn a_terminal_channel_produces_the_agents_screen() {
    let stream = stream();
    stream.send(ClientFrame::Open {
        id: 1,
        channel: Channel::Terminal {
            session: session(),
            tmux: None,
        },
    });
    stream.send(ClientFrame::Resize {
        id: 1,
        cols: 100,
        rows: 30,
    });

    let opened = wait_for(&stream, Duration::from_secs(10), |f| {
        matches!(f, ServerFrame::Opened { id: 1 }).then_some(())
    });
    assert!(opened.is_some(), "the channel never opened");

    let output = wait_for(&stream, Duration::from_secs(20), |f| match f {
        ServerFrame::Output { data, .. } => bytes::decode(data),
        ServerFrame::Closed { reason, .. } => panic!("the terminal closed: {reason:?}"),
        _ => None,
    })
    .expect("no output from the terminal");

    assert!(!output.is_empty());
    // tmux redraws on attach, so the first thing through is escape sequences.
    // Asserting on the agent's own text would be asserting on Claude Code's
    // banner, which is not a contract.
    assert!(
        output.contains(&0x1b) || output.iter().any(|b| b.is_ascii_graphic()),
        "output carried neither escapes nor text: {output:?}"
    );
}

/// Closing a terminal must detach, not kill. A killed `exec --tty` wedges the
/// exec path for the sandbox, so the check is that the *next* channel still
/// works -- which it cannot if the first one broke the path.
#[test]
#[ignore = "needs a paired server and a live session"]
fn a_terminal_can_be_opened_again_after_being_closed() {
    for attempt in 1..=2 {
        let stream = stream();
        stream.send(ClientFrame::Open {
            id: 1,
            channel: Channel::Terminal {
                session: session(),
                tmux: None,
            },
        });

        let got = wait_for(&stream, Duration::from_secs(20), |f| match f {
            ServerFrame::Output { data, .. } => bytes::decode(data),
            _ => None,
        });
        assert!(got.is_some(), "attempt {attempt} produced no output");

        stream.send(ClientFrame::Close { id: 1 });
        drop(stream);
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// The feed sends the recent log once and then only what is new. A second
/// batch repeating the first would fill a pane with duplicates within a minute.
#[test]
#[ignore = "needs a paired server and a live session"]
fn the_events_channel_does_not_repeat_itself() {
    let stream = stream();
    stream.send(ClientFrame::Open {
        id: 7,
        channel: Channel::Events { session: session() },
    });

    let mut keys: Vec<(u64, String, String)> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if let Ok(Incoming::Frame(frame)) = stream.frames().recv_timeout(Duration::from_millis(500))
            && let ServerFrame::Events { events, .. } = *frame
        {
            for e in events {
                let key = e.key();
                assert!(!keys.contains(&key), "sent twice: {key:?}");
                keys.push(key);
            }
        }
    }
    assert!(!keys.is_empty(), "the feed said nothing at all");
}

/// A preview's channel: one TCP connection to a port inside the sandbox,
/// through the server's forward, as `Input` and `Output` frames.
///
/// Needs something listening in the session's sandbox -- `python3 -m
/// http.server 8000 --bind 127.0.0.1` is enough -- and `HURA_LIVE_PORT` if it
/// is not 8000. Two connections, one after the other, so the second one
/// reuses the forward the first started.
#[test]
#[ignore = "needs a paired server and a live session with a port open"]
fn a_port_channel_reaches_a_service_in_the_sandbox() {
    let port: u16 = std::env::var("HURA_LIVE_PORT").map_or(8000, |p| p.parse().unwrap());
    let stream = stream();

    for id in [41, 42] {
        stream.send(ClientFrame::Open {
            id,
            channel: Channel::Port {
                session: session(),
                port,
                host: Default::default(),
            },
        });
        stream.send(ClientFrame::Input {
            id,
            data: bytes::encode(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"),
        });

        let mut reply = Vec::new();
        let got = wait_for(&stream, Duration::from_secs(30), |frame| match frame {
            ServerFrame::Output { id: i, data } if *i == id => {
                reply.extend(bytes::decode(data).unwrap());
                String::from_utf8_lossy(&reply)
                    .starts_with("HTTP/1.")
                    .then(|| String::from_utf8_lossy(&reply).into_owned())
            }
            ServerFrame::Closed { id: i, reason } if *i == id => {
                panic!("the channel closed before answering: {reason:?}")
            }
            _ => None,
        });
        let got = got.expect("an HTTP reply within thirty seconds");
        assert!(got.starts_with("HTTP/1."), "{got}");
        stream.send(ClientFrame::Close { id });
    }
}

/// The ports pane's three requests against a real sandbox: who holds a port,
/// a forward that shows its connection, stopping it, and killing the holder.
///
/// Destructive: the process listening on `HURA_LIVE_PORT` is killed at the
/// end. Run it against a throwaway server, such as `python3 -m http.server`.
#[test]
#[ignore = "needs a paired server and a live session with a port open; kills the process"]
fn a_port_can_be_seen_stopped_and_killed() {
    use hura_proto::{Reply, Request};

    let port: u16 = std::env::var("HURA_LIVE_PORT").map_or(8000, |p| p.parse().unwrap());
    let remotes = Remotes::load().expect("remotes");
    let remote = remotes.select(None).expect("one paired server");
    let ports = |request: Request| match remote.call(request).expect("a reply") {
        Reply::Ports(view) => view,
        other => panic!("not a ports reply: {other:?}"),
    };

    let view = ports(Request::Ports { name: session() });
    let listener = view
        .listening
        .iter()
        .find(|l| l.port == port)
        .expect("the port is listed");
    let owner = listener.owner.as_ref().expect("its owner is visible");
    assert!(
        owner.command.contains("python") || !owner.command.is_empty(),
        "{owner:?}"
    );

    // One connection held open through the forward.
    let stream = stream();
    stream.send(ClientFrame::Open {
        id: 51,
        channel: Channel::Port {
            session: session(),
            port,
            host: Default::default(),
        },
    });
    stream.send(ClientFrame::Input {
        id: 51,
        data: bytes::encode(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n"),
    });
    wait_for(&stream, Duration::from_secs(30), |f| {
        matches!(f, ServerFrame::Output { id: 51, .. }).then_some(())
    })
    .expect("an answer through the forward");

    let view = ports(Request::Ports { name: session() });
    let forward = view
        .forwards
        .iter()
        .find(|f| f.port == port)
        .expect("the forward is listed");
    assert_eq!(forward.connections, 1, "{forward:?}");

    // Stopping it ends the connection that was holding it open.
    let view = ports(Request::StopForward {
        name: session(),
        port,
    });
    assert!(
        view.forwards.iter().all(|f| f.port != port),
        "{:?}",
        view.forwards
    );
    wait_for(&stream, Duration::from_secs(10), |f| {
        matches!(f, ServerFrame::Closed { id: 51, .. }).then_some(())
    })
    .expect("the held connection closed with the forward");

    // And killing the holder takes the port away.
    let view = ports(Request::KillPort {
        name: session(),
        port,
    });
    assert!(
        view.listening.iter().all(|l| l.port != port),
        "{:?}",
        view.listening
    );
}
