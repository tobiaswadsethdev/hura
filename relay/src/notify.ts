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
  test: { category: "TEST", level: "active" },
} as const;

export type Kind = keyof typeof KINDS;

// What an encrypted notification says until the app's extension decrypts it.
export const GENERIC_BODY: Record<Kind, string> = {
  claude_needs_input: "Claude needs your input",
  claude_permission: "Claude needs your permission",
  claude_turn_finished: "Claude finished a turn",
  claude_failed: "A session failed",
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
