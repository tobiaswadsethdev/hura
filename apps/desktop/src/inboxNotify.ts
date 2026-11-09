// An OS notification when something of yours on a tracker changes.
//
// **What counts as a change is decided on hura cloud** (`tracker::changes` in
// hura-core): a status that moved, somebody else's comment, a ticket that
// became yours or turned up in a filter, and on pull requests a review asked
// of you, failing checks, an approval or changes asked for. The service reads
// the account's trackers all day and says what changed over a stream the Rust
// side keeps open while signed in; each batch arrives as a `tracker-changes`
// event, and this says it.

import { sendNotification } from "@tauri-apps/plugin-notification";

import type { InboxChange } from "./gen/InboxChange";
import { permitted } from "./notify";

/// Past this many changes in one batch, one notification says how many rather
/// than a stack of them: ten toasts would be worse than one line.
const SUMMARISE_OVER = 3;

/// What a pull request is called in a notification: the repository's own name
/// and its number, since a number alone is ambiguous across repositories.
/// `pull_name` in hura-core, which names them in a change.
export function pullName(p: { repo: string; key: string }): string {
  const repo = p.repo.split("/").pop() ?? p.repo;
  return `${repo} ${p.key}`;
}

/// What hura cloud found, as it found it. `on` is the window's switch for
/// these notifications.
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
