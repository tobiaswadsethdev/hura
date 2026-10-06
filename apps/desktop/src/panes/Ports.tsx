// What is listening inside the sandbox, what is forwarded out of it, and the
// two ways to make it stop.
//
// Three sources, each the one that knows:
//
//   - the status poll, for *which* ports -- already arriving every couple of
//     seconds, so a dev server the agent starts shows up within one;
//   - the server, asked while this pane is open, for *who* holds each port and
//     which forwards it is running -- a walk of `/proc` that is not worth
//     doing when nobody is looking;
//   - this machine, for the previews it is running and their connections.
//
// **Stop** ends the preview: this machine stops listening, every connection
// through it is closed, and the server's forward goes too -- so a browser tab
// with a live-reload socket open loses it, which is what "stop" has to mean.
// **Kill** ends the process inside the sandbox that holds the port. That is
// usually the agent's own dev server, which is why it asks first.
//
// Which process holds a port is usually a guess, and the pane says so. The
// sandbox lets a process read another's descriptors only if it is that
// process's ancestor, and the exec that asks never is -- so the owner is the
// one process whose command line names the port, when exactly one does. A dev
// server that does not name its port (`vite`) has no owner shown, and is
// stopped from the process list under the ports instead.
//
// A port that is not in the list can still be typed: a server bound to one
// interface rather than to loopback is not listed, because the gateway cannot
// forward to it, and the error from trying says so better than its absence.

import { useCallback, useEffect, useState } from "react";

import { api, messageOf, type Preview } from "../api";
import { useConfirm } from "../Confirm";
import { Empty } from "../Empty";
import type { Forward } from "../gen/Forward";
import type { Listener } from "../gen/Listener";
import type { Listening } from "../gen/Listening";
import type { Loopback } from "../gen/Loopback";
import type { PortsView } from "../gen/PortsView";
import type { Process } from "../gen/Process";
import { Copy, Elsewhere, Forget, NoPorts, Ports, Stop } from "../icons";
import { openExternal } from "../open";
import { refreshPreviews, usePreviews } from "../previews";

/// How often the server is asked who holds each port, while this is open.
const OWNERS_MS = 5000;
/// How often this machine's previews are re-read, for their connection counts.
const PREVIEWS_MS = 2000;

type Row = {
  port: number;
  host: Loopback;
  listening: boolean;
  owner: Listener["owner"];
  preview: Preview | null;
  forward: Forward | null;
};

export function PortsPane({
  server,
  name,
  listening,
}: {
  server: string;
  name: string;
  listening: Listening[];
}) {
  const [view, setView] = useState<PortsView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<number | null>(null);
  const [typed, setTyped] = useState("");
  const previews = usePreviews().filter((p) => p.session === name);
  const { ask, dialog } = useConfirm();

  const reload = useCallback(
    () =>
      api.ports(server, name).then(setView, (e) => setError(messageOf(e))),
    [server, name],
  );
  useEffect(() => {
    void reload();
    const id = setInterval(() => void reload(), OWNERS_MS);
    return () => clearInterval(id);
  }, [reload]);
  useEffect(() => {
    void refreshPreviews(server);
    const id = setInterval(() => void refreshPreviews(server).catch(() => {}), PREVIEWS_MS);
    return () => clearInterval(id);
  }, [server]);

  const act = async (port: number, work: () => Promise<PortsView | void>) => {
    setError(null);
    setBusy(port);
    try {
      const next = await work();
      if (next) setView(next);
      await refreshPreviews(server);
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(null);
    }
  };

  const open = (port: number, host: Loopback) =>
    act(port, async () => {
      const preview = await api.previewOpen(server, name, port, host);
      openExternal(preview.url);
    });
  const stop = (port: number) => act(port, () => api.previewStop(server, name, port));
  const kill = (row: Row) =>
    ask({
      title: `Stop the process on ${row.port}?`,
      body: (
        <>
          <code>{row.owner?.command || "the process"}</code>
          {row.owner && <> (pid {row.owner.pid})</>} is sent <code>TERM</code>, then{" "}
          <code>KILL</code> if it is still there two seconds later.
          {row.owner && !row.owner.certain && (
            <> It was picked because its command line names {row.port}.</>
          )}{" "}
          If the agent started it, the agent will see it exit.
        </>
      ),
      confirm: "kill",
      onConfirm: () => void act(row.port, () => api.killPort(server, name, row.port)),
    });

  const killProcess = (p: Process) =>
    ask({
      title: `Stop ${short(p.command)}?`,
      body: (
        <>
          <code>{p.command}</code> (pid {p.pid}) is sent <code>TERM</code>, then <code>KILL</code>{" "}
          if it is still there two seconds later. If the agent started it, the agent will see it
          exit.
        </>
      ),
      confirm: "kill",
      onConfirm: () => void act(-p.pid, () => api.killProcess(server, name, p.pid)),
    });

  const rows = merge(listening, view, previews);
  const typedPort = Number(typed);
  const typedValid = Number.isInteger(typedPort) && typedPort > 0 && typedPort < 65536;

  return (
    <div className="ports">
      {rows.length === 0 ? (
        <Empty icon={NoPorts} note="nothing listening" />
      ) : (
        <ul>
          {rows.map((row) => {
            const live = row.preview !== null || row.forward !== null;
            return (
              <li key={row.port} className={`${live ? "on" : ""}${busy === row.port ? " busy" : ""}`}>
                <div className="port-line">
                  <Ports />
                  <span
                    className="port"
                    title={row.host === "v6" ? "listening on ::1" : "listening on 127.0.0.1"}
                  >
                    {row.port}
                  </span>
                  <span
                    className={`owner${row.owner && !row.owner.certain ? " guessed" : ""}`}
                    title={
                      row.owner
                        ? `pid ${row.owner.pid}: ${row.owner.command}${
                            row.owner.certain ? "" : "\n(guessed: its command line names this port)"
                          }`
                        : row.listening
                          ? "the process holding this is not visible from here. Check the process list."
                          : undefined
                    }
                  >
                    {!row.listening
                      ? "not listening"
                      : row.owner
                        ? short(row.owner.command)
                        : ""}
                  </span>
                  <span className="port-tools">
                    <button
                      className="quiet-icon"
                      title={row.preview ? "open in browser" : "preview in browser"}
                      disabled={busy === row.port}
                      onClick={() => void open(row.port, row.host)}
                    >
                      <Elsewhere />
                    </button>
                    {row.preview && (
                      <button
                        className="quiet-icon"
                        title="copy address"
                        onClick={() => void navigator.clipboard.writeText(row.preview!.url)}
                      >
                        <Copy />
                      </button>
                    )}
                    {live && (
                      <button
                        className="quiet-icon"
                        title="stop forwarding and close every connection"
                        disabled={busy === row.port}
                        onClick={() => void stop(row.port)}
                      >
                        <Stop />
                      </button>
                    )}
                    {row.owner && (
                      <button
                        className="quiet-icon danger"
                        title={`kill ${short(row.owner.command)} in the sandbox`}
                        disabled={busy === row.port}
                        onClick={() => kill(row)}
                      >
                        <Forget />
                      </button>
                    )}
                  </span>
                </div>
                {live && <Forwarded row={row} />}
              </li>
            );
          })}
        </ul>
      )}

      {view && view.processes.length > 0 && (
        <section className="procs">
          <h4>processes</h4>
          <ul>
            {view.processes.map((p) => (
              <li key={p.pid} className={busy === -p.pid ? "busy" : ""}>
                <span className="pid">{p.pid}</span>
                <span className="cmd" title={p.command}>
                  {short(p.command)}
                </span>
                {p.protected ? (
                  <span className="hint" title="the agent or its terminal. Destroy the worktree to end it.">
                    agent
                  </span>
                ) : (
                  <button
                    className="quiet-icon danger"
                    title={`kill ${short(p.command)}`}
                    disabled={busy === -p.pid}
                    onClick={() => killProcess(p)}
                  >
                    <Forget />
                  </button>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      <form
        className="port-add"
        onSubmit={(e) => {
          e.preventDefault();
          if (!typedValid) return;
          void open(typedPort, "v4");
          setTyped("");
        }}
      >
        <input
          value={typed}
          inputMode="numeric"
          placeholder="another port"
          aria-label="port"
          onChange={(e) => setTyped(e.target.value.replace(/\D/g, ""))}
        />
        <button className="quiet" type="submit" disabled={!typedValid}>
          preview
        </button>
      </form>

      {error && <p className="error">{error}</p>}
      {dialog}
    </div>
  );
}

/// The second line of a forwarded port: where it is on this machine, and how
/// much is going through it at each end.
function Forwarded({ row }: { row: Row }) {
  const bits: React.ReactNode[] = [];
  if (row.preview) {
    bits.push(
      <span key="local" className="local">
        → localhost:{row.preview.local}
      </span>,
    );
    bits.push(
      <span key="here" title="connections open through it from this machine">
        {row.preview.connections} open
      </span>,
    );
  } else {
    // A forward with no preview on this machine: another client's, or one of
    // ours left over from before the window restarted.
    bits.push(<span key="other">forwarded by another client</span>);
  }
  if (row.forward) {
    bits.push(
      <span key="server" title="the server's forward into the sandbox">
        {row.forward.idle_secs !== null
          ? `server idle ${idle(row.forward.idle_secs)}`
          : `${row.forward.connections} on the server`}
      </span>,
    );
  }
  return (
    <div className="port-forward">
      {bits.flatMap((b, i) => (i === 0 ? [b] : [<span key={`dot${i}`} className="dot">·</span>, b]))}
    </div>
  );
}

/// Every port worth a row, from the three sources. A preview whose server has
/// stopped listening keeps its row, so it can still be stopped -- and so a dev
/// server that restarts comes back to a preview that is still there.
function merge(listening: Listening[], view: PortsView | null, previews: Preview[]): Row[] {
  const rows = new Map<number, Row>();
  const row = (port: number, host: Loopback): Row => {
    let r = rows.get(port);
    if (!r) {
      r = { port, host, listening: false, owner: null, preview: null, forward: null };
      rows.set(port, r);
    }
    return r;
  };
  for (const l of listening) row(l.port, l.host).listening = true;
  for (const l of view?.listening ?? []) {
    const r = row(l.port, l.host);
    r.listening = true;
    r.owner = l.owner;
  }
  for (const f of view?.forwards ?? []) row(f.port, f.host).forward = f;
  for (const p of previews) row(p.port, "v4").preview = p;
  return [...rows.values()].sort((a, b) => a.port - b.port);
}

/// A command line, cut to what identifies it: the program and the first
/// argument that is not a flag -- `node vite`, `python3 http.server`.
function short(command: string): string {
  const words = command.split(/\s+/).filter(Boolean);
  if (words.length === 0) return "";
  const base = (w: string) => w.split("/").pop() ?? w;
  const first = base(words[0]);
  const arg = words.slice(1).find((w) => !w.startsWith("-"));
  return arg ? `${first} ${base(arg)}` : first;
}

function idle(secs: number): string {
  return secs < 60 ? `${secs}s` : `${Math.floor(secs / 60)}m`;
}
