//! What is listening inside a sandbox, for the window to offer as a preview.
//!
//! Read from `/proc/net/tcp` and `/proc/net/tcp6` by the status poll, which is
//! already an exec every couple of seconds: a second exec to ask the same
//! sandbox one more question would be another third of a second on every
//! poll. The script keeps only the listening sockets' local
//! addresses, so what comes back is a few short lines however many connections
//! the agent has open.
//!
//! Only what a forward can reach is kept. A relay carries to loopback and
//! nothing else, so a socket bound to one loopback or to every address is a
//! preview, and a socket bound to one particular interface is not.

use serde::{Deserialize, Serialize};

/// The two addresses a forward may target. Not a free string: a relay only
/// carries to loopback, and a client naming any other host would be
/// asking the server to dial somewhere on its behalf.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum Loopback {
    #[default]
    V4,
    V6,
}

impl Loopback {
    pub fn address(self) -> &'static str {
        match self {
            Loopback::V4 => "127.0.0.1",
            Loopback::V6 => "::1",
        }
    }
}

/// A port something in the sandbox is listening on, and where to reach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Listening {
    pub port: u16,
    pub host: Loopback,
}

/// The ports `hurad relay` listens on inside a sandbox, one per preview.
///
/// The sandbox runtime publishes only ports bound beyond the sandbox's own
/// loopback, so a relay on one of these carries a preview of a server that is
/// bound to it. Reserved, so the relays never show up as previews themselves.
pub const RELAY_PORTS: std::ops::RangeInclusive<u16> = 47700..=47799;

/// The shell that prints each listening socket's local address, one per line,
/// in the kernel's own hex. State `0A` is `LISTEN`.
pub const SCRIPT: &str = r#"awk '$4=="0A"{print $2}' /proc/net/tcp /proc/net/tcp6 2>/dev/null"#;

/// The poll's ports section, interpreted. Sorted by port, one entry each.
///
/// IPv4 wins a port that is open on both: it is what a dev server bound to
/// `0.0.0.0` or `::` -- dual-stack -- answers on, and `127.0.0.1` is the
/// address nobody has to think about.
pub fn parse(text: &str) -> Vec<Listening> {
    let mut found: Vec<Listening> = Vec::new();
    for line in text.lines() {
        let Some((addr, port)) = line.trim().split_once(':') else {
            continue;
        };
        let Ok(port) = u16::from_str_radix(port, 16) else {
            continue;
        };
        if RELAY_PORTS.contains(&port) {
            continue;
        }
        let Some(host) = reachable(addr) else {
            continue;
        };
        match found.iter_mut().find(|l| l.port == port) {
            Some(existing) if host == Loopback::V4 => existing.host = Loopback::V4,
            Some(_) => {}
            None => found.push(Listening { port, host }),
        }
    }
    found.sort_by_key(|l| l.port);
    found
}

/// Which loopback reaches a socket bound to this address, if either does.
///
/// The kernel prints addresses as the bytes in memory, which on every machine
/// a sandbox runs on is little-endian per 32-bit word: `127.0.0.1` is
/// `0100007F`, and `::1` is three zero words then `01000000`.
fn reachable(addr: &str) -> Option<Loopback> {
    match addr.to_ascii_uppercase().as_str() {
        // 127.0.0.1, and 0.0.0.0.
        "0100007F" | "00000000" => Some(Loopback::V4),
        // ::ffff:127.0.0.1, an IPv4 loopback socket seen through tcp6.
        "0000000000000000FFFF00000100007F" => Some(Loopback::V4),
        // ::1, and :: -- which takes IPv6 for certain, and IPv4 only when the
        // socket is dual-stack, which nothing here can see.
        "00000000000000000000000001000000" | "00000000000000000000000000000000" => {
            Some(Loopback::V6)
        }
        _ => None,
    }
}

/// A listening port, and the process that holds it when that can be told.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Listener {
    pub port: u16,
    pub host: Loopback,
    pub owner: Option<Owner>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Owner {
    pub pid: u32,
    /// The command line, arguments and all: `node` alone does not say which
    /// of three dev servers this is.
    pub command: String,
    /// Whether the socket was traced to this process, or the process was
    /// picked because its command line names the port.
    ///
    /// Certain for anything running as the sandbox's user: the VM has no
    /// Yama, so an exec reads the descriptors of a dev server it did not
    /// start (measured against v0.47.0). A server started with `sudo` keeps
    /// its descriptors from the exec, and is the guess.
    pub certain: bool,
}

/// A process in the sandbox that a person might want to stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pub command: String,
    /// The agent itself, or the tmux it runs in. Shown, so the list is the
    /// truth about the sandbox, and never killed from here: that is ending
    /// the session, which has its own button.
    pub protected: bool,
}

/// A forward `hurad` is running into a sandbox, for a client to show and stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Forward {
    pub port: u16,
    pub host: Loopback,
    /// Connections through it right now, from every client.
    pub connections: u32,
    /// How long it has had none, in seconds. `None` while it has some.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub idle_secs: Option<u64>,
}

/// One sandbox's ports, as the ports pane draws them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PortsView {
    pub listening: Vec<Listener>,
    pub processes: Vec<Process>,
    pub forwards: Vec<Forward>,
}

/// What [`sandbox_script`] found, before the server adds its forwards.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub listening: Vec<Listener>,
    pub processes: Vec<Process>,
}

const LISTEN_MARKER: &str = "===hura-listen===";
const FDS_MARKER: &str = "===hura-fds===";
const PROCS_MARKER: &str = "===hura-procs===";

/// The script that reads a sandbox's listening sockets and processes, in one
/// exec.
///
/// Three sections: the listening sockets with their inodes; every process's
/// descriptors as `ls -l` shows them, which names a socket's inode wherever the
/// sandbox lets one process read another's -- one `ls` for all of them, where a
/// `readlink` each would be hundreds; and each process's parent, user and
/// command line, with this script's own pid first so it can leave itself out.
///
/// Asked only while somebody is looking at the ports, not on the poll: it walks
/// `/proc`, and the poll runs every couple of seconds for as long as the
/// session exists.
pub fn sandbox_script() -> String {
    format!(
        r#"echo {LISTEN_MARKER}
awk '$4=="0A"{{print $2, $10}}' /proc/net/tcp /proc/net/tcp6 2>/dev/null
echo {FDS_MARKER}
ls -l /proc/[0-9]*/fd 2>/dev/null
echo {PROCS_MARKER}
echo "self $$ $(id -u)"
for d in /proc/[0-9]*; do
  st=$(awk '/^PPid/{{p=$2}} /^Uid/{{u=$2}} END{{print p, u}}' "$d/status" 2>/dev/null) || continue
  printf '%s %s ' "${{d#/proc/}}" "$st"
  tr '\0\n' '  ' < "$d/cmdline" 2>/dev/null
  echo
done
"#
    )
}

/// What [`sandbox_script`] printed, interpreted.
pub fn parse_sandbox(out: &str) -> Found {
    let section = |marker: &str, next: Option<&str>| {
        let rest = out.split_once(marker).map(|(_, r)| r).unwrap_or("");
        match next {
            Some(n) => rest.split_once(n).map(|(s, _)| s).unwrap_or(rest),
            None => rest,
        }
    };
    let listen = section(LISTEN_MARKER, Some(FDS_MARKER));
    let fds = section(FDS_MARKER, Some(PROCS_MARKER));
    let procs = section(PROCS_MARKER, None);

    // inode -> pid, from `ls -l`'s `/proc/123/fd:` headers and its
    // `... 5 -> socket:[2576303]` lines, which a root process's are without.
    let mut by_inode = std::collections::HashMap::<&str, u32>::new();
    let mut pid: Option<u32> = None;
    for line in fds.lines() {
        if let Some(header) = line
            .strip_prefix("/proc/")
            .and_then(|l| l.strip_suffix("/fd:"))
        {
            pid = header.parse().ok();
            continue;
        }
        if let (Some(p), Some(inode)) = (pid, line.split("socket:[").nth(1)) {
            by_inode.entry(inode.trim_end_matches(']')).or_insert(p);
        }
    }

    let processes = processes(procs);

    let mut found: Vec<Listener> = Vec::new();
    for line in listen.lines() {
        let mut words = line.split_whitespace();
        let (Some(local), Some(inode)) = (words.next(), words.next()) else {
            continue;
        };
        let Some(listening) = parse(local).into_iter().next() else {
            continue;
        };
        let owner = by_inode
            .get(inode)
            .and_then(|pid| processes.iter().find(|p| p.pid == *pid))
            .map(|p| Owner {
                pid: p.pid,
                command: p.command.clone(),
                certain: true,
            });
        match found.iter_mut().find(|l| l.port == listening.port) {
            Some(existing) => {
                if listening.host == Loopback::V4 {
                    existing.host = Loopback::V4;
                }
                if existing.owner.is_none() {
                    existing.owner = owner;
                }
            }
            None => found.push(Listener {
                port: listening.port,
                host: listening.host,
                owner,
            }),
        }
    }
    for l in &mut found {
        if l.owner.is_none() {
            l.owner = guess(l.port, &processes);
        }
    }
    found.sort_by_key(|l| l.port);
    Found {
        listening: found,
        processes,
    }
}

/// The processes worth listing: this user's, without the sandbox's own
/// machinery, this script, or the shells everything else runs under.
fn processes(section: &str) -> Vec<Process> {
    let mut lines = section.lines().filter(|l| !l.trim().is_empty());
    let (me, uid) = match lines
        .next()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
    {
        Some(w) if w.len() == 3 && w[0] == "self" => (w[1].parse::<u32>().ok(), w[2].to_string()),
        _ => (None, String::new()),
    };
    let all: Vec<(u32, u32, String, String)> = lines
        .filter_map(|l| {
            let mut w = l.splitn(4, ' ');
            let pid = w.next()?.parse().ok()?;
            let ppid = w.next()?.parse().ok()?;
            let user = w.next()?.to_string();
            Some((pid, ppid, user, w.next().unwrap_or("").trim().to_string()))
        })
        .collect();
    let parent: std::collections::HashMap<u32, u32> =
        all.iter().map(|(p, pp, _, _)| (*p, *pp)).collect();
    // This script and everything under it: the `awk` and `tr` it is running
    // as it reads the list are processes too.
    let mine = |mut pid: u32| {
        for _ in 0..16 {
            if Some(pid) == me {
                return true;
            }
            match parent.get(&pid) {
                Some(&pp) if pp != 0 => pid = pp,
                _ => return false,
            }
        }
        false
    };
    let mut out: Vec<Process> = all
        .into_iter()
        .filter(|(pid, _, user, cmd)| {
            *pid > 1 && *user == uid && !cmd.is_empty() && !mine(*pid) && !plumbing(cmd)
        })
        .map(|(pid, ppid, _, command)| Process {
            pid,
            ppid,
            protected: protected(&command),
            command,
        })
        .collect();
    out.sort_by_key(|p| p.pid);
    out
}

fn program(command: &str) -> &str {
    let first = command.split_whitespace().next().unwrap_or("");
    first.rsplit('/').next().unwrap_or(first)
}

/// Not worth a row: a shell, the sandbox keeping itself alive, or a relay
/// carrying a preview out.
fn plumbing(command: &str) -> bool {
    matches!(
        program(command),
        "sh" | "bash" | "dash" | "zsh" | "fish" | "ash" | "tini" | "socat"
    ) || command == "sleep infinity"
}

/// The agent and its tmux, and the sandbox's own Docker Engine. A `node`
/// running Claude Code is named for the script rather than the interpreter, so
/// every word is looked at.
fn protected(command: &str) -> bool {
    command.split_whitespace().any(|w| {
        let base = w.rsplit('/').next().unwrap_or(w);
        matches!(base, "claude" | "tmux" | "dockerd" | "containerd") || base.starts_with("tmux:")
    })
}

/// The one process whose command line names the port, if exactly one does:
/// `http.server 8000`, `--port 5173`, `-p 3000`, `:8080`.
fn guess(port: u16, processes: &[Process]) -> Option<Owner> {
    let needle = port.to_string();
    let mut named = processes.iter().filter(|p| {
        !p.protected
            && p.command
                .split(|c: char| !c.is_ascii_digit())
                .any(|w| w == needle)
    });
    let one = named.next()?;
    if named.next().is_some() {
        return None;
    }
    Some(Owner {
        pid: one.pid,
        command: one.command.clone(),
        certain: false,
    })
}

/// Stop a process: asked nicely, then told. A dev server that traps `TERM`
/// to shut down cleanly gets two seconds to do it.
pub fn kill_script(pid: u32) -> String {
    format!(
        "kill -TERM {pid} 2>/dev/null || exit 0
i=0; while [ $i -lt 10 ] && kill -0 {pid} 2>/dev/null; do sleep 0.2; i=$((i+1)); done
kill -KILL {pid} 2>/dev/null; exit 0"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn procs(lines: &str) -> String {
        format!("{PROCS_MARKER}\nself 200 998\n{lines}")
    }

    /// No descriptor targets, as for a server started with `sudo`: the owner
    /// is a guess from the command line, and says so.
    #[test]
    fn without_descriptors_the_owner_is_guessed_from_the_command_line() {
        let out = format!(
            "{LISTEN_MARKER}\n0100007F:1F40 2851747\n00000000000000000000000001000000:1435 991\n\
             {FDS_MARKER}\n/proc/71/fd:\nlrwx------ 1 sandbox sandbox 64 Oct  2 14:18 3\n{}",
            procs(
                "1 0 0 /opt/openshell/bin/openshell-sandbox --workdir /sandbox \n\
                 56 1 998 sleep infinity \n\
                 71 1 998 python3 -m http.server 8000 --bind 127.0.0.1 \n\
                 80 1 998 node /repo/node_modules/.bin/vite \n\
                 200 1 998 sh -c the script \n\
                 201 200 998 awk /^PPid/ \n"
            )
        );
        let found = parse_sandbox(&out);
        let ports: Vec<_> = found.listening.iter().map(|l| l.port).collect();
        assert_eq!(ports, vec![5173, 8000]);
        assert_eq!(
            found.listening[0].owner, None,
            "vite does not name its port"
        );
        let py = found.listening[1].owner.as_ref().expect("a guess");
        assert_eq!((py.pid, py.certain), (71, false));

        let pids: Vec<_> = found.processes.iter().map(|p| p.pid).collect();
        assert_eq!(
            pids,
            vec![71, 80],
            "no supervisor, keepalive, shell or self"
        );
    }

    #[test]
    fn a_readable_descriptor_is_certain() {
        let out = format!(
            "{LISTEN_MARKER}\n0100007F:1F40 2576303\n{FDS_MARKER}\n/proc/71/fd:\n\
             lrwx------ 1 sandbox sandbox 64 Oct  2 13:40 3 -> socket:[2576303]\n{}",
            procs("71 1 998 python3 -m http.server 9999 \n")
        );
        let owner = parse_sandbox(&out).listening[0].owner.clone().unwrap();
        assert_eq!((owner.pid, owner.certain), (71, true));
    }

    #[test]
    fn two_processes_naming_the_port_are_no_guess_at_all() {
        let found = parse_sandbox(&format!(
            "{LISTEN_MARKER}\n0100007F:0BB8 1\n{FDS_MARKER}\n{}",
            procs("10 1 998 node server.js --port 3000 \n11 1 998 curl localhost:3000 \n")
        ));
        assert_eq!(found.listening[0].owner, None);
    }

    #[test]
    fn the_agent_and_its_tmux_are_listed_but_protected() {
        let found = parse_sandbox(&procs(
            "30 1 998 tmux new-session -d -s agent \n\
             31 30 998 node /usr/local/lib/node_modules/@anthropic-ai/claude-code/cli.js \n\
             32 30 998 /usr/local/bin/claude --dangerously \n\
             40 1 1000 someone-elses \n",
        ));
        let protected: Vec<_> = found
            .processes
            .iter()
            .map(|p| (p.pid, p.protected))
            .collect();
        assert_eq!(protected, vec![(30, true), (31, false), (32, true)]);
    }

    #[test]
    fn the_kill_script_names_the_pid_and_nothing_else() {
        let s = kill_script(4242);
        assert!(s.contains("kill -TERM 4242"));
        assert!(s.contains("kill -KILL 4242"));
    }

    fn v4(port: u16) -> Listening {
        Listening {
            port,
            host: Loopback::V4,
        }
    }

    fn v6(port: u16) -> Listening {
        Listening {
            port,
            host: Loopback::V6,
        }
    }

    /// Captured from a sandbox running `python3 -m http.server 8000 --bind
    /// 127.0.0.1` and another on 8001 bound to `::1`.
    #[test]
    fn each_loopback_is_recognised() {
        let out = "0100007F:1F40\n00000000000000000000000001000000:1F41\n";
        assert_eq!(parse(out), vec![v4(8000), v6(8001)]);
    }

    #[test]
    fn every_address_counts_and_one_interface_does_not() {
        let out = "00000000:0BB8\n0A00000A:1F90\n00000000000000000000000000000000:1538\n";
        assert_eq!(parse(out), vec![v4(3000), v6(5432)]);
    }

    /// A dual-stack server shows up in both tables, and is reached on IPv4
    /// whichever order they were read in.
    #[test]
    fn a_port_open_on_both_is_one_entry_on_ipv4() {
        let out = "00000000000000000000000000000000:1435\n00000000:1435\n";
        assert_eq!(parse(out), vec![v4(5173)]);
        let out = "00000000:1435\n00000000000000000000000000000000:1435\n";
        assert_eq!(parse(out), vec![v4(5173)]);
    }

    #[test]
    fn noise_is_ignored() {
        assert!(parse("").is_empty());
        assert!(parse("not a line\n0100007F:zzzz\n:\n").is_empty());
    }

    #[test]
    fn mapped_ipv4_loopback_is_ipv4() {
        assert_eq!(parse("0000000000000000FFFF00000100007F:0050"), vec![v4(80)]);
    }
}
