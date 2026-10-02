//! Previews: a port inside a sandbox, reachable from a client's loopback.
//!
//! The client listens and turns each connection it accepts into a port
//! channel -- see [`hura_proto::stream::Channel::Port`]. This end needs a way
//! into the sandbox for each of them, and that is `openshell forward service`:
//! it binds a loopback port here, travels over the gateway's gRPC path, and
//! serves any number of connections until it is killed.
//!
//! **One forward per port, shared.** A browser opens half a dozen connections
//! to load one page, and a process per connection would be a second of startup
//! in front of every one of them. So a forward is started by the first
//! connection to a port, leased by every connection after it, and stopped once
//! nothing has used it for [`IDLE`].
//!
//! Killing one is safe, unlike an `exec --tty`: the forward is its own gRPC
//! stream and not the exec path, so a stopped preview does not wedge the
//! sandbox for the polls and terminals that come after it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use hura_core::ports::Loopback;
use hura_core::session::Session;
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex as AsyncMutex;

/// How long a forward with no connections is kept for the next one.
///
/// Long enough that a page reloaded after a minute's reading does not pay the
/// startup again; short enough that a sandbox somebody has stopped previewing
/// is not holding a process here for the rest of the day.
const IDLE: Duration = Duration::from_secs(120);

/// How long a forward may take to say which port it bound.
const START: Duration = Duration::from_secs(20);

/// What a forward runs under, so that it cannot outlive this process.
///
/// The registry is a static, and statics are never dropped, so `kill_on_drop`
/// alone left every forward running after `hurad` exited -- measured: one
/// orphan per port after each restart, and `hurad` restarts itself to update.
/// A pipe is the one thing that closes however a process ends, `SIGKILL`
/// included, so the forward runs beside a reader holding this process's end
/// of one: when the read returns, the forward is killed. Closing the pipe is
/// also how [`reap`] stops one.
///
/// The pipe is moved to fd 3 *first*, because a background job in a
/// non-interactive shell has its standard input replaced with `/dev/null` --
/// before its own redirections, so a `3<&0` on the job itself duplicates
/// `/dev/null`, the read returns at once, and the forward is killed as it
/// starts. Measured, which is how this comment came to be.
/// The shell waits on the forward rather than on the reader, so a forward that
/// dies on its own is a process that has exited, which is what [`acquire`]
/// checks for.
const GUARD: &str = r#"exec 3<&0
"$@" 3<&- & c=$!
( read -r _ <&3; kill "$c" 2>/dev/null ) & r=$!
exec 3<&-
wait "$c"; s=$?
kill "$r" 2>/dev/null
exit "$s""#;

type Key = (String, u16, Loopback);

/// One forward, while it is running.
struct Running {
    local: u16,
    child: Child,
    /// This process's end of the guard's pipe. Dropping it stops the forward.
    lifeline: Option<ChildStdin>,
    users: usize,
    idle_since: Instant,
}

/// A slot per key, each with its own lock, so that starting a forward to one
/// port -- a second or so -- does not hold up a connection to another.
type Slot = Arc<AsyncMutex<Option<Running>>>;

fn slots() -> &'static Mutex<HashMap<Key, Slot>> {
    static SLOTS: OnceLock<Mutex<HashMap<Key, Slot>>> = OnceLock::new();
    SLOTS.get_or_init(|| {
        tokio::spawn(reap());
        Mutex::default()
    })
}

/// A connection's claim on a forward. The forward stays up while one exists.
pub struct Lease {
    pub local: u16,
    slot: Slot,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let slot = self.slot.clone();
        // Dropped from inside a task that may be being aborted, so the
        // bookkeeping goes on a task of its own rather than awaiting here.
        tokio::spawn(async move {
            if let Some(running) = slot.lock().await.as_mut() {
                running.users = running.users.saturating_sub(1);
                if running.users == 0 {
                    running.idle_since = Instant::now();
                }
            }
        });
    }
}

/// A local port that reaches `port` in the session's sandbox, started if
/// nothing is forwarding there yet.
pub async fn acquire(session: &Session, port: u16, host: Loopback) -> Result<Lease, String> {
    let key = (session.sandbox.clone(), port, host);
    let slot = slots()
        .lock()
        .map_err(|_| "the forward registry is poisoned".to_string())?
        .entry(key)
        .or_default()
        .clone();

    let mut guard = slot.lock().await;
    // A forward whose process has gone -- the sandbox was recreated, the
    // gateway restarted -- is started again rather than handed out.
    let alive = match guard.as_mut() {
        Some(running) => matches!(running.child.try_wait(), Ok(None)),
        None => false,
    };
    if !alive {
        *guard = Some(start(session, port, host).await?);
    }
    let running = guard.as_mut().expect("just started");
    running.users += 1;
    let local = running.local;
    drop(guard);
    Ok(Lease { local, slot })
}

async fn start(session: &Session, port: u16, host: Loopback) -> Result<Running, String> {
    let argv = {
        let backends = crate::rpc::backends();
        backends
            .for_session(session)
            .forward_argv(session, port, host)
            .map_err(|e| e.to_string())?
    };
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(GUARD)
        .arg("sh")
        .args(&argv)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("could not start a forward: {e}"))?;
    let lifeline = child.stdin.take();

    // The address arrives on one of the two streams and the CLI has moved its
    // messages between them before, so both are read. They keep being read
    // after it has arrived: a warning per refused connection goes there, and a
    // pipe nobody drains is a forward that stops once it fills.
    let (lines, mut said) = tokio::sync::mpsc::unbounded_channel::<String>();
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let lines = lines.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = lines.send(line);
            }
        });
    }
    drop(lines);

    let mut heard = Vec::new();
    let found = tokio::time::timeout(START, async {
        while let Some(line) = said.recv().await {
            if let Some(local) = bound_port(&line) {
                return Some(local);
            }
            heard.push(hura_core::ansi::strip(&line).trim().to_string());
        }
        None
    })
    .await;

    match found {
        Ok(Some(local)) => {
            // The rest of what it says is noise from here; drained so the
            // reader tasks above never block on a full channel.
            tokio::spawn(async move { while said.recv().await.is_some() {} });
            Ok(Running {
                local,
                child,
                lifeline,
                users: 0,
                idle_since: Instant::now(),
            })
        }
        Ok(None) => Err(match heard.iter().rev().find(|l| !l.is_empty()) {
            Some(last) => format!("the forward to port {port} stopped: {last}"),
            None => format!("the forward to port {port} stopped without saying why"),
        }),
        Err(_) => Err(format!(
            "the forward to port {port} did not start within {}s",
            START.as_secs()
        )),
    }
}

/// The forwards running into one sandbox, for the ports pane.
///
/// Called from a request handler, which runs on a blocking thread, so the
/// slots are tried rather than awaited: one that is busy is a forward being
/// started this second, and the next look will show it.
pub fn list(sandbox: &str) -> Vec<hura_core::ports::Forward> {
    let Ok(map) = slots().lock() else {
        return Vec::new();
    };
    let mut out: Vec<_> = map
        .iter()
        .filter(|((s, _, _), _)| s == sandbox)
        .filter_map(|((_, port, host), slot)| {
            let mut guard = slot.try_lock().ok()?;
            let running = guard.as_mut()?;
            if !matches!(running.child.try_wait(), Ok(None)) {
                return None;
            }
            Some(hura_core::ports::Forward {
                port: *port,
                host: *host,
                connections: running.users as u32,
                idle_secs: (running.users == 0).then(|| running.idle_since.elapsed().as_secs()),
            })
        })
        .collect();
    out.sort_by_key(|f| f.port);
    out
}

/// Stop every forward to `port` in a sandbox now, rather than when it has
/// been idle long enough. Every connection through it ends with it, which is
/// the point: a browser's live-reload socket would otherwise keep it in use
/// for as long as the tab is open.
pub fn stop_now(sandbox: &str, port: u16) {
    let Ok(map) = slots().lock() else {
        return;
    };
    let matching: Vec<Slot> = map
        .iter()
        .filter(|((s, p, _), _)| s == sandbox && *p == port)
        .map(|(_, slot)| slot.clone())
        .collect();
    drop(map);
    for slot in matching {
        // A forward still starting holds its slot for a second or so.
        for _ in 0..50 {
            if let Ok(mut guard) = slot.try_lock() {
                if let Some(mut running) = guard.take() {
                    drop(running.lifeline.take());
                    let _ = running.child.start_kill();
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// End a forward: close the pipe, which is what the guard waits for, and
/// kill the guard only if it has not gone a few seconds later.
async fn stop(mut running: Running) {
    drop(running.lifeline.take());
    if tokio::time::timeout(Duration::from_secs(5), running.child.wait())
        .await
        .is_err()
    {
        let _ = running.child.kill().await;
    }
}

/// The port out of `✓ Forwarding 127.0.0.1:38076 -> 127.0.0.1:8000 in ...`.
fn bound_port(line: &str) -> Option<u16> {
    let plain = hura_core::ansi::strip(line);
    let rest = plain.split("Forwarding 127.0.0.1:").nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|p| *p != 0)
}

/// Stop the forwards nobody has used for a while.
async fn reap() {
    loop {
        tokio::time::sleep(IDLE / 4).await;
        let all: Vec<Slot> = match slots().lock() {
            Ok(map) => map.values().cloned().collect(),
            Err(_) => return,
        };
        for slot in all {
            // Busy means starting or in use: either way, not idle.
            let Ok(mut guard) = slot.try_lock() else {
                continue;
            };
            let stale = guard
                .as_ref()
                .is_some_and(|r| r.users == 0 && r.idle_since.elapsed() >= IDLE);
            if stale && let Some(running) = guard.take() {
                stop(running).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from `openshell forward service` 0.0.110, colour and all.
    #[test]
    fn the_bound_port_is_read_from_what_the_cli_says() {
        let line = "\x1b[1m\x1b[32m✓\x1b[39m\x1b[0m Forwarding 127.0.0.1:38076 -> 127.0.0.1:8000 in sandbox fwd-probe via gRPC";
        assert_eq!(bound_port(line), Some(38076));
        let v6 = "✓ Forwarding 127.0.0.1:38608 -> ::1:8001 in sandbox s via gRPC";
        assert_eq!(bound_port(v6), Some(38608));
    }

    #[test]
    fn anything_else_is_not_a_port() {
        for line in [
            "",
            "WARN service forward connection failed",
            "Forwarding 127.0.0.1:0 -> 127.0.0.1:8000",
            "Forwarding 127.0.0.1:abc",
        ] {
            assert_eq!(bound_port(line), None, "{line}");
        }
    }

    const _: () = assert!(
        IDLE.as_secs() >= 30,
        "a reload after reading should not restart it"
    );

    /// Against a real gateway: a sandbox with something listening, reached
    /// through a forward, by several connections at once that share it.
    ///
    /// ```text
    /// HURA_LIVE_SANDBOX=fwd-probe HURA_LIVE_PORT=8000 \
    ///   cargo test -p hurad -- --ignored forward
    /// ```
    #[tokio::test]
    #[ignore]
    async fn a_forward_is_started_once_and_shared() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let sandbox = std::env::var("HURA_LIVE_SANDBOX").expect("HURA_LIVE_SANDBOX");
        let port: u16 = std::env::var("HURA_LIVE_PORT").map_or(8000, |p| p.parse().unwrap());
        let mut session = Session::new("live".into(), String::new(), String::new());
        session.sandbox = sandbox;

        let mut leases = Vec::new();
        for _ in 0..4 {
            leases.push(
                acquire(&session, port, Loopback::V4)
                    .await
                    .expect("a forward"),
            );
        }
        let local = leases[0].local;
        assert!(
            leases.iter().all(|l| l.local == local),
            "one forward, shared"
        );

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", local))
            .await
            .unwrap();
        stream
            .write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        // Read what arrives rather than to the end: the forward does not pass
        // on the service closing its side -- measured against 0.0.110, the
        // reply arrives whole and the socket stays open -- so a read to EOF
        // waits for ever. A browser never notices, because a dev server sends
        // a length.
        let mut buf = vec![0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buf))
            .await
            .expect("a reply within ten seconds")
            .unwrap();
        let reply = String::from_utf8_lossy(&buf[..n]);
        assert!(reply.starts_with("HTTP/1."), "{reply}");
    }
}
