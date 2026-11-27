// Event kinds and the APNs payload built from them. The relay, not hurad,
// decides the category and interruption level.

import { clip } from "./util";

export const KINDS = {
  claude_needs_input: { category: "CLAUDE_INPUT", level: "active" },
  // A permission request the app can answer from the notification, with the
  // Allow and Deny actions its category registers. Carries an `ask_id`.
  claude_permission: { category: "CLAUDE_PERMISSION", level: "active" },
  claude_turn_finished: { category: "CLAUDE_TURN", level: "active" },
  claude_failed: { category: "CLAUDE_FAILED", level: "time-sensitive" },
  // Something of yours changed on a tracker, sent by hura cloud, which reads
  // the trackers of an account signed in to it. Its text names the ticket, so
  // it is sent encrypted.
  tracker_change: { category: "TRACKER_CHANGE", level: "active" },
  // Nothing to show: the inbox was looked at on another of the account's
  // devices, so the app wakes in the background to clear what it showed.
  // Sent as a background push, which carries no text.
  inbox_seen: { category: "", level: "passive" },
  test: { category: "TEST", level: "active" },
  // A session's Live Activity on the Lock Screen and in the Dynamic Island,
  // started, updated or ended with an ActivityKit push rather than shown as
  // an alert (docs/relay-api.md, "Live Activities").
  session_activity: { category: "", level: "passive" },
} as const;

export type Kind = keyof typeof KINDS;

/** Kinds sent as background pushes: no alert, no text, low priority. */
export const BACKGROUND: ReadonlySet<Kind> = new Set<Kind>(["inbox_seen"]);

// What an encrypted notification says until the app's extension decrypts it.
export const GENERIC_BODY: Record<Kind, string> = {
  claude_needs_input: "Claude needs your input",
  claude_permission: "Claude needs your permission",
  claude_turn_finished: "Claude finished a turn",
  claude_failed: "A session failed",
  tracker_change: "Something changed on your trackers",
  inbox_seen: "",
  test: "Test notification",
  session_activity: "",
};

// Either plaintext (title, subtitle, body) or `encrypted`, which only the
// phone can read (docs/relay-api.md, "Encrypted text").
export type Notification = {
  kind: Kind;
  session?: string;
  askId?: string;
} & ({ title: string; subtitle?: string; body: string; encrypted?: undefined } | { encrypted: string });

export function apnsPayload(n: Notification, link: { id: string; serverId: string; label: string }) {
  if (BACKGROUND.has(n.kind)) {
    return { aps: { "content-available": 1 }, hura: { kind: n.kind, server_id: link.serverId, link_id: link.id } };
  }
  const { category, level } = KINDS[n.kind];
  let alert: Record<string, string>;
  if (n.encrypted !== undefined) {
    alert = { title: clip(link.label, 100) || "hura", body: GENERIC_BODY[n.kind] };
  } else {
    alert = { title: clip(n.title, 100), body: clip(n.body, 500) };
    if (n.subtitle) alert.subtitle = clip(n.subtitle, 100);
  }
  const hura: Record<string, string> = { kind: n.kind, server_id: link.serverId, link_id: link.id };
  if (n.session) hura.session = n.session;
  if (n.askId !== undefined) hura.ask_id = n.askId;
  const aps: Record<string, unknown> = {
    alert,
    sound: "default",
    category,
    "thread-id": n.session ? `${link.serverId}:${n.session}` : link.serverId,
    "interruption-level": level,
  };
  if (n.encrypted !== undefined) {
    // Lets the app's notification service extension replace the text.
    aps["mutable-content"] = 1;
    hura.enc = n.encrypted;
  }
  return { aps, hura };
}

// --- Live Activities -----------------------------------------------------

export const PHASES = ["working", "permission", "input", "done", "failed", "ended"] as const;
export type Phase = (typeof PHASES)[number];

/** The phases that can start an activity when none is showing. */
export const STARTING: ReadonlySet<Phase> = new Set<Phase>(["working", "permission", "input"]);

/** What an activity's start says, since its real text may be encrypted. */
const START_BODY: Partial<Record<Phase, string>> = {
  working: "Claude is working",
  permission: "Claude needs your permission",
  input: "Claude needs your input",
};

export type ActivityEvent = "start" | "update" | "end";

export type SessionActivity = {
  session: string;
  phase: Phase;
  /** Unix seconds: when the session moved into the phase. */
  at: number;
  /** Unix seconds: when the turn began, which the phone counts from. */
  since?: number;
  askId?: string;
} & ({ title: string; subtitle?: string; body: string; encrypted?: undefined } | { encrypted: string });

export function activityPayload(
  a: SessionActivity,
  event: ActivityEvent,
  link: { id: string; serverId: string; label: string },
  now = Date.now(),
) {
  const state: Record<string, unknown> = { phase: a.phase, at: a.at };
  if (a.since !== undefined) state.since = a.since;
  if (a.askId !== undefined) state.ask_id = a.askId;
  if (a.encrypted !== undefined) {
    state.enc = a.encrypted;
  } else {
    state.title = clip(a.title, 100);
    if (a.subtitle) state.subtitle = clip(a.subtitle, 100);
    state.body = clip(a.body, 500);
  }
  const aps: Record<string, unknown> = { timestamp: a.at, event, "content-state": state };
  if (event === "start") {
    aps["attributes-type"] = "SessionActivityAttributes";
    aps.attributes = { server_id: link.serverId, link_id: link.id, session: a.session };
    // APNs wants an alert to start an activity; this one makes no sound.
    aps.alert = { title: clip(link.label, 100) || "hura", body: START_BODY[a.phase] ?? "" };
  } else if (event === "end") {
    // An activity only ends to go: a finished turn is an update, which the
    // relay ends later (docs/relay-api.md, "Live Activities").
    aps["dismissal-date"] = Math.floor(now / 1000);
  }
  return { aps };
}

// --- One activity for every session --------------------------------------

/** A session's row in a device's one activity: its latest event, and whose. */
export type AgentRow = SessionActivity & { serverId: string };

/** The most an activity's push may be, under APNs' 4 KB, with room to spare. */
const AGENTS_PAYLOAD_BYTES = 3800;

/**
 * The order T3 Code's activity lists its agents in: waiting on you first, then
 * failed, then working, then done; the newest first within each.
 */
function rank(phase: Phase): number {
  if (phase === "permission" || phase === "input") return 0;
  if (phase === "failed") return 1;
  if (phase === "working") return 2;
  return 3;
}

export function orderRows(rows: AgentRow[]): AgentRow[] {
  return [...rows].sort((a, b) => rank(a.phase) - rank(b.phase) || b.at - a.at);
}

function rowState(r: AgentRow): Record<string, unknown> {
  const s: Record<string, unknown> = { server_id: r.serverId, session: r.session, phase: r.phase, at: r.at };
  if (r.since !== undefined) s.since = r.since;
  if (r.askId !== undefined) s.ask_id = r.askId;
  if (r.encrypted !== undefined) {
    s.enc = r.encrypted;
  } else {
    s.title = clip(r.title, 100);
    if (r.subtitle) s.subtitle = clip(r.subtitle, 100);
    s.body = clip(r.body, 200);
  }
  return s;
}

/**
 * The push for a device's one activity: every session's row in T3 Code's
 * order, as many as fit, and how many there are of each kind, which count
 * them all whether or not they fit.
 */
export function agentsPayload(rows: AgentRow[], event: ActivityEvent, now = Date.now()) {
  const ordered = orderRows(rows);
  const state: Record<string, unknown> = {
    active: ordered.filter((r) => r.phase === "working" || r.phase === "permission" || r.phase === "input").length,
    waiting: ordered.filter((r) => r.phase === "permission" || r.phase === "input").length,
    failed: ordered.filter((r) => r.phase === "failed").length,
    rows: [] as Record<string, unknown>[],
  };
  const at = Math.floor(now / 1000);
  const aps: Record<string, unknown> = { timestamp: at, event, "content-state": state };
  if (event === "start") {
    aps["attributes-type"] = "AgentsActivityAttributes";
    aps.attributes = { kind: "agents" };
    const waiting = state.waiting as number;
    // APNs wants an alert to start an activity; this one makes no sound.
    aps.alert = {
      title: "hura",
      body: waiting > 0 ? (waiting === 1 ? "An agent needs you" : `${waiting} agents need you`) : "An agent is working",
    };
  } else if (event === "end") {
    aps["dismissal-date"] = at;
  }
  const list = state.rows as Record<string, unknown>[];
  for (const r of ordered) {
    list.push(rowState(r));
    if (JSON.stringify({ aps }).length > AGENTS_PAYLOAD_BYTES) {
      list.pop();
      break;
    }
  }
  return { aps };
}
