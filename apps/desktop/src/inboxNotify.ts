// An OS notification when something of yours on a tracker changes.
//
// Tickets, three ways: its status moved, somebody else commented on it, or it
// became yours (assigned to you, where it was not). Anything else Jira counts
// as an update (a label, a sprint, a re-estimate) is left alone, because a
// notification for every edit is one nobody reads by the end of the week.
//
// Pull requests, four ways: a review was asked of you or one was assigned to
// you, and on yours, the checks failed, it was approved, or changes were asked
// for. Those are the four that are a reason to stop and look.
//
// **Compared with a snapshot kept on disk.** Not in memory, like `notify.ts`
// keeps session states: a ticket that moved while the window was closed has
// still moved, and finding out when the window opens is the point. The very
// first read is the baseline and announces nothing, and so is the first read
// of a tracker added since.
//
// **Only trackers that were read take part.** A tracker that is down reads as
// an empty list, and treating that as everything having left would make all
// of it "new" when the tracker comes back. `Inbox.read` and `pulls_read` say
// which trackers actually answered; the rest keep what they had.

import { sendNotification } from "@tauri-apps/plugin-notification";

import type { Inbox } from "./gen/Inbox";
import type { PullChecks } from "./gen/PullChecks";
import type { PullRequest } from "./gen/PullRequest";
import type { PullReview } from "./gen/PullReview";
import type { Relation } from "./gen/Relation";
import type { Task } from "./gen/Task";
import { permitted } from "./notify";

/// What is remembered about one ticket.
type SeenTicket = {
  tracker: string;
  key: string;
  title: string;
  status: string;
  comments: number | null;
  relation: Relation;
};

/// What is remembered about one pull request.
type SeenPull = {
  tracker: string;
  key: string;
  title: string;
  checks: PullChecks | null;
  review: PullReview | null;
  relation: Relation;
};

type Snapshot = {
  /// By ticket, keyed `tracker \0 id`.
  tickets: Record<string, SeenTicket>;
  /// By pull request, keyed `tracker \0 repo \0 id`.
  pulls: Record<string, SeenPull>;
  /// The trackers read at least once, each half on its own: a tracker's
  /// first answer is a baseline, not a list of arrivals.
  read: string[];
  pullsRead: string[];
};

/// One change worth saying, about one ticket or pull request.
export type Change = { key: string; title: string; what: string };

const STORE = "hura.inbox.v1";
/// What the filters-era snapshot was kept under. Removed, since nothing reads
/// it any more and it would sit in the window's storage for good.
const OLD_STORE = "hura.tickets.v2";

/// Past this many changes in one read, one notification says how many rather
/// than a stack of them. The usual cause is the window opening after a
/// weekend, and ten toasts would be worse than one line.
const SUMMARISE_OVER = 3;

const ticketKey = (t: { tracker: string; id: string }) => `${t.tracker}\0${t.id}`;
const pullKey = (p: { tracker: string; repo: string; id: string }) =>
  `${p.tracker}\0${p.repo}\0${p.id}`;

/// What a pull request is called in a notification: the repository's own name
/// and its number, since a number alone is ambiguous across repositories.
export function pullName(p: { repo: string; key: string }): string {
  const repo = p.repo.split("/").pop() ?? p.repo;
  return `${repo} ${p.key}`;
}

/// Record a read, and announce what changed since the last one.
///
/// **`on` gates the notification and not the bookkeeping**, for the reason
/// `onSessions` gives: a change seen while notifications were off must not be
/// announced when they come back on.
export function onInbox(inbox: Inbox, on = true): Change[] {
  const before = load();
  save(snapshot(before, inbox));
  if (before === null) return [];
  const changes = diff(before, inbox);
  if (on && changes.length > 0) void announce(changes);
  return changes;
}

/// What the snapshot becomes once this read is taken into account.
///
/// A tracker that was read is replaced wholesale, so what left it is
/// forgotten and the store does not grow with everything that ever passed
/// through. A tracker that was not read keeps what it had.
function snapshot(before: Snapshot | null, inbox: Inbox): Snapshot {
  const read = new Set(inbox.read);
  const pullsRead = new Set(inbox.pulls_read);

  const tickets: Record<string, SeenTicket> = {};
  for (const [key, seen] of Object.entries(before?.tickets ?? {})) {
    if (!read.has(seen.tracker)) tickets[key] = seen;
  }
  for (const t of inbox.tasks) {
    tickets[ticketKey(t)] = {
      tracker: t.tracker,
      key: t.key,
      title: t.title,
      status: t.status,
      comments: t.comments,
      relation: t.relation,
    };
  }

  const pulls: Record<string, SeenPull> = {};
  for (const [key, seen] of Object.entries(before?.pulls ?? {})) {
    if (!pullsRead.has(seen.tracker)) pulls[key] = seen;
  }
  for (const p of inbox.pulls) {
    pulls[pullKey(p)] = {
      tracker: p.tracker,
      key: pullName(p),
      title: p.title,
      checks: p.checks,
      review: p.review,
      relation: p.relation,
    };
  }

  return {
    tickets,
    pulls,
    read: [...new Set([...(before?.read ?? []), ...inbox.read])],
    pullsRead: [...new Set([...(before?.pullsRead ?? []), ...inbox.pulls_read])],
  };
}

/// The changes between a snapshot and a read, one entry per ticket or pull
/// request.
export function diff(before: Snapshot, inbox: Inbox): Change[] {
  const known = new Set(before.read);
  const knownPulls = new Set(before.pullsRead);
  const out: Change[] = [];

  for (const task of inbox.tasks) {
    // A tracker answering for the first time is a baseline.
    if (!known.has(task.tracker)) continue;
    const what = ticketChanges(before.tickets[ticketKey(task)], task);
    if (what.length > 0) out.push({ key: task.key, title: task.title, what: what.join(", ") });
  }

  for (const pull of inbox.pulls) {
    if (!knownPulls.has(pull.tracker)) continue;
    const what = pullChanges(before.pulls[pullKey(pull)], pull);
    if (what.length > 0) out.push({ key: pullName(pull), title: pull.title, what: what.join(", ") });
  }

  return out;
}

function ticketChanges(was: SeenTicket | undefined, task: Task): string[] {
  const what: string[] = [];
  // Yours now, and not before. One you created turning up is not news: you
  // know you made it.
  if (task.relation === "assigned" && was?.relation !== "assigned") {
    what.push("assigned to you");
  }
  if (!was) return what;
  if (was.status !== task.status) what.push(`${was.status} → ${task.status}`);
  // Only when both counts are known, and not for your own comment: you know
  // what you wrote.
  if (
    was.comments !== null &&
    task.comments !== null &&
    task.comments > was.comments &&
    !task.last_comment_mine
  ) {
    what.push(task.last_commenter ? `${task.last_commenter} commented` : "new comment");
  }
  return what;
}

function pullChanges(was: SeenPull | undefined, pull: PullRequest): string[] {
  if (pull.relation === "assigned") {
    // Waiting on you now, and not before. Once you have reviewed it, it
    // leaves the list, so the next request for a review is news again.
    if (was?.relation === "assigned") return [];
    return [pull.review_requested ? "review requested" : "assigned to you"];
  }
  // Yours: only what came back from somebody else, or from the checks.
  if (!was) return [];
  const what: string[] = [];
  if (pull.checks === "failing" && was.checks !== "failing") what.push("checks failed");
  if (pull.review !== was.review) {
    if (pull.review === "approved") what.push("approved");
    if (pull.review === "changes-requested") what.push("changes requested");
  }
  return what;
}

async function announce(changes: Change[]) {
  if (!(await permitted())) return;
  const toasts =
    changes.length > SUMMARISE_OVER
      ? [
          {
            title: `${changes.length} changes in your tickets and pull requests`,
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

function load(): Snapshot | null {
  try {
    window.localStorage.removeItem(OLD_STORE);
    const raw = window.localStorage.getItem(STORE);
    if (raw === null) return null;
    const parsed = JSON.parse(raw) as Partial<Snapshot>;
    if (typeof parsed !== "object" || parsed === null) return null;
    return {
      tickets: parsed.tickets ?? {},
      pulls: parsed.pulls ?? {},
      read: Array.isArray(parsed.read) ? parsed.read : [],
      pullsRead: Array.isArray(parsed.pullsRead) ? parsed.pullsRead : [],
    };
  } catch {
    // A snapshot that will not parse is one written by something else; the
    // next read is a fresh baseline, which costs one quiet poll.
    return null;
  }
}

function save(snapshot: Snapshot) {
  try {
    window.localStorage.setItem(STORE, JSON.stringify(snapshot));
  } catch {
    // Storage full or refused: the next read compares with the last one that
    // did save, which errs towards telling you twice rather than never.
  }
}
