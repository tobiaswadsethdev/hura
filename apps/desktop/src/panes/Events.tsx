// The allow/deny feed: every decision the sandbox made, and what to do about
// one.
//
// The pane with no equivalent in an ADE built on git worktrees, and the reason
// the policy one beside it is worth reading -- a rule is a claim, and this is
// what actually happened.
//
// Two readings of one feed. **Endpoints** folds it by destination, because the
// question a denial asks is "should this be reachable?" and that is a question
// about `docs.rs:443`, not about the fourteenth time the agent tried it. **Log**
// is the feed as the runtime said it, newest first, for when the order is the
// point. Both offer the same two changes beside the evidence: open an endpoint,
// or close one that is open, for this session or for every new session too. A
// change is to the whole sandbox, since its rules are not about one program.
//
// The endpoints come in two sections, denied above allowed, because the
// denied ones are the reason anybody opens this pane: each says what was
// refused and carries an allow button, where it used to say "right-click" in a
// hint over the list. Verdicts are the shields the policy pane and the menus
// use, and inside an endpoint the rows say only what the endpoint does not:
// the request, not the host again.
//
// An allow is the whole host or some of its paths. A denial of the host names
// no path, because the connection was refused before any request was made, so
// the paths are typed or pasted as a URL. A denial by a rule's own paths names
// the request it refused, and the panel starts from that.

import { useCallback, useEffect, useMemo, useState } from "react";

import { copy, type MenuItem, useContextMenu } from "../ContextMenu";
import { Empty, Waiting } from "../Empty";
import { AsLog, ByEndpoint, Chevron, Close, Copy, Events, Grant, Plus, Record, Refresh, Revoke } from "../icons";
import { Select } from "../Select";
import { api, messageOf } from "../api";
import type { FeedEvent } from "../gen/FeedEvent";
import type { Route } from "../gen/Route";
import type { View as PolicyView } from "../gen/View";

/// How often the feed is re-read. The worktree list's interval, but never
/// faster than this: a log call per second per open pane is the runtime being
/// watched rather than used.
const FLOOR_MS = 2000;

/// How many of an endpoint's decisions an expanded row shows. The rest are in
/// the log.
const RECENT = 6;

type Mode = "endpoints" | "log";
type Filter = "all" | "denied" | "allowed";

/// One destination, and everything the feed says about it.
type Group = {
  endpoint: string;
  /// Newest first, like the feed.
  events: FeedEvent[];
  denied: number;
  allowed: number;
  /// Every request a denial named, newest first: what a rule's paths refused.
  requests: Route[];
};

/// What the sandbox's own policy says about an endpoint, which is not what the
/// feed says: a denial a minute ago and an allow since is an endpoint that is
/// open now.
type Standing = {
  /// Whether some allow lets requests through to it, and no deny stops them.
  open: boolean;
  /// Whether every request gets through, rather than only some paths.
  whole: boolean;
  /// Whether a deny of the whole host is on it.
  denied: boolean;
  /// The ids of the rules that name it.
  rules: string[];
  /// Which global list it is on, if either.
  listed: "allow" | "block" | null;
};

/// What the action panel under a row is doing, keyed by the row it is under.
/// `scope` is where an allow starts, when the menu item that opened it said.
type Asking = { row: string; kind: "allow" | "block"; scope?: Scope };

/// How much of an endpoint an allow opens.
type Scope = "host" | "paths";

export function EventsPane({
  server,
  name,
  refreshMs,
}: {
  server: string;
  name: string;
  refreshMs: number;
}) {
  const [feed, setFeed] = useState<FeedEvent[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [policy, setPolicy] = useState<PolicyView | null>(null);
  const [mode, setMode] = useState<Mode>("endpoints");
  const [filter, setFilter] = useState<Filter>("all");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [asking, setAsking] = useState<Asking | null>(null);
  // The endpoint a change from the menu is being applied to, and what went
  // wrong with the last one.
  const [busy, setBusy] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const menu = useContextMenu();

  const read = useCallback(
    () =>
      api.events(server, name).then(
        (events) => {
          setFeed(events);
          setError(null);
        },
        (e) => setError(messageOf(e)),
      ),
    [server, name],
  );

  // On a timer, and keeping what it had between reads: a feed that blanked to
  // a spinner every few seconds would lose the row being read.
  useEffect(() => {
    void read();
    const timer = setInterval(() => void read(), Math.max(refreshMs, FLOOR_MS));
    return () => clearInterval(timer);
  }, [read, refreshMs]);

  // Once, and then from what every change answers with. A policy read is a
  // runtime call, and the policy only changes when somebody changes it.
  useEffect(() => {
    let live = true;
    api.policy(server, name).then(
      (view) => live && setPolicy(view),
      // Without it the pane still works; it just cannot say what is open now.
      () => {},
    );
    return () => {
      live = false;
    };
  }, [server, name]);

  const groups = useMemo(() => group(feed ?? []), [feed]);
  const standing = useCallback((endpoint: string) => standingOf(policy, endpoint), [policy]);

  if (error && !feed) return <p className="error">{error}</p>;
  if (!feed) return <Waiting />;
  // The note stays, and it is the qualifier that earns it: this is the
  // *recent* log rather than every decision ever made, so an empty feed is
  // not a claim that the sandbox has never denied anything.
  if (feed.length === 0) return <Empty icon={Events} note="no decisions in the recent log" />;

  const toneOf = (g: Group) => toneFor(g, standing(g.endpoint));
  const blocked = groups.filter((g) => toneOf(g) === "denied");
  const passing = groups.filter((g) => toneOf(g) === "allowed");
  // The chips count what the view lists: endpoints when it is folded, events
  // when it is the log.
  const counts =
    mode === "endpoints"
      ? { all: groups.length, denied: blocked.length, allowed: passing.length }
      : {
          all: feed.length,
          denied: feed.filter((e) => e.verdict === "Denied").length,
          allowed: feed.filter((e) => e.verdict === "Allowed").length,
        };
  const shownLog = feed.filter((e) =>
    filter === "denied"
      ? e.verdict === "Denied"
      : filter === "allowed"
        ? e.verdict === "Allowed"
        : true,
  );
  const untargeted = feed.length - groups.reduce((n, g) => n + g.events.length, 0);

  const onChanged = (view: PolicyView) => {
    setPolicy(view);
    void read();
  };
  const actions: Actions = {
    server,
    name,
    asking,
    setAsking,
    standing,
    onChanged,
    busy,
    menu,
    apply: (endpoint, change) => {
      setBusy(endpoint);
      setFailed(null);
      setAsking(null);
      change
        .then(onChanged, (e) => setFailed(messageOf(e)))
        .finally(() => setBusy(null));
    },
  };
  const rows = (list: Group[]) =>
    list.map((g) => (
      <EndpointRow
        key={g.endpoint}
        group={g}
        tone={toneOf(g)}
        open={expanded.has(g.endpoint)}
        onToggle={() =>
          setExpanded((prev) => {
            const next = new Set(prev);
            if (next.has(g.endpoint)) next.delete(g.endpoint);
            else next.add(g.endpoint);
            return next;
          })
        }
        {...actions}
      />
    ));
  const showDenied = filter !== "allowed" && blocked.length > 0;
  const showAllowed = filter !== "denied" && passing.length > 0;

  return (
    <div className="traffic">
      {/* One row: which decisions, then how they are read. The two readings
          used to be a row of tabs of their own above the filters. */}
      <div className="traffic-bar">
        <div className="traffic-filters" role="group" aria-label="which decisions">
          <Chip on={filter === "all"} onClick={() => setFilter("all")} label="all" count={counts.all} />
          <Chip
            on={filter === "denied"}
            onClick={() => setFilter("denied")}
            label="denied"
            count={counts.denied}
            tone="bad"
          />
          <Chip
            on={filter === "allowed"}
            onClick={() => setFilter("allowed")}
            label="allowed"
            count={counts.allowed}
            tone="ok"
          />
        </div>
        <div className="segmented icons" role="tablist" aria-label="how the feed is read">
          {(
            [
              ["endpoints", ByEndpoint, "by endpoint"],
              ["log", AsLog, "as a log, newest first"],
            ] as const
          ).map(([m, Glyph, label]) => (
            <button
              key={m}
              role="tab"
              aria-selected={mode === m}
              aria-label={label}
              title={label}
              className={mode === m ? "on" : ""}
              onClick={() => setMode(m)}
            >
              <Glyph />
            </button>
          ))}
        </div>
        <button className="quiet-icon" onClick={() => void read()} title="re-read the feed">
          <Refresh aria-label="re-read the feed" />
        </button>
      </div>

      {error && <p className="error">{error}</p>}
      {failed && <p className="error">{failed}</p>}

      {mode === "endpoints" ? (
        <>
          {!showDenied && !showAllowed && <p className="traffic-none">nothing {filter} in the recent log</p>}
          {showDenied && (
            <section>
              <h4>
                denied <span className="count">{blocked.length}</span>
              </h4>
              <ul className="endpoints-list">{rows(blocked)}</ul>
            </section>
          )}
          {showAllowed && (
            <section>
              <h4>
                allowed <span className="count">{passing.length}</span>
              </h4>
              <ul className="endpoints-list">{rows(passing)}</ul>
            </section>
          )}
          {untargeted > 0 && filter === "all" && (
            <button className="traffic-more" onClick={() => setMode("log")}>
              {untargeted === 1 ? "1 event with no endpoint is" : `${untargeted} events with no endpoint are`} in the log
            </button>
          )}
        </>
      ) : shownLog.length === 0 ? (
        <p className="traffic-none">nothing {filter} in the recent log</p>
      ) : (
        <ul className="traffic-log">
          {shownLog.map((e) => (
            <LogRow key={`${e.at}|${e.class}|${e.subject}`} event={e} {...actions} />
          ))}
        </ul>
      )}
    </div>
  );
}

type Actions = {
  server: string;
  name: string;
  asking: Asking | null;
  setAsking: (a: Asking | null) => void;
  standing: (endpoint: string) => Standing;
  onChanged: (view: PolicyView) => void;
  busy: string | null;
  apply: (endpoint: string, change: Promise<PolicyView>) => void;
  menu: ReturnType<typeof useContextMenu>;
};

function EndpointRow({
  group: g,
  tone,
  open,
  onToggle,
  ...actions
}: { group: Group; tone: "denied" | "allowed"; open: boolean; onToggle: () => void } & Actions) {
  const s = actions.standing(g.endpoint);
  const row = `ep:${g.endpoint}`;
  const [host, port] = split(g.endpoint);
  const busy = actions.busy === g.endpoint;
  // The one change that is on the row rather than in its menu: it is what a
  // denial is asking for, and it opens the panel, so it is still asked.
  const canAllow = tone === "denied" && actions.busy === null;
  // A denial by a rule's paths names the request, which is the thing to read.
  const request = g.requests.length > 0 ? g.events.find((e) => e.verdict === "Denied") : undefined;

  const meta: React.ReactNode[] = [
    ...(request ? [<span key="req" className="bin">{brief(request)}</span>] : []),
    ...(g.denied > 0 ? [<span key="d" className="count bad">{g.denied} denied</span>] : []),
    ...(g.allowed > 0 ? [<span key="a" className="count">{g.allowed} allowed</span>] : []),
    <span key="s" title={s.rules.length ? `${s.rules.length} ${s.rules.length === 1 ? "rule" : "rules"} name it` : undefined}>
      {s.denied ? "denied" : s.whole ? "open" : s.open ? "some paths open" : "not in policy"}
    </span>,
    ...(s.listed === "allow" ? [<span key="l">allowed everywhere</span>] : []),
    ...(s.listed === "block" ? [<span key="l">blocked everywhere</span>] : []),
  ];

  return (
    <li
      className={`ep ${tone}${busy ? " busy" : ""}`}
      {...actions.menu(() => itemsFor(g.endpoint, row, g.requests, g.denied > 0, actions))}
    >
      <div className="ep-head">
        <button className="ep-open" onClick={onToggle} aria-expanded={open}>
          <Chevron open={open} className="ep-chevron" />
          <Verdict verdict={tone === "denied" ? "Denied" : "Allowed"} />
          <span className="ep-host" title={g.endpoint}>
            {host}
            <span className="ep-port">:{port}</span>
          </span>
        </button>
        {busy ? (
          <span className="ep-age applying">
            <span className="spin" /> applying
          </span>
        ) : (
          <>
            {canAllow && (
              <button
                className="ep-allow"
                title={`allow this sandbox to reach ${g.endpoint}`}
                onClick={() => actions.setAsking({ row, kind: "allow" })}
              >
                <Grant />
                allow
              </button>
            )}
            <span className="ep-age" title={`last decision ${clock(lastSeen(g))} UTC`}>
              {ago(lastSeen(g))}
            </span>
          </>
        )}
      </div>

      <div className="ep-meta">{dotted(meta)}</div>

      <Panel endpoint={g.endpoint} row={row} requests={g.requests} {...actions} />

      {open && (
        <ul className="ep-log">
          {g.events.slice(0, RECENT).map((e) => (
            <li key={`${e.at}|${e.class}|${e.subject}`}>
              <span className="clock">{clock(e.at)}</span>
              <Verdict verdict={e.verdict} />
              <span className="what" title={e.subject}>
                {breakable(brief(e))}
              </span>
              <Times event={e} />
              {e.policy && <span className="rule">{e.policy}</span>}
              {e.reason && !GENERIC.test(e.reason) && <span className="reason">{e.reason}</span>}
            </li>
          ))}
          {g.events.length > RECENT && (
            <li className="ep-log-more">{g.events.length - RECENT} earlier, in the log</li>
          )}
        </ul>
      )}
    </li>
  );
}

function LogRow({ event: e, ...actions }: { event: FeedEvent } & Actions) {
  const row = `log:${e.at}|${e.class}|${e.subject}`;
  const request = e.verdict === "Denied" ? requestOf(e) : null;
  const requests = request ? [request] : [];
  return (
    <li
      className={`entry ${e.verdict.toLowerCase()}`}
      // Focusable, so the Menu key reaches it: nothing else in a log row is.
      tabIndex={0}
      {...actions.menu(() => {
        const line: MenuItem = { label: "Copy line", icon: Copy, run: () => copy(lineOf(e)) };
        return e.target
          ? itemsFor(e.target.endpoint, row, requests, e.verdict === "Denied", actions, [line])
          : [line];
      })}
    >
      {/* What happened first, and when at the end of the same line; the
          class, the rule and the reason are the quiet line under it. */}
      <div className="entry-head">
        <Verdict verdict={e.verdict} />
        <span className={`subject${e.target ? "" : " prose"}`} title={e.subject}>
          {breakable(pretty(e))}
        </span>
        {e.target && actions.busy === e.target.endpoint ? (
          <span className="ep-age applying">
            <span className="spin" /> applying
          </span>
        ) : (
          <>
            <Times event={e} />
            <span className="clock">{clock(e.at)}</span>
          </>
        )}
      </div>
      <div className="entry-why">
        <span className="class">{e.class}</span>
        {e.policy && <span className="rule">{e.policy}</span>}
        {e.reason && (
          <span className="reason" title={e.reason}>
            {GENERIC.test(e.reason) ? "no rule allows it" : e.reason}
          </span>
        )}
      </div>
      {e.target && (
        <Panel endpoint={e.target.endpoint} row={row} requests={requests} {...actions} />
      )}
    </li>
  );
}

/// A decision, as the shield the policy pane and the menus draw it: a tick for
/// let through, a bar for refused, and the plain information glyph for a line
/// that decides nothing.
function Verdict({ verdict }: { verdict: FeedEvent["verdict"] }) {
  if (verdict === "Denied") return <Revoke className="verdict-glyph denied" aria-label="denied" />;
  if (verdict === "Allowed") return <Grant className="verdict-glyph allowed" aria-label="allowed" />;
  return <Record className="verdict-glyph neutral" aria-label="no decision" />;
}

/// What a row's menu offers, each item only when it would change something: an
/// allow beside a denial the policy has not since answered, a block beside an
/// endpoint something still opens. Copying is always there.
///
/// An allow of the whole host from the menu goes straight through, because
/// that is the answer nearly every time and the menu is already the deliberate
/// gesture. Choosing paths, and a block (which takes an endpoint from the whole
/// sandbox, git included), open the panel under the row instead, so the
/// question is asked beside the evidence.
///
/// A denial by a rule's paths has no one-click allow. The click would give the
/// whole host, which is the opposite of what a rule written path by path was
/// for, so the panel opens on the path that was refused instead.
function itemsFor(
  endpoint: string,
  row: string,
  requests: Route[],
  denied: boolean,
  a: Actions,
  more: MenuItem[] = [],
): MenuItem[] {
  const s = a.standing(endpoint);
  const items: MenuItem[] = [];

  if (denied && !s.whole && a.busy === null) {
    if (requests.length > 0) {
      items.push(
        {
          label: requests.length === 1 ? "Allow this path…" : "Allow these paths…",
          icon: Grant,
          hint: requests.length === 1 ? routeText(requests[0]) : `${requests.length} refused`,
          run: () => a.setAsking({ row, kind: "allow", scope: "paths" }),
        },
        {
          label: "Allow the whole host…",
          run: () => a.setAsking({ row, kind: "allow", scope: "host" }),
        },
      );
    } else {
      items.push(
        {
          label: "Allow in this session",
          icon: Grant,
          run: () => a.apply(endpoint, api.allow(a.server, a.name, endpoint, false)),
        },
        {
          label: "Allow in every new session too",
          icon: Grant,
          run: () => a.apply(endpoint, api.allow(a.server, a.name, endpoint, true)),
        },
        {
          label: "Allow only some paths…",
          hint: "method and path",
          run: () => a.setAsking({ row, kind: "allow", scope: "paths" }),
        },
      );
    }
  }
  if (s.open && !s.denied && a.busy === null) {
    if (items.length) items.push("separator");
    items.push({
      label: "Block…",
      icon: Revoke,
      danger: true,
      run: () => a.setAsking({ row, kind: "block" }),
    });
  }
  if (items.length) items.push("separator");
  items.push({ label: "Copy endpoint", icon: Copy, hint: endpoint, run: () => copy(endpoint) }, ...more);
  return items;
}

/// The question an allow or a block asks, inline under the row it is about.
///
/// Inline rather than a dialog: the evidence is the row above it, and a modal
/// would cover the thing the answer depends on.
function Panel({
  endpoint,
  row,
  requests,
  server,
  name,
  asking,
  setAsking,
  standing,
  onChanged,
}: { endpoint: string; row: string; requests: Route[] } & Actions) {
  if (asking?.row !== row) return null;
  return (
    <ChangePanel
      key={`${row}:${asking.kind}:${asking.scope ?? ""}`}
      kind={asking.kind}
      scope={asking.scope}
      endpoint={endpoint}
      standing={standing(endpoint)}
      requests={requests}
      run={(everywhere, routes) =>
        asking.kind === "block"
          ? api.block(server, name, endpoint, everywhere)
          : routes
            ? api.allowPaths(server, name, endpoint, routes, everywhere)
            : api.allow(server, name, endpoint, everywhere)
      }
      onDone={(view) => {
        setAsking(null);
        onChanged(view);
      }}
      onCancel={() => setAsking(null)}
    />
  );
}

function ChangePanel({
  kind,
  scope: asked,
  endpoint,
  standing: s,
  requests,
  run,
  onDone,
  onCancel,
}: {
  kind: "allow" | "block";
  scope?: Scope;
  endpoint: string;
  standing: Standing;
  requests: Route[];
  run: (everywhere: boolean, routes?: Route[]) => Promise<PolicyView>;
  onDone: (view: PolicyView) => void;
  onCancel: () => void;
}) {
  // A refused path is the evidence that paths are what is wanted.
  const [scope, setScope] = useState<Scope>(asked ?? (requests.length > 0 ? "paths" : "host"));
  const [routes, setRoutes] = useState<Route[]>(() =>
    requests.length > 0 ? requests.map((r) => ({ ...r })) : [{ method: "GET", path: "" }],
  );
  const [everywhere, setEverywhere] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [host] = split(endpoint);

  const narrowing = kind === "allow" && scope === "paths";
  const problems = routes.map((r) => pathProblem(r.path));
  const ready = !narrowing || problems.every((p) => p === null);

  const edit = (i: number, change: Partial<Route>) =>
    setRoutes((prev) => prev.map((r, j) => (j === i ? { ...r, ...change } : r)));

  const go = async () => {
    setBusy(true);
    setError(null);
    try {
      if (narrowing) {
        const wanted = routes.map((r) => ({ method: r.method, path: r.path.trim() }));
        const unique = wanted.filter((r, i) => wanted.findIndex((q) => sameRoute(q, r)) === i);
        onDone(await run(everywhere, unique));
      } else {
        onDone(await run(everywhere));
      }
    } catch (e) {
      setError(messageOf(e));
      setBusy(false);
    }
  };

  return (
    <div className={`change-panel ${kind}`}>
      {kind === "allow" ? (
        <>
          <p className="change-what">
            <Grant className="allowed" /> Allow <code>{endpoint}</code>
          </p>

          <div className="segmented" role="tablist" aria-label="how much of the endpoint">
            {(
              [
                ["host", "the whole host"],
                ["paths", "only these paths"],
              ] as const
            ).map(([value, label]) => (
              <button
                key={value}
                role="tab"
                aria-selected={scope === value}
                className={scope === value ? "on" : ""}
                disabled={busy}
                onClick={() => setScope(value)}
              >
                {label}
              </button>
            ))}
          </div>

          {!narrowing ? (
            <p className="change-note">
              Every request, from anything in this sandbox.
              {s.denied && <> The deny on it here is lifted first.</>}
            </p>
          ) : (
            <>
              <div className="change-paths">
                {routes.map((r, i) => (
                  <div key={i} className="change-path">
                    <Select
                      value={r.method}
                      options={METHODS}
                      onChange={(method) => edit(i, { method })}
                      disabled={busy}
                      className="change-method"
                      aria-label="method"
                    />
                    <input
                      value={r.path}
                      // A URL pasted whole is cut down to its path, which is
                      // the part the rule names.
                      onChange={(e) => edit(i, { path: asPath(e.target.value, host) })}
                      placeholder="/contoso/_packaging/feed/nuget/v3/**"
                      spellCheck={false}
                      autoFocus={i === 0 && !r.path}
                      disabled={busy}
                      aria-label="path"
                      aria-invalid={!!problems[i]}
                    />
                    {routes.length > 1 && (
                      <button
                        className="quiet-icon"
                        disabled={busy}
                        onClick={() => setRoutes((prev) => prev.filter((_, j) => j !== i))}
                        title="remove this path"
                      >
                        <Close aria-label="remove this path" />
                      </button>
                    )}
                    {problems[i] && <span className="change-problem">{problems[i]}</span>}
                  </div>
                ))}
                <button
                  className="quiet change-more"
                  disabled={busy}
                  onClick={() => setRoutes((prev) => [...prev, { method: "GET", path: "" }])}
                >
                  <Plus /> another path
                </button>
              </div>
              <p className="change-note">
                Anything else on the host stays denied and turns up here with its path.{" "}
                <code>*</code> is one segment, <code>**</code> any number.
              </p>
            </>
          )}
        </>
      ) : (
        <>
          <p className="change-what">
            <Revoke className="denied" /> Block <code>{endpoint}</code>
          </p>
          <p className="change-note">
            Denied to everything in this sandbox, whatever opens it. Anything that depends on it
            stops working.
          </p>
        </>
      )}

      <label className="tick">
        <input
          type="checkbox"
          checked={everywhere}
          disabled={busy}
          onChange={(e) => setEverywhere(e.target.checked)}
        />
        <span>every new session too</span>
      </label>

      {error && <p className="error">{error}</p>}

      <div className="change-actions">
        <button className="quiet" disabled={busy} onClick={onCancel}>
          cancel
        </button>
        <button
          className={`go ${kind === "block" ? "danger" : ""}`}
          disabled={busy || !ready}
          onClick={() => void go()}
        >
          {busy ? (
            <>
              <span className="spin" /> applying
            </>
          ) : kind === "allow" ? (
            "allow"
          ) : (
            "block"
          )}
        </button>
      </div>
    </div>
  );
}

/// The methods a path can be opened for. `ANY` is the runtime's own word for
/// all of them.
const METHODS = [
  { value: "GET", label: "GET" },
  { value: "HEAD", label: "HEAD" },
  { value: "POST", label: "POST" },
  { value: "PUT", label: "PUT" },
  { value: "PATCH", label: "PATCH" },
  { value: "DELETE", label: "DELETE" },
  { value: "ANY", label: "ANY", hint: "any method" },
];

/// What is wrong with a path as the runtime will read it, `""` for nothing
/// typed yet, and `null` for nothing. The server checks again; this is so the
/// button says so before a round trip does.
function pathProblem(path: string): string | null {
  const p = path.trim();
  if (!p) return "";
  if (!p.startsWith("/")) return "a path starts with /";
  if (/[?#%]/.test(p)) return "the path alone: no query, fragment or %-escapes";
  if (p.split("/").includes("..")) return "no .. in a path";
  if (/\s/.test(p)) return "no spaces";
  return null;
}

/// `https://host/a/b` or `host/a/b` as `/a/b`, when the host is this one.
/// Anything else is left as typed.
function asPath(text: string, host: string): string {
  const url = /^\s*(?:https?:\/\/)?([^/\s:]+)(?::\d+)?(\/\S*)?\s*$/i.exec(text);
  if (!url || url[1].toLowerCase() !== host.toLowerCase()) return text;
  return url[2] ?? "/";
}

function Chip({
  on,
  onClick,
  label,
  count,
  tone,
}: {
  on: boolean;
  onClick: () => void;
  label: string;
  count: number;
  tone?: "ok" | "bad";
}) {
  return (
    <button className={`chip ${on ? "on" : ""} ${tone ?? ""}`} aria-pressed={on} onClick={onClick}>
      {label} <span className="n">{count}</span>
    </button>
  );
}

/// `Verdict` is PascalCase on the wire where `State` is lowercase, which is an
/// inconsistency worth leaving alone: events are persisted as JSONL per session,
/// so a `rename_all` would make every file already on disk unreadable. The
/// generated type is what settles it -- this comparison was written against the
/// wrong casing and would have failed silently, colouring every denial as
/// neutral, if the types had been copied by hand instead.
const VERDICT = { Allowed: "allow", Denied: "DENY", Neutral: "-" } as const;

/// The feed folded by endpoint, most recently active first. Events that name
/// no endpoint are left to the log.
function group(feed: FeedEvent[]): Group[] {
  const byEndpoint = new Map<string, Group>();
  for (const e of feed) {
    if (!e.target) continue;
    const key = e.target.endpoint;
    let g = byEndpoint.get(key);
    if (!g) {
      g = { endpoint: key, events: [], denied: 0, allowed: 0, requests: [] };
      byEndpoint.set(key, g);
    }
    g.events.push(e);
    if (e.verdict === "Denied") {
      g.denied += e.count;
      const r = requestOf(e);
      if (r && !g.requests.some((q) => sameRoute(q, r))) g.requests.push(r);
    } else if (e.verdict === "Allowed") {
      g.allowed += e.count;
    }
  }
  // Most recently active first: the runtime counts a decision in one row, so
  // a row first seen long ago may be the one that happened a second ago.
  return [...byEndpoint.values()].sort((a, b) => lastSeen(b) - lastSeen(a));
}

function standingOf(policy: PolicyView | null, endpoint: string): Standing {
  const rules = (policy?.rules ?? []).filter((r) => r.hosts.some((h) => hostMatches(h, endpoint)));
  const denied = rules.some((r) => !r.allow && r.methods.length === 0);
  const lists = policy?.lists;
  const allowed = [...(lists?.allow ?? []), ...(lists?.routes ?? [])];
  const listed = allowed.some((a) => a.endpoint === endpoint)
    ? "allow"
    : lists?.block.some((b) => b.endpoint === endpoint)
      ? "block"
      : null;
  return {
    open: !denied && rules.some((r) => r.allow),
    whole: !denied && rules.some((r) => r.allow && r.methods.length === 0),
    denied,
    rules: rules.map((r) => r.id),
    listed,
  };
}

/// Whether a rule's host covers an endpoint, the way the runtime matches it: a
/// bare host is any port and not its subdomains, `*.` is one label, `**.` any
/// number, and `**` is everything. The same rule as `policy::host_matches`.
function hostMatches(resource: string, endpoint: string): boolean {
  const [host, port] = split(endpoint);
  const m = /^(.*):(\d+)$/.exec(resource);
  const pattern = (m ? m[1] : resource).toLowerCase();
  if (m && m[2] !== port) return false;
  const h = host.toLowerCase();
  if (pattern === "**") return true;
  if (pattern.startsWith("**.")) return h.endsWith(pattern.slice(2));
  if (pattern.startsWith("*.")) {
    const label = h.endsWith(pattern.slice(1)) ? h.slice(0, h.length - pattern.length + 1) : "";
    return label.length > 0 && !label.includes(".");
  }
  return h === pattern;
}

/// OpenShell's reason when no rule named the endpoint at all, on events kept
/// from then. It repeats the endpoint the line above it names, so the
/// endpoint's row drops it, since the row already says `not in policy`, and the
/// log says it in four words, with the original on hover and in "Copy line".
const GENERIC = /^endpoint \S+ is not allowed by any policy$/;

/// The verdict an endpoint's row wears: its last decision, unless the policy
/// has since moved. A denial followed by an allow of the whole host from this
/// pane is an endpoint that is open, and should stop looking like a problem.
function toneFor(g: Group, s: Standing): "denied" | "allowed" {
  return g.events[0].verdict === "Denied" && !s.whole ? "denied" : "allowed";
}

/// When an endpoint was last decided about. The runtime counts on in one row,
/// so that is the latest `last` rather than the latest first sighting.
function lastSeen(g: Group): number {
  return Math.max(...g.events.map((e) => e.last || e.at));
}

/// How many times a row stands for, when it is more than once, with when it
/// last happened on hover.
function Times({ event: e }: { event: FeedEvent }) {
  if (e.count <= 1) return null;
  return (
    <span className="times" title={`${e.count} times, last at ${clock(e.last || e.at)} UTC`}>
      ×{e.count}
    </span>
  );
}

/// One event as the terminal's feed prints it, for pasting into an issue.
function lineOf(e: FeedEvent): string {
  return [clock(e.at), VERDICT[e.verdict], e.class, e.subject, e.count > 1 && `x${e.count}`, e.policy && `[${e.policy}]`, e.reason]
    .filter(Boolean)
    .join("  ");
}

/// `/usr/bin/node(812) -> registry.npmjs.org:443`, OpenShell's L4 line, which
/// events kept from then still carry.
const OPEN = /^(.*?)(\(\d+\))? -> (\S+)$/;
/// `GET github.com:443/owner/repo.git/info/refs`, its L7 one, and the shape a
/// request the runtime judged is given too.
const REQUEST = /^([A-Z]+) [^/\s]+(\/\S*)?$/;

/// An event as its endpoint's row needs it: the endpoint is the row, so what
/// is left is who asked, or what they asked for.
function brief(e: FeedEvent): string {
  const open = OPEN.exec(e.subject);
  if (open) return `${base(open[1])}${open[2] ?? ""}`;
  const request = REQUEST.exec(e.subject);
  if (request) return `${request[1]} ${request[2] ?? "/"}`;
  return e.subject;
}

/// The method and path a denial refused, when it names a request:
/// `GET pkgs.example.com:443/a/b` or `/usr/bin/curl(5180) -> GET github.com/`.
/// The query is left off, since a path rule names the path.
function requestOf(e: FeedEvent): Route | null {
  const arrow = e.subject.indexOf(" -> ");
  const request = REQUEST.exec(arrow < 0 ? e.subject : e.subject.slice(arrow + 4));
  if (!request) return null;
  return { method: request[1], path: (request[2] ?? "/").split("?")[0] };
}

function sameRoute(a: Route, b: Route): boolean {
  return a.method === b.method && a.path === b.path;
}

function routeText(r: Route): string {
  return `${r.method} ${r.path}`;
}

/// An event as the log shows it: the program by name and an arrow, with the
/// line as it was recorded, path and all, on hover and in "Copy line".
function pretty(e: FeedEvent): string {
  const open = OPEN.exec(e.subject);
  return open ? `${base(open[1])}${open[2] ?? ""} → ${open[3]}` : e.subject;
}

/// A path that may wrap only after a `/`, so a long request breaks between its
/// segments rather than in the middle of `hura.git`. `<wbr>` rather than a
/// zero-width space, which would come along when the line is copied.
function breakable(text: string): React.ReactNode[] {
  return text.split("/").flatMap((part, i) => (i === 0 ? [part] : [<wbr key={i} />, `/${part}`]));
}

/// Parts of a line, with the ports pane's middle dot between them.
function dotted(parts: React.ReactNode[]): React.ReactNode[] {
  return parts.flatMap((p, i) =>
    i === 0 ? [p] : [<span key={`dot${i}`} className="dot">·</span>, p],
  );
}

function split(endpoint: string): [string, string] {
  const i = endpoint.lastIndexOf(":");
  return [endpoint.slice(0, i), endpoint.slice(i + 1)];
}

function base(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

/// UTC, matching the terminal's feed. A denial is compared against the other
/// feed, and both being in the same zone is what makes that possible.
function clock(at: number): string {
  return new Date(at * 1000).toISOString().slice(11, 19);
}

/// How long ago, for a row that is about an endpoint rather than a moment. The
/// exact time is on hover, in the zone the log uses.
function ago(at: number): string {
  const s = Math.max(0, Math.floor(Date.now() / 1000 - at));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}
