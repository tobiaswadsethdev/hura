// An OS notification when a ticket in one of your filters changes.
//
// Three kinds of change, and only three: its status moved, somebody else
// commented on it, or it turned up in a filter it was not in -- which is what
// "ready to start" is for. Anything else Jira counts as an update (a label, a
// sprint, a re-estimate) is left alone, because a notification for every edit
// is one nobody reads by the end of the week.
//
// **Compared with a snapshot kept on disk, per server.** Not in memory, like
// `notify.ts` keeps session states: a ticket that moved while the window was
// closed has still moved, and finding out when the window opens is the point.
// The very first read on a server is the baseline and announces nothing.
//
// **Only filters that were read take part.** A tracker that is down reads as
// an empty filter, and treating that as every ticket having left would make
// every one of them "new" when it comes back. `Inbox.read` is what says which
// filters actually answered; the rest keep the membership they had.

import { sendNotification } from "@tauri-apps/plugin-notification";

import type { Inbox } from "./gen/Inbox";
import type { Task } from "./gen/Task";
import { permitted } from "./notify";

/// What is remembered about one ticket.
type Seen = { key: string; title: string; status: string; comments: number | null };

type Snapshot = {
  /// By ticket, keyed `tracker \0 id`.
  tickets: Record<string, Seen>;
  /// By filter, keyed `tracker \0 filter`: the ticket ids it matched.
  members: Record<string, string[]>;
};

/// One change worth saying, about one ticket.
export type Change = { key: string; title: string; what: string };

const STORE = "hura.tickets.v1.";

/// Past this many changed tickets in one read, one notification says how many
/// rather than a stack of them -- the usual cause is the window opening after a
/// weekend, and ten toasts would be worse than one line.
const SUMMARISE_OVER = 3;

const ticketKey = (t: { tracker: string; id: string }) => `${t.tracker}\0${t.id}`;
const filterKey = (tracker: string, filter: string) => `${tracker}\0${filter}`;

/// Record a read of the tickets, and announce what changed since the last one.
///
/// Called with every read, whoever made it: the window's timer and the
/// tickets screen both. **`on` gates the notification and not the bookkeeping**,
/// for the reason `onSessions` gives: a change seen while notifications were
/// off must not be announced when they come back on.
export function onTickets(server: string, inbox: Inbox, on = true): Change[] {
  const before = load(server);
  const next = snapshot(before, inbox);
  save(server, next);
  if (before === null) return [];

  const changes = diff(before, inbox);
  if (on && changes.length > 0) void announce(changes);
  return changes;
}

/// What the snapshot becomes once this read is taken into account.
function snapshot(before: Snapshot | null, inbox: Inbox): Snapshot {
  const members: Record<string, string[]> = { ...(before?.members ?? {}) };
  for (const r of inbox.read) {
    members[filterKey(r.tracker, r.filter)] = inbox.tasks
      .filter((t) => t.tracker === r.tracker && t.filter === r.filter)
      .map((t) => t.id);
  }

  // Every ticket some filter still holds, the latest word on each. A ticket
  // no filter holds any more is forgotten, so the store does not grow with
  // every ticket that ever passed through.
  const held = new Set<string>();
  for (const [key, ids] of Object.entries(members)) {
    const tracker = key.split("\0")[0];
    for (const id of ids) held.add(`${tracker}\0${id}`);
  }
  const tickets: Record<string, Seen> = {};
  for (const [key, seen] of Object.entries(before?.tickets ?? {})) {
    if (held.has(key)) tickets[key] = seen;
  }
  for (const t of inbox.tasks) {
    tickets[ticketKey(t)] = { key: t.key, title: t.title, status: t.status, comments: t.comments };
  }
  return { tickets, members };
}

/// The changes between a snapshot and a read, one entry per ticket.
export function diff(before: Snapshot, inbox: Inbox): Change[] {
  const read = new Set(inbox.read.map((r) => filterKey(r.tracker, r.filter)));
  // A ticket two filters match is in `tasks` twice; its status and comments
  // are the same in both, so each ticket is judged once.
  const byTicket = new Map<string, { task: Task; what: string[] }>();
  const note = (task: Task, what: string) => {
    const entry = byTicket.get(ticketKey(task)) ?? { task, what: [] };
    if (!entry.what.includes(what)) entry.what.push(what);
    byTicket.set(ticketKey(task), entry);
  };

  for (const task of inbox.tasks) {
    const was = before.tickets[ticketKey(task)];
    if (was && was.status !== task.status) {
      note(task, `${was.status} → ${task.status}`);
    }
    // Only when both counts are known, and not for your own comment: you know
    // what you wrote.
    if (
      was &&
      was.comments !== null &&
      task.comments !== null &&
      task.comments > was.comments &&
      !task.last_comment_mine
    ) {
      note(task, task.last_commenter ? `${task.last_commenter} commented` : "new comment");
    }
    // New in a filter that was read now *and* last time. A filter seen for the
    // first time -- one just added -- is a baseline, not a list of arrivals.
    const fk = filterKey(task.tracker, task.filter);
    const previously = before.members[fk];
    if (read.has(fk) && previously && !previously.includes(task.id)) {
      note(task, `now in ${task.filter}`);
    }
  }

  return [...byTicket.values()].map(({ task, what }) => ({
    key: task.key,
    title: task.title,
    what: what.join(", "),
  }));
}

async function announce(changes: Change[]) {
  if (!(await permitted())) return;
  const toasts =
    changes.length > SUMMARISE_OVER
      ? [
          {
            title: `${changes.length} tickets changed`,
            body: changes.map((c) => `${c.key} · ${c.what}`).join("\n"),
          },
        ]
      : changes.map((c) => ({ title: `${c.key} · ${c.what}`, body: c.title }));
  for (const toast of toasts) {
    try {
      sendNotification(toast);
    } catch {
      // Not worth a message in the window: the tickets screen shows the same thing.
    }
  }
}

function load(server: string): Snapshot | null {
  try {
    const raw = window.localStorage.getItem(STORE + server);
    if (raw === null) return null;
    const parsed = JSON.parse(raw) as Partial<Snapshot>;
    if (typeof parsed !== "object" || parsed === null) return null;
    return { tickets: parsed.tickets ?? {}, members: parsed.members ?? {} };
  } catch {
    // A snapshot that will not parse is one written by something else; the
    // next read is a fresh baseline, which costs one quiet poll.
    return null;
  }
}

function save(server: string, snapshot: Snapshot) {
  try {
    window.localStorage.setItem(STORE + server, JSON.stringify(snapshot));
  } catch {
    // Storage full or refused: the next read compares with the last one that
    // did save, which errs towards telling you twice rather than never.
  }
}
