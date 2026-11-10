// The sidebar on the far right: your tickets and your pull requests.
//
// Always there, whatever worktree is selected and whatever screen is open,
// because the question it answers ("what is mine right now?") is not about the
// worktree in front of you. Every tracker is asked the same thing, what is
// assigned to you or created by you, so that part means the same thing
// whichever tracker it reads.
//
// **Two views, two groups each.** Tickets assigned to you and tickets you
// created; pull requests waiting on you (a review asked of you, or assigned
// to you) and pull requests you opened. A row in both groups is in the first,
// because that is the half that asks something of you. A Jira tracker's JQL
// filters are a group each, between the two, holding every ticket they match,
// yours included; and a tracker can leave out what you created, so with none
// showing it the group is gone. The header switches
// between tickets and pull requests rather than stacking them, so neither is
// scrolled past to reach the other.
//
// **A row knows its worktree.** A ticket a session was started from, or a pull
// request whose branch a worktree is working on, shows that worktree and its
// agent's state, and selects it on a click. A row with none has the start
// button instead, and starting is the create form, filled in. A pull request
// starts in a project that is a clone of its repository, on its own branch,
// either to work on it or to review it.
//
// **Folded, it is a rail of two counts**, because four columns do not fit a
// laptop: tree, middle, dock and this leave little of the middle at 1280
// pixels. Each count unfolds it to its own view. The shortcut that used to open the tickets screen folds and unfolds
// it.

import { useMemo } from "react";

import { useButtonMenu, useContextMenu, copy, type MenuItem } from "./ContextMenu";
import { Empty } from "./Empty";
import type { ConfiguredTracker } from "./gen/ConfiguredTracker";
import type { Inbox } from "./gen/Inbox";
import type { Project } from "./gen/Project";
import type { PullRequest as Pull } from "./gen/PullRequest";
import type { Relation } from "./gen/Relation";
import type { Session } from "./gen/Session";
import type { Task } from "./gen/Task";
import type { InboxView } from "./prefs";
import type { PullStart } from "./Pull";
import {
  Chevron,
  Comments,
  Copy,
  Elsewhere,
  Failing,
  Fold,
  Passing,
  Pending,
  PullDraft,
  PullRequest,
  Refresh,
  Review,
  Start,
  StateDot,
  Tracker,
  Unfold,
  Working,
} from "./icons";
import { pullName } from "./inboxNotify";
import { openExternal } from "./open";
import { ago, when } from "./Ticket";

/// The groups, in the order they are drawn, and what each is called.
const GROUPS: { key: string; relation: Relation; label: string }[] = [
  { key: "assigned", relation: "assigned", label: "Assigned to me" },
  { key: "created", relation: "created", label: "Created by me" },
];

/// The tickets' groups: yours, then each JQL filter by name in the order the
/// trackers list them (two trackers' filters of one name are one group), then
/// what you created -- when a tracker still shows it *and* there is something
/// in it. Hiding it on one tracker left the group on screen, empty, whenever
/// another tracker (a GitHub one with no issues of yours, say) still had it on;
/// an empty "assigned to me" is news, an empty "created by me" is not.
function ticketGroups(
  trackers: ConfiguredTracker[] | null,
  tasks: Task[],
): { key: string; label: string; holds: (t: Task) => boolean }[] {
  const filters = [
    ...new Set((trackers ?? []).flatMap((t) => t.source.jql_filters.map((f) => f.name))),
  ];
  const created =
    (trackers === null || trackers.some((t) => !t.source.hide_created)) &&
    tasks.some((t) => t.relation === "created");
  return [
    { key: "assigned", label: "Assigned to me", holds: (t) => t.relation === "assigned" },
    ...filters.map((name) => ({
      key: `filter:${name}`,
      label: name,
      holds: (t: Task) => t.filters.includes(name),
    })),
    ...(created
      ? [{ key: "created", label: "Created by me", holds: (t: Task) => t.relation === "created" }]
      : []),
  ];
}

export function InboxBar({
  width,
  open,
  shut,
  view,
  inbox,
  reading,
  error,
  trackers,
  signedIn,
  sessions,
  projects,
  currentProject,
  showing,
  showingPull,
  keys,
  onFold,
  onShut,
  onView,
  onRefresh,
  onSetup,
  onSelect,
  onShow,
  onShowPull,
  onStart,
  onStartPull,
}: {
  /// How wide, in pixels, when open. From the window's own preferences and
  /// written by the handle on this sidebar's left edge.
  width: number;
  /// Open, or folded to its rail.
  open: boolean;
  /// The groups that are shut, by `tickets:assigned`-style key.
  shut: string[];
  /// Tickets or pull requests: the one being shown.
  view: InboxView;
  /// The last read. `null` before the first one has answered.
  inbox: Inbox | null;
  /// A read is in flight.
  reading: boolean;
  /// Why the last read could not be made at all: hura cloud did not answer,
  /// say. A tracker that could not be read is a warning in `inbox`.
  error: string | null;
  /// The hura cloud account's trackers. `null` until they have been read.
  trackers: ConfiguredTracker[] | null;
  /// Whether this computer is signed in to hura cloud, which trackers need.
  /// `null` until that is known.
  signedIn: boolean | null;
  sessions: Session[];
  projects: Project[];
  /// The selected worktree's project, which is the likeliest answer to where
  /// a ticket should be started, so it is first on the start menu.
  currentProject: string | null;
  /// The ticket or pull request open beside the sidebar, if any.
  showing: Task | null;
  showingPull: Pull | null;
  /// The shortcut that folds and unfolds this, for the tooltips.
  keys: { label: string; aria: string };
  onFold: () => void;
  onShut: (shut: string[]) => void;
  /// Show the other one. Also what a count on the folded rail calls, before
  /// unfolding.
  onView: (view: InboxView) => void;
  onRefresh: () => void;
  onSetup: () => void;
  onSelect: (session: string) => void;
  /// Read a Jira ticket beside the sidebar.
  onShow: (task: Task) => void;
  /// Read a pull request beside the sidebar.
  onShowPull: (pull: Pull) => void;
  onStart: (project: Project, task: Task) => void;
  onStartPull: (project: Project, pull: Pull, mode: PullStart) => void;
}) {
  const tasks = useMemo(() => newestFirst(inbox?.tasks ?? []), [inbox]);
  const pulls = useMemo(() => newestFirst(inbox?.pulls ?? []), [inbox]);

  if (!open) {
    return (
      <Rail
        view={view}
        tasks={tasks}
        pulls={pulls}
        warned={(inbox?.warnings.length ?? 0) > 0 || error !== null}
        keys={keys}
        onUnfold={(to) => {
          onView(to);
          onFold();
        }}
      />
    );
  }

  const none = trackers !== null && trackers.length === 0;
  // Pull requests come from GitHub and Azure DevOps. With only Jira or Fizzy
  // set up, their view says where they would come from rather than sitting
  // empty.
  const forges =
    trackers?.some((t) => t.source.kind === "git-hub" || t.source.kind === "azure-dev-ops") ?? false;
  const isShut = (key: string) => shut.includes(key);
  const toggle = (key: string) =>
    onShut(isShut(key) ? shut.filter((k) => k !== key) : [...shut, key]);

  return (
    <aside className="inbox" style={{ width }} aria-label="Your tickets and pull requests">
      <header className="inbox-head">
        <div className="segmented inbox-views" role="tablist" aria-label="Mine">
          {(
            [
              ["tickets", Tracker, "Tickets", tasks.length],
              ["pulls", PullRequest, "Pull Requests", pulls.length],
            ] as const
          ).map(([v, Mark, label, count]) => (
            <button
              key={v}
              role="tab"
              aria-selected={view === v}
              aria-label={inbox === null ? label : `${label}, ${count}`}
              title={label}
              className={view === v ? "on" : ""}
              onClick={() => onView(v)}
            >
              <Mark />
              <span className="inbox-views-label">{label}</span>
              {inbox !== null && <span className="inbox-views-count">{count}</span>}
            </button>
          ))}
        </div>
        {reading && <Working className="inbox-reading" />}
        <span className="inbox-tools">
          <button
            className="quiet-icon"
            title="Read Them Again"
            disabled={reading || none}
            onClick={onRefresh}
          >
            <Refresh aria-label="Refresh" />
          </button>
          <button
            className="quiet-icon"
            title={`Fold This Sidebar (${keys.label})`}
            aria-keyshortcuts={keys.aria}
            onClick={onFold}
          >
            <Fold aria-label="Fold" />
          </button>
        </span>
      </header>

      <div className="inbox-body">
        {(error || (inbox?.warnings.length ?? 0) > 0) && (
          <div className="inbox-notes">
            {error && <p className="error">{error}</p>}
            {/* A tracker that could not be read is said out loud rather than
                leaving its rows quietly missing, which looks exactly like
                having nothing assigned. */}
            {inbox?.warnings.map((w) => (
              <p key={w} className="warn">
                {w}
              </p>
            ))}
          </div>
        )}

        {signedIn === false ? (
          <Empty icon={Tracker} note="Trackers need a hura cloud account">
            <button className="go" onClick={onSetup}>
              Sign In
            </button>
          </Empty>
        ) : none ? (
          <Empty icon={Tracker} note="No trackers yet">
            <button className="go" onClick={onSetup}>
              Add a Tracker
            </button>
          </Empty>
        ) : view === "tickets" ? (
          ticketGroups(trackers, tasks).map((g) => {
            const key = `tickets:${g.key}`;
            const rows = tasks.filter(g.holds);
            return (
              <Group
                key={key}
                label={g.label}
                count={rows.length}
                open={!isShut(key)}
                loading={inbox === null}
                onToggle={() => toggle(key)}
              >
                {rows.map((t) => (
                  <TicketRow
                    key={`${t.tracker}:${t.id}`}
                    task={t}
                    session={ticketSession(t, sessions)}
                    projects={projects}
                    currentProject={currentProject}
                    on={showing?.tracker === t.tracker && showing.key === t.key}
                    onSelect={onSelect}
                    onShow={onShow}
                    onStart={onStart}
                  />
                ))}
              </Group>
            );
          })
        ) : !forges && trackers !== null ? (
          <p className="inbox-hint">
            Pull requests come from GitHub and Azure DevOps trackers.{" "}
            <button className="link" onClick={onSetup}>
              Add One
            </button>
          </p>
        ) : (
          GROUPS.map((g) => {
            const key = `pulls:${g.key}`;
            const rows = pulls.filter((p) => p.relation === g.relation);
            return (
              <Group
                key={key}
                label={g.relation === "assigned" ? "Waiting on me" : g.label}
                count={rows.length}
                open={!isShut(key)}
                loading={inbox === null}
                onToggle={() => toggle(key)}
              >
                {rows.map((p) => (
                  <PullRow
                    key={`${p.tracker}:${p.repo}:${p.id}`}
                    pull={p}
                    session={pullSession(p, sessions)}
                    projects={pullProjects(p, projects)}
                    on={samePull(showingPull, p)}
                    onSelect={onSelect}
                    onShow={onShowPull}
                    onStart={onStartPull}
                  />
                ))}
              </Group>
            );
          })
        )}
      </div>
    </aside>
  );
}

/// Folded: the two counts, and the way back.
function Rail({
  view,
  tasks,
  pulls,
  warned,
  keys,
  onUnfold,
}: {
  view: InboxView;
  tasks: Task[];
  pulls: Pull[];
  warned: boolean;
  keys: { label: string; aria: string };
  onUnfold: (view: InboxView) => void;
}) {
  const split = <T extends { relation: Relation }>(rows: T[], mine: string, theirs: string) => {
    const assigned = rows.filter((r) => r.relation === "assigned").length;
    const matched = rows.filter((r) => r.relation === "matched").length;
    const rest = `${assigned} ${theirs}, ${rows.length - assigned - matched} ${mine}`;
    return matched > 0 ? `${rest}, ${matched} more from filters` : rest;
  };
  return (
    <aside className="inbox-rail" aria-label="Your tickets and pull requests, folded">
      <button
        className="quiet-icon"
        title={`Open This Sidebar (${keys.label})`}
        aria-keyshortcuts={keys.aria}
        onClick={() => onUnfold(view)}
      >
        <Unfold aria-label="Open" />
      </button>
      <button
        className="inbox-rail-count"
        title={`Tickets: ${split(tasks, "created by you", "assigned to you")}`}
        onClick={() => onUnfold("tickets")}
      >
        <Tracker />
        <span>{tasks.length}</span>
      </button>
      <button
        className="inbox-rail-count"
        title={`Pull requests: ${split(pulls, "yours", "waiting on you")}`}
        onClick={() => onUnfold("pulls")}
      >
        <PullRequest />
        <span>{pulls.length}</span>
      </button>
      {warned && (
        <span className="inbox-rail-warn" title="A tracker could not be read; open the sidebar to see which" />
      )}
    </aside>
  );
}

/// One group, shut by a click on its name.
function Group({
  label,
  count,
  open,
  loading,
  onToggle,
  children,
}: {
  label: string;
  count: number;
  open: boolean;
  /// Nothing has been read yet, which is not the same as nothing there.
  loading: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  return (
    <section className="inbox-group">
      <button className="inbox-group-head" aria-expanded={open} onClick={onToggle}>
        <Chevron open={open} />
        <span>{label}</span>
        <span className="inbox-count">{loading ? "" : count}</span>
      </button>
      {open && count > 0 && <ul className="inbox-rows">{children}</ul>}
      {open && count === 0 && !loading && <p className="inbox-none">Nothing</p>}
    </section>
  );
}

function TicketRow({
  task,
  session,
  projects,
  currentProject,
  on,
  onSelect,
  onShow,
  onStart,
}: {
  task: Task;
  session: Session | null;
  projects: Project[];
  currentProject: string | null;
  on: boolean;
  onSelect: (session: string) => void;
  onShow: (task: Task) => void;
  onStart: (project: Project, task: Task) => void;
}) {
  const menu = useContextMenu();
  const startMenu = useButtonMenu();
  // A Jira ticket is read here; the others are still read in the browser.
  const readsHere = task.kind === "jira";
  const open = () => (readsHere ? onShow(task) : openExternal(task.url));
  // The selected worktree's project first: that is where a ticket picked
  // while looking at it most likely belongs.
  const ordered = [...projects].sort(
    (a, b) => Number(b.name === currentProject) - Number(a.name === currentProject),
  );
  const starts: MenuItem[] = ordered.map((p) => ({
    label: `Start in ${p.name}`,
    icon: Start,
    run: () => onStart(p, task),
  }));
  const menuProps = menu(() => [
    ...(readsHere ? [{ label: "Open", run: () => onShow(task) }] : []),
    { label: "Open in the Browser", icon: Elsewhere, run: () => openExternal(task.url) },
    ...(session ? [{ label: `Go to ${session.title ?? session.name}`, run: () => onSelect(session.name) }] : []),
    ...(starts.length > 0 ? (["separator", ...starts] as MenuItem[]) : []),
    "separator",
    { label: "Copy Key", icon: Copy, hint: task.key, run: () => copy(task.key) },
    { label: "Copy Link", icon: Copy, run: () => copy(task.url) },
  ]);
  const updated = ago(task.updated);
  const comments = task.comments ?? 0;

  return (
    <li
      className={`inbox-row${on ? " on" : ""}`}
      tabIndex={0}
      aria-current={on ? "true" : undefined}
      title={`${task.key} ${task.title}\n${task.tracker}`}
      onClick={(e) => {
        if ((e.target as HTMLElement).closest("button")) return;
        open();
      }}
      onContextMenu={menuProps.onContextMenu}
      onKeyDown={(e) => {
        if (e.key === "Enter" && e.target === e.currentTarget) open();
        else menuProps.onKeyDown(e);
      }}
    >
      <div className="inbox-row-top">
        <span className="inbox-key">{task.key}</span>
        {!readsHere && <Elsewhere className="inbox-out" aria-label="Opens in the browser" />}
        {task.item_type && <span className="inbox-type">{task.item_type}</span>}
        <span className="inbox-status">{task.status}</span>
      </div>
      <p className="inbox-row-title">{task.title}</p>
      <div className="inbox-meta">
        {updated && <span title={task.updated ?? undefined}>{updated}</span>}
        {comments > 0 && (
          <span
            className="inbox-comments"
            title={task.last_commenter ? `Last comment by ${task.last_commenter}` : undefined}
          >
            <Comments /> {comments}
            {task.last_commenter && !task.last_comment_mine && (
              <span className="inbox-commenter"> · {task.last_commenter}</span>
            )}
          </span>
        )}
        {session ? (
          <Worktree session={session} onSelect={onSelect} />
        ) : (
          <button
            className="quiet-icon inbox-start"
            disabled={projects.length === 0}
            title={
              projects.length === 0
                ? "Make a project first"
                : projects.length === 1
                  ? `Start a Worktree for It in ${projects[0]!.name}`
                  : "Start a Worktree for It"
            }
            onClick={(e) => {
              if (projects.length === 1) onStart(projects[0]!, task);
              else startMenu(() => starts)(e);
            }}
          >
            <Start aria-label="Start" />
          </button>
        )}
      </div>
    </li>
  );
}

function PullRow({
  pull,
  session,
  projects,
  on,
  onSelect,
  onShow,
  onStart,
}: {
  pull: Pull;
  session: Session | null;
  /// The projects that are clones of its repository.
  projects: Project[];
  on: boolean;
  onSelect: (session: string) => void;
  onShow: (pull: Pull) => void;
  onStart: (project: Project, pull: Pull, mode: PullStart) => void;
}) {
  const menu = useContextMenu();
  const startMenu = useButtonMenu();
  // Two ways in each project: work on it, or review it.
  const starts: MenuItem[] = pull.fork
    ? []
    : projects.flatMap((p) => [
        {
          label: projects.length > 1 ? `Work on It in ${p.name}` : "Work on It",
          icon: Start,
          run: () => onStart(p, pull, "work"),
        },
        {
          label: projects.length > 1 ? `Review It in ${p.name}` : "Review It",
          icon: Review,
          run: () => onStart(p, pull, "review"),
        },
      ]);
  const menuProps = menu(() => [
    { label: "Open", run: () => onShow(pull) },
    { label: "Open in the Browser", icon: Elsewhere, run: () => openExternal(pull.url) },
    ...(session ? [{ label: `Go to ${session.title ?? session.name}`, run: () => onSelect(session.name) }] : []),
    ...(starts.length > 0 && !session ? (["separator", ...starts] as MenuItem[]) : []),
    "separator",
    { label: "Copy Branch", icon: Copy, hint: pull.branch, run: () => copy(pull.branch) },
    { label: "Copy Link", icon: Copy, run: () => copy(pull.url) },
  ]);
  const updated = ago(pull.updated);
  const Mark = pull.draft ? PullDraft : PullRequest;
  const open = () => onShow(pull);

  return (
    <li
      className={`inbox-row pull${on ? " on" : ""}`}
      tabIndex={0}
      aria-current={on ? "true" : undefined}
      title={`${pullName(pull)} ${pull.title}\n${pull.branch} → ${pull.base}`}
      onClick={(e) => {
        if ((e.target as HTMLElement).closest("button")) return;
        open();
      }}
      onContextMenu={menuProps.onContextMenu}
      onKeyDown={(e) => {
        if (e.key === "Enter" && e.target === e.currentTarget) open();
        else menuProps.onKeyDown(e);
      }}
    >
      <div className="inbox-row-top">
        <Mark className={`inbox-pull-mark${pull.draft ? " draft" : ""}`} aria-label={pull.draft ? "Draft" : "Pull request"} />
        <span className="inbox-key">{pullName(pull)}</span>
        <span className="inbox-pull-state">
          <ChecksMark checks={pull.checks} />
          <ReviewWord pull={pull} />
        </span>
      </div>
      <p className="inbox-row-title">{pull.title}</p>
      <div className="inbox-meta">
        {updated && (
          <span title={pull.updated ?? undefined}>
            {pull.kind === "azure-dev-ops" ? `opened ${updated}` : updated}
          </span>
        )}
        {/* Who is asking, on the ones waiting on you. Yours, you know. */}
        {pull.relation === "assigned" && pull.author && <span>{pull.author}</span>}
        {pull.draft && <span>draft</span>}
        {pull.conflicts && <span className="inbox-conflicts">conflicts</span>}
        {(pull.comments ?? 0) > 0 && (
          <span className="inbox-comments">
            <Comments /> {pull.comments}
          </span>
        )}
        {pull.fork && <span title="Opened from a fork">fork</span>}
        {session ? (
          <Worktree session={session} onSelect={onSelect} />
        ) : (
          !pull.fork && (
            <button
              className="quiet-icon inbox-start"
              disabled={projects.length === 0}
              title={
                projects.length === 0
                  ? `No project is a clone of ${pull.repo}`
                  : "Start a Worktree on Its Branch"
              }
              onClick={(e) => startMenu(() => starts)(e)}
            >
              <Start aria-label="Start" />
            </button>
          )
        )}
      </div>
    </li>
  );
}

/// The checks as one mark. Nothing at all when the forge does not say, rather
/// than a mark that reads as "none ran".
export function ChecksMark({ checks }: { checks: Pull["checks"] }) {
  switch (checks) {
    case "passing":
      return <Passing className="inbox-checks ok" aria-label="Checks passed" />;
    case "failing":
      return <Failing className="inbox-checks bad" aria-label="Checks failed" />;
    case "pending":
      return <Pending className="inbox-checks running" aria-label="Checks running" />;
    default:
      return null;
  }
}

/// Where the reviews stand, in a word, on the ones you opened. On the ones
/// waiting on you the answer is you.
function ReviewWord({ pull }: { pull: Pull }) {
  if (pull.relation === "assigned") {
    return <span className="inbox-review">{pull.review_requested ? "review" : "assigned"}</span>;
  }
  switch (pull.review) {
    case "approved":
      return <span className="inbox-review ok">approved</span>;
    case "changes-requested":
      return <span className="inbox-review bad">changes</span>;
    case "pending":
      return <span className="inbox-review">in review</span>;
    default:
      return null;
  }
}

/// The worktree a row belongs to, as its agent's state and its name. A
/// button, because going there is what you do with it.
function Worktree({ session, onSelect }: { session: Session; onSelect: (name: string) => void }) {
  return (
    <button
      className="inbox-worktree"
      title={`Go to ${session.title ?? session.name}`}
      onClick={() => onSelect(session.name)}
    >
      <StateDot state={session.state} />
      <span>{session.title ?? session.name}</span>
    </button>
  );
}

/// Newest first, by the tracker's own clock. Rows with no time go last.
function newestFirst<T extends { updated: string | null }>(rows: T[]): T[] {
  return [...rows].sort((a, b) => (when(b.updated) ?? 0) - (when(a.updated) ?? 0));
}

/// A Jira key, which a branch named after a ticket starts its last segment
/// with: `tobias/CODE-123-the-title`.
const JIRA_KEY = /^[A-Z][A-Z0-9_]+-\d+$/;

/// The worktree a ticket was started from, or failing that, one whose branch
/// is named after it. The second is how a branch made outside this window,
/// by the same convention, still finds its ticket.
export function ticketSession(task: Task, sessions: Session[]): Session | null {
  const live = sessions.filter((s) => s.state !== "dead");
  const started = live.find((s) => s.ticket?.tracker === task.tracker && s.ticket.key === task.key);
  if (started) return started;
  if (!JIRA_KEY.test(task.key)) return null;
  return (
    live.find((s) => {
      const last = s.work_branch.split("/").pop() ?? "";
      return last === task.key || last.startsWith(`${task.key}-`);
    }) ?? null
  );
}

/// The worktree a pull request came from: the same branch, in the same
/// repository.
export function pullSession(pull: Pull, sessions: Session[]): Session | null {
  const live = sessions.filter((s) => s.state !== "dead");
  return (
    live.find((s) => startedFrom(s, pull)) ??
    live.find((s) => onBranchOf(s, pull)) ??
    null
  );
}

/// The open pull request a worktree is on: the one it was started from, or
/// one whose branch it is working on in the same repository. Only the ones
/// the sidebar knows of, which are yours and the ones waiting on you.
export function sessionPull(session: Session, pulls: Pull[]): Pull | null {
  return (
    pulls.find((p) => startedFrom(session, p)) ??
    pulls.find((p) => onBranchOf(session, p)) ??
    null
  );
}

/// The projects a pull request can start in: the clones of its repository.
export function pullProjects(pull: Pull, projects: Project[]): Project[] {
  const repo = bare(pull.repo_url);
  return projects.filter((p) => bare(p.repo) === repo);
}

export function samePull(a: Pull | null, b: Pull): boolean {
  return a !== null && a.tracker === b.tracker && a.repo === b.repo && a.id === b.id;
}

function startedFrom(s: Session, p: Pull): boolean {
  return s.pull?.tracker === p.tracker && s.pull.repo === p.repo && s.pull.id === p.id;
}

function onBranchOf(s: Session, p: Pull): boolean {
  return s.work_branch === p.branch && bare(s.repo) === bare(p.repo_url);
}

/// A repository's address with what varies between two spellings of it taken
/// off: the scheme, a user before the host (Azure DevOps puts the organisation
/// there), `.git`, a trailing slash, and case. The scp-like form git uses for
/// SSH, `git@github.com:o/r.git`, has its colon read as the slash it stands
/// for, so a project cloned over SSH is still the repository its pull
/// requests name over HTTPS.
export function bare(url: string): string {
  const plain = url
    .trim()
    .toLowerCase()
    .replace(/^[a-z+]+:\/\//, "")
    .replace(/^[^@/]+@/, "")
    .replace(/\/+$/, "")
    .replace(/\.git$/, "");
  return /^[a-z0-9.-]+:[^/\d]/.test(plain) ? plain.replace(":", "/") : plain;
}
