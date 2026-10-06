// The allow/deny feed: every decision the gateway made, and what to do about
// one.
//
// The pane with no equivalent in an ADE built on git worktrees, and the reason
// the policy one beside it is worth reading -- a rule is a claim, and this is
// what actually happened.
//
// Two readings of one feed. **Endpoints** folds it by destination, because the
// question a denial asks is "should this be reachable?" and that is a question
// about `docs.rs:443`, not about the fourteenth time the agent tried it. **Log**
// is the feed as the gateway said it, newest first, for when the order is the
// point. Both offer the same two changes beside the evidence: open an endpoint
// for the binaries that were refused, or close one that is open -- for this
// session, or for every new session too.
//
// The endpoints come in two sections, denied above allowed, because the
// denied ones are the reason anybody opens this pane: each says what was
// refused and carries an allow button, where it used to say "right-click" in a
// hint over the list. Verdicts are the shields the policy pane and the menus
// use, and inside an endpoint the rows say only what the endpoint does not --
// `node(812)`, not `/usr/bin/node(812) -> registry.npmjs.org:443` again.

import { useCallback, useEffect, useMemo, useState } from "react";

import { copy, type MenuItem, useContextMenu } from "../ContextMenu";
import { Empty, Waiting } from "../Empty";
import { AsLog, ByEndpoint, Chevron, Copy, Events, Grant, Record, Refresh, Revoke } from "../icons";
import { api, messageOf } from "../api";
import type { FeedEvent } from "../gen/FeedEvent";
import type { View as PolicyView } from "../gen/View";

/// How often the feed is re-read. The worktree list's interval, but never
/// faster than this: a logs call per second per open pane is the gateway being
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
  /// Every binary a denial named, newest first. What an allow is offered for.
  refused: string[];
};

/// What the sandbox's own policy says about an endpoint, which is not what the
/// feed says: a denial a minute ago and an allow since is an endpoint that is
/// open now.
type Standing = {
  /// Whether any rule names it.
  open: boolean;
  /// The binaries those rules grant it to.
  binaries: string[];
  rules: string[];
  /// Which global list it is on, if either.
  listed: "allow" | "block" | null;
};

/// What the action panel under a row is doing, keyed by the row it is under.
type Asking = { row: string; kind: "allow" | "block" };

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
  // gateway call, and the policy only changes when somebody changes it.
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
  // not a claim that the gateway has never denied anything.
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
  const last = g.events[0];
  const s = actions.standing(g.endpoint);
  const row = `ep:${g.endpoint}`;
  const [host, port] = split(g.endpoint);
  const busy = actions.busy === g.endpoint;
  const grant = grantable(s, g.refused);
  // The one change that is on the row rather than in its menu: it is what a
  // denial is asking for, and it opens the panel, so it is still asked.
  const canAllow = tone === "denied" && grant.length > 0 && actions.busy === null;
  // An L7 denial names no binary; what it names is the request, which is the
  // thing to read.
  const request = g.refused.length === 0 ? g.events.find((e) => e.verdict === "Denied") : undefined;

  const meta: React.ReactNode[] = [
    ...binariesOf(g).map((b) => (
      <span key={b} className="bin" title={b}>
        {base(b)}
      </span>
    )),
    ...(request ? [<span key="req" className="bin">{brief(request)}</span>] : []),
    ...(g.denied > 0 ? [<span key="d" className="count bad">{g.denied} denied</span>] : []),
    ...(g.allowed > 0 ? [<span key="a" className="count">{g.allowed} allowed</span>] : []),
    <span key="s" title={s.rules.length ? `by ${s.rules.join(", ")}` : undefined}>
      {s.open ? "in policy" : "not in policy"}
    </span>,
    ...(s.listed === "allow" ? [<span key="l">allowed everywhere</span>] : []),
    ...(s.listed === "block" ? [<span key="l">blocked everywhere</span>] : []),
  ];

  return (
    <li
      className={`ep ${tone}${busy ? " busy" : ""}`}
      {...actions.menu(() => itemsFor(g.endpoint, row, g.refused, g.denied > 0, actions))}
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
                title={`allow ${grant.map(base).join(", ")} to reach ${g.endpoint}`}
                onClick={() => actions.setAsking({ row, kind: "allow" })}
              >
                <Grant />
                allow
              </button>
            )}
            <span className="ep-age" title={`last decision ${clock(last.at)} UTC`}>
              {ago(last.at)}
            </span>
          </>
        )}
      </div>

      <div className="ep-meta">{dotted(meta)}</div>

      <Panel endpoint={g.endpoint} row={row} refused={g.refused} {...actions} />

      {open && (
        <ul className="ep-log">
          {g.events.slice(0, RECENT).map((e) => (
            <li key={`${e.at}|${e.class}|${e.subject}`}>
              <span className="clock">{clock(e.at)}</span>
              <Verdict verdict={e.verdict} />
              <span className="what" title={e.subject}>
                {breakable(brief(e))}
              </span>
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
  const refused = e.verdict === "Denied" && e.target?.binary ? [e.target.binary] : [];
  return (
    <li
      className={`entry ${e.verdict.toLowerCase()}`}
      // Focusable, so the Menu key reaches it: nothing else in a log row is.
      tabIndex={0}
      {...actions.menu(() => {
        const line: MenuItem = { label: "Copy line", icon: Copy, run: () => copy(lineOf(e)) };
        return e.target
          ? itemsFor(e.target.endpoint, row, refused, e.verdict === "Denied", actions, [line])
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
          <span className="clock">{clock(e.at)}</span>
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
      {e.target && <Panel endpoint={e.target.endpoint} row={row} refused={refused} {...actions} />}
    </li>
  );
}

/// A decision, as the shield the policy pane and the menus draw it: a tick for
/// let through, a bar for refused, and the plain information glyph for the
/// gateway saying something that decides nothing.
function Verdict({ verdict }: { verdict: FeedEvent["verdict"] }) {
  if (verdict === "Denied") return <Revoke className="verdict-glyph denied" aria-label="denied" />;
  if (verdict === "Allowed") return <Grant className="verdict-glyph allowed" aria-label="allowed" />;
  return <Record className="verdict-glyph neutral" aria-label="no decision" />;
}

/// What a row's menu offers, each item only when it would change something: an
/// allow beside a denial the policy has not since answered, a block beside an
/// endpoint some rule still opens. Copying is always there.
///
/// An allow from the menu goes straight through, for the binaries that were
/// refused, because that is the answer nearly every time and the menu is
/// already the deliberate gesture. Choosing which binaries, when there is more
/// than one, and a block -- which takes an endpoint from everything, git
/// included -- open the panel under the row instead, so the question is asked
/// beside the evidence.
function itemsFor(
  endpoint: string,
  row: string,
  refused: string[],
  denied: boolean,
  a: Actions,
  more: MenuItem[] = [],
): MenuItem[] {
  const s = a.standing(endpoint);
  const offered = candidates(s, refused);
  const grant = grantable(s, refused);
  const who = grant.map(base).join(", ");
  const items: MenuItem[] = [];

  if (denied && grant.length > 0 && a.busy === null) {
    items.push(
      {
        label: "Allow in this session",
        icon: Grant,
        hint: who,
        run: () => a.apply(endpoint, api.allow(a.server, a.name, endpoint, grant, false)),
      },
      {
        label: "Allow in every new session too",
        icon: Grant,
        hint: who,
        run: () => a.apply(endpoint, api.allow(a.server, a.name, endpoint, grant, true)),
      },
    );
    if (offered.length > 1) {
      items.push({
        label: "Allow…",
        hint: "choose binaries",
        run: () => a.setAsking({ row, kind: "allow" }),
      });
    }
  }
  if (s.open && a.busy === null) {
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
  refused,
  server,
  name,
  asking,
  setAsking,
  standing,
  onChanged,
}: { endpoint: string; row: string; refused: string[] } & Actions) {
  if (asking?.row !== row) return null;
  return (
    <ChangePanel
      key={`${row}:${asking.kind}`}
      kind={asking.kind}
      endpoint={endpoint}
      standing={standing(endpoint)}
      refused={refused}
      run={(binaries, everywhere) =>
        asking.kind === "allow"
          ? api.allow(server, name, endpoint, binaries, everywhere)
          : api.block(server, name, endpoint, everywhere)
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
  endpoint,
  standing: s,
  refused,
  run,
  onDone,
  onCancel,
}: {
  kind: "allow" | "block";
  endpoint: string;
  standing: Standing;
  refused: string[];
  run: (binaries: string[], everywhere: boolean) => Promise<PolicyView>;
  onDone: (view: PolicyView) => void;
  onCancel: () => void;
}) {
  const offered = candidates(s, refused);
  // The ones that were refused, ticked; the ones a rule already grants are
  // offered only for an L7 denial, where there is nothing else to name.
  const [picked, setPicked] = useState<Set<string>>(
    () => new Set(offered.filter((b) => !s.binaries.includes(b) || refused.length === 0)),
  );
  const [everywhere, setEverywhere] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const go = async () => {
    setBusy(true);
    setError(null);
    try {
      onDone(await run([...picked], everywhere));
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
            <Grant className="allowed" /> Allow <code>{endpoint}</code> for
          </p>
          <div className="change-bins">
            {offered.map((b) => (
              <label key={b} className="tick">
                <input
                  type="checkbox"
                  checked={picked.has(b)}
                  disabled={busy}
                  onChange={(e) =>
                    setPicked((prev) => {
                      const next = new Set(prev);
                      if (e.target.checked) next.add(b);
                      else next.delete(b);
                      return next;
                    })
                  }
                />
                <span className="bin-name">{base(b)}</span>
                <span className="bin-path">{b}</span>
              </label>
            ))}
          </div>
          <p className="change-note">Full access, for these binaries only.</p>
        </>
      ) : (
        <>
          <p className="change-what">
            <Revoke className="denied" /> Block <code>{endpoint}</code>
          </p>
          <p className="change-note">
            Removed from this sandbox for every binary
            {s.binaries.length > 0 && (
              <>
                , including <b>{s.binaries.map(base).join(", ")}</b>
              </>
            )}
            . Anything that depends on it stops working.
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
          disabled={busy || (kind === "allow" && picked.size === 0)}
          onClick={() => void go()}
        >
          {/* The one wait in this pane that keeps its words: the gateway
              takes seconds to load a revision, and the policy is changing. */}
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
      g = { endpoint: key, events: [], denied: 0, allowed: 0, refused: [] };
      byEndpoint.set(key, g);
    }
    g.events.push(e);
    if (e.verdict === "Denied") {
      g.denied++;
      const b = e.target.binary;
      if (b && !g.refused.includes(b)) g.refused.push(b);
    } else if (e.verdict === "Allowed") {
      g.allowed++;
    }
  }
  // The feed is newest first, so insertion order already is.
  return [...byEndpoint.values()];
}

function standingOf(policy: PolicyView | null, endpoint: string): Standing {
  const rules = (policy?.network ?? []).filter((r) =>
    r.endpoints.some((e) => e.host_port === endpoint),
  );
  const listed = policy?.lists?.allow.some((a) => a.endpoint === endpoint)
    ? "allow"
    : policy?.lists?.block.some((b) => b.endpoint === endpoint)
      ? "block"
      : null;
  return {
    open: rules.length > 0,
    binaries: [...new Set(rules.flatMap((r) => r.binaries))],
    rules: rules.map((r) => r.key),
    listed,
  };
}

/// The gateway's reason when no rule names the endpoint at all. It repeats the
/// endpoint the line above it names, so the endpoint's row drops it -- the row
/// already says `not in policy` -- and the log says it in four words, with the
/// gateway's own on hover and in "Copy line".
const GENERIC = /^endpoint \S+ is not allowed by any policy$/;

/// The verdict an endpoint's row wears: its last decision, unless the policy
/// has since moved -- a denial followed by an allow from this pane is an
/// endpoint that is open, and should stop looking like a problem.
function toneFor(g: Group, s: Standing): "denied" | "allowed" {
  return g.events[0].verdict === "Denied" && !opensFor(s, g.refused) ? "denied" : "allowed";
}

/// Who an allow would add: the refused binaries no rule grants yet, or, for an
/// L7 denial that names none, the binaries a rule already sends there.
function grantable(s: Standing, refused: string[]): string[] {
  const offered = candidates(s, refused);
  return refused.length > 0 ? offered.filter((b) => !s.binaries.includes(b)) : offered;
}

/// What an allow can be offered for: the binaries that were refused, or, for
/// an L7 denial that names none, the binaries a rule already sends there --
/// adding full access for them is what lifts a path restriction.
function candidates(s: Standing, refused: string[]): string[] {
  return refused.length > 0 ? refused : s.binaries;
}

/// Whether the policy already grants every refused binary. For an L7 denial
/// there is nothing to compare, so it never counts as already open.
function opensFor(s: Standing, refused: string[]): boolean {
  return refused.length > 0 && refused.every((b) => s.binaries.includes(b));
}

function binariesOf(g: Group): string[] {
  const all: string[] = [];
  for (const e of g.events) {
    const b = e.target?.binary;
    if (b && !all.includes(b)) all.push(b);
  }
  return all;
}

/// One event as the terminal's feed prints it, for pasting into an issue.
function lineOf(e: FeedEvent): string {
  return [clock(e.at), VERDICT[e.verdict], e.class, e.subject, e.policy && `[${e.policy}]`, e.reason]
    .filter(Boolean)
    .join("  ");
}

/// `/usr/bin/node(812) -> registry.npmjs.org:443`, the gateway's L4 line.
const OPEN = /^(.*?)(\(\d+\))? -> (\S+)$/;
/// `GET github.com:443/owner/repo.git/info/refs`, its L7 one.
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

/// An event as the log shows it: the program by name and an arrow, with the
/// gateway's own line, path and all, on hover and in "Copy line".
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

/// UTC, matching the terminal's feed. A denial is compared against a gateway
/// log, and both being in the same zone is what makes that possible.
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
