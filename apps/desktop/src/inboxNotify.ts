// An OS notification when something of yours on a tracker changes.
//
// **What counts as a change is decided in hura-core** (`tracker::changes`),
// by the same code the phone runs: a status that moved, somebody else's
// comment, a ticket that became yours or turned up in a filter, and on pull
// requests a review asked of you, failing checks, an approval or changes
// asked for. This keeps what was seen and says it.
//
// **Signed in to hura cloud, the service says what changed** instead, over a
// stream the Rust side keeps open, and each batch arrives as a
// `tracker-changes` event (see `onCloudChanges`). While that stream is open,
// what this window finds is recorded but not announced.
//
// **What was seen is kept on disk.** Not in memory, like `notify.ts` keeps
// session states: a ticket that moved while the window was closed has still
// moved, and finding out when the window opens is the point. The very first
// read is the baseline and announces nothing.

import { sendNotification } from "@tauri-apps/plugin-notification";

import { api } from "./api";
import type { Inbox } from "./gen/Inbox";
import type { InboxChange } from "./gen/InboxChange";
import { permitted } from "./notify";

const STORE = "hura.inbox.v1";
/// What the filters-era snapshot was kept under. Removed, since nothing reads
/// it any more and it would sit in the window's storage for good.
const OLD_STORE = "hura.tickets.v2";

/// Past this many changes in one read, one notification says how many rather
/// than a stack of them. The usual cause is the window opening after a
/// weekend, and ten toasts would be worse than one line.
const SUMMARISE_OVER = 3;

/// What a pull request is called in a notification: the repository's own name
/// and its number, since a number alone is ambiguous across repositories.
/// `pull_name` in hura-core, which names them in a change.
export function pullName(p: { repo: string; key: string }): string {
  const repo = p.repo.split("/").pop() ?? p.repo;
  return `${repo} ${p.key}`;
}

/// Record a read, and announce what changed since the last one.
///
/// **`on` gates the notification and not the bookkeeping**, for the reason
/// `onSessions` gives: a change seen while notifications were off must not be
/// announced when they come back on. Never rejects: a read that could not be
/// recorded is one whose changes are told next time, or not at all.
export async function onInbox(inbox: Inbox, on = true): Promise<InboxChange[]> {
  try {
    const { seen, changes } = await api.inboxChanges(inbox, load());
    save(seen);
    if (on && changes.length > 0 && !(await api.cloudLive())) void announce(changes);
    return changes;
  } catch {
    return [];
  }
}

/// What hura cloud found, as it found it: announced like what this window
/// finds, under the same switch.
export function onCloudChanges(changes: InboxChange[], on = true) {
  if (on && changes.length > 0) void announce(changes);
}

async function announce(changes: InboxChange[]) {
  if (!(await permitted())) return;
  const toasts =
    changes.length > SUMMARISE_OVER
      ? [
          {
            title: `${changes.length} changes in your tickets, filters and pull requests`,
            body: changes.map((c) => `${c.key} · ${c.what}`).join("\n"),
          },
        ]
      : changes.map((c) => ({ title: `${c.key} · ${c.what}`, body: c.title }));
  for (const toast of toasts) {
    try {
      sendNotification(toast);
    } catch {
      // Not worth a message in the window: the sidebar shows the same thing.
    }
  }
}

/// What was seen at the last read, as hura-core gave it. Not looked into
/// here: one that is not what hura-core wrote reads as nothing seen, which
/// costs one quiet poll.
function load(): unknown {
  try {
    window.localStorage.removeItem(OLD_STORE);
    const raw = window.localStorage.getItem(STORE);
    return raw === null ? null : JSON.parse(raw);
  } catch {
    return null;
  }
}

function save(seen: unknown) {
  try {
    window.localStorage.setItem(STORE, JSON.stringify(seen));
  } catch {
    // Storage full or refused: the next read compares with the last one that
    // did save, which errs towards telling you twice rather than never.
  }
}
