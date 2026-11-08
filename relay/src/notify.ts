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
