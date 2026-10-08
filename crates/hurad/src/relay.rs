//! `hurad relay`: a port inside a session's sandbox, on this machine's loopback.
//!
//! The sandbox runtime publishes a sandbox's port on a loopback port here, but
//! only reaches a server bound beyond the sandbox's own loopback, and what is
//! worth previewing is usually bound to it: a dev server on `localhost`, the
//! chat host on `127.0.0.1`. So the relay starts a `socat` inside the sandbox,
//! on a port of [`RELAY_PORTS`] bound to every address, carrying connections
//! to the real one, and publishes that.
//!
//! It is what [`crate::forward`] runs for each forward, so it keeps that
//! contract: it says which port it bound as `Forwarding 127.0.0.1:N`, and it
//! runs until it is told to stop. Stopping is a `SIGTERM` from the forward's
//! guard, and it stops the `socat` on its way out.
//!
//! **A relay port is published once and kept.** Measured against v0.47.0:
//! unpublishing a port takes it off the running sandbox but not out of the
//! runtime's own record, and the next publish on that sandbox brings every one
//! of them back, each on a new port here. Publishing and withdrawing per
//! preview would leave a growing pile of ports on this machine, some of them
//! leading to whatever a later relay on the same port carries. So a relay
//! reuses the mapping its port already has, and leaves it when it stops: with
//! no `socat` behind it, a connection to it is refused. There are a hundred
//! relay ports, so a sandbox holds at most a hundred.

use std::io::Write as _;

use hura_core::ports::RELAY_PORTS;
use sbx_client::Sbx as _;

/// Start the relay and serve until told to stop.
pub fn run(sandbox: &str, port: u16, host: &str) -> Result<(), String> {
    let client = client();
    let target = match host {
        "127.0.0.1" => format!("TCP:127.0.0.1:{port}"),
        "::1" => format!("TCP6:[::1]:{port}"),
        other => return Err(format!("`{other}` is not a loopback address")),
    };
    let started = client
        .exec(sandbox, &["sh", "-c", &SCRIPT, "sh", &target])
        .map_err(|e| e.to_string())?;
    if !started.ok() {
        return Err(format!(
            "could not start a relay in {sandbox}: {}",
            started.stderr.trim()
        ));
    }
    let (relay, pid) = started
        .stdout
        .trim()
        .split_once(' ')
        .and_then(|(r, p)| Some((r.parse::<u16>().ok()?, p.to_string())))
        .ok_or_else(|| format!("the relay in {sandbox} said `{}`", started.stdout.trim()))?;

    let existing = client
        .ports(sandbox)
        .ok()
        .and_then(|ports| ports.into_iter().find(|p| p.sandbox_port == relay));
    let published = match existing {
        Some(p) => Ok(p),
        None => client.publish(sandbox, relay),
    };
    let published = match published {
        Ok(p) => p,
        Err(e) => {
            let _ = client.exec(sandbox, &["kill", &pid]);
            return Err(format!("could not publish the relay: {e}"));
        }
    };
    println!("{}", announce(published.host_port, host, port, sandbox));
    let _ = std::io::stdout().flush();

    wait_for_stop();

    let _ = client.exec(sandbox, &["kill", &pid]);
    Ok(())
}

/// The line saying where the relay listens, in the words [`crate::forward`]
/// reads the port from.
pub fn announce(local: u16, host: &str, port: u16, sandbox: &str) -> String {
    format!("Forwarding 127.0.0.1:{local} -> {host}:{port} in sandbox {sandbox}")
}

/// Starts `socat` on the first free relay port, detached from the exec that
/// starts it, and prints the port and its pid. A port is free when a `socat`
/// on it is still running a moment later: one that could not bind has exited.
const START: &str = r#"target="$1"
for r in $(seq FIRST LAST); do
  setsid socat TCP-LISTEN:"$r",bind=0.0.0.0,fork,reuseaddr "$target" </dev/null >/dev/null 2>&1 &
  pid=$!
  sleep 0.2
  if kill -0 "$pid" 2>/dev/null; then echo "$r $pid"; exit 0; fi
done
echo "every relay port is taken" >&2
exit 1"#;

/// Block until the forward's guard, or anything else, asks the relay to stop.
fn wait_for_stop() {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    rt.block_on(async {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut term), Ok(mut hup), Ok(mut int)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
            signal(SignalKind::interrupt()),
        ) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = hup.recv() => {}
            _ = int.recv() => {}
        }
    });
}

fn client() -> sbx_client::CliClient {
    let cfg = hura_core::config::Config::load().unwrap_or_default();
    let mut client = sbx_client::CliClient::new();
    if let Some(bin) = &cfg.sbx {
        client = client.with_bin(bin);
    }
    client
}

/// The start script with the relay range written in.
static SCRIPT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    START
        .replace("FIRST", &RELAY_PORTS.start().to_string())
        .replace("LAST", &RELAY_PORTS.end().to_string())
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_script_walks_the_relay_range() {
        assert!(SCRIPT.contains("seq 47700 47799"), "{}", *SCRIPT);
        assert!(!SCRIPT.contains("FIRST"));
    }
}
