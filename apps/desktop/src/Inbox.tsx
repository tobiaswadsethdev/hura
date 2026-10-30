// The sidebar on the far right: your tickets and your pull requests.
//
// Always there, whatever worktree is selected and whatever screen is open,
// because the question it answers ("what is mine right now?") is not about the
// worktree in front of you. It replaced a tickets screen that hid the
// workspace to show one column per filter, and the filters went with it:
// every tracker is asked the same thing, what is assigned to you or created by
// you, so the sidebar means the same thing whichever tracker it reads.
//
// **Two headings, two groups each.** Tickets assigned to you and tickets you
// created; pull requests waiting on you (a review asked of you, or assigned
// to you) and pull requests you opened. A row in both groups is in the first,
// because that is the half that asks something of you.
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
// pixels. The shortcut that used to open the tickets screen folds and unfolds
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
  { key: "assigned", relation: "assigned", label: "assigned to me" },
  { key: "created", relation: "created", label: "created by me" },
];

export function InboxBar({
  width,
  open,
  shut,
  inbox,
  reading,
  error,
  trackers,
  sessions,
  projects,
  currentProject,
  showing,
  showingPull,
  keys,
  onFold,
  onShut,
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
  /// The last read. `null` before the first one has answered.
  inbox: Inbox | null;
  /// A read is in flight.
  reading: boolean;
  /// Why the last read could not be made at all: the trackers file would not
  /// load, say. A tracker that could not be read is a warning in `inbox`.
  error: string | null;
  /// The trackers set up on this machine. `null` until they have been read.
  trackers: ConfiguredTracker[] | null;
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
        tasks={tasks}
        pulls={pulls}
        warned={(inbox?.warnings.length ?? 0) > 0 || error !== null}
        keys={keys}
        onUnfold={onFold}
      />
    );
  }

  const none = trackers !== null && trackers.length === 0;
  // Pull requests come from GitHub and Azure DevOps. With only Jira set up,
  // the heading says where they would come from rather than sitting empty.
  const forges = trackers?.some((t) => t.source.kind !== "jira") ?? false;
  const isShut = (key: string) => shut.includes(key);
  const toggle = (key: string) =>
    onShut(isShut(key) ? shut.filter((k) => k !== key) : [...shut, key]);

  return (
    <aside className="inbox" style={{ width }} aria-label="your tickets and pull requests">
      <header className="inbox-head">
        <span className="inbox-title">mine</span>
        {reading && <Working className="inbox-reading" />}
        <span className="inbox-tools">
          <button
            className="quiet-icon"
            title="read them again"
            disabled={reading || none}
            onClick={onRefresh}
          >
            <Refresh aria-label="refresh" />
          </button>
          <button
            className="quiet-icon"
            title={`fold this sidebar (${keys.label})`}
            aria-keyshortcuts={keys.aria}
            onClick={onFold}
          >
            <Fold aria-label="fold" />
          </button>
        </span>
      </header>

      <div className="inbox-body scrollbar-sleek">
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

        {none ? (
          <Empty icon={Tracker} note="no trackers yet">
            <button className="go" onClick={onSetup}>
              add a tracker
            </button>
          </Empty>
        ) : (
          <>
            <Heading icon={Tracker} label="tickets" count={tasks.length} />
            {GROUPS.map((g) => {
              const key = `tickets:${g.key}`;
              const rows = tasks.filter((t) => t.relation === g.relation);
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
            })}

            <Heading icon={PullRequest} label="pull requests" count={pulls.length} />
            {!forges && trackers !== null ? (
              <p className="inbox-hint">
                Pull requests come from GitHub and Azure DevOps trackers.{" "}
                <button className="link" onClick={onSetup}>
                  add one
                </button>
              </p>
            ) : (
              GROUPS.map((g) => {
                const key = `pulls:${g.key}`;
                const rows = pulls.filter((p) => p.relation === g.relation);
                return (
                  <Group
                    key={key}
                    label={g.relation === "assigned" ? "waiting on me" : g.label}
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
          </>
        )}
      </div>
    </aside>
  );
}

/// Folded: the two counts, and the way back.
function Rail({
  tasks,
  pulls,
  warned,
  keys,
  onUnfold,
}: {
  tasks: Task[];
  pulls: Pull[];
  warned: boolean;
  keys: { label: string; aria: string };
  onUnfold: () => void;
}) {
  const split = <T extends { relation: Relation }>(rows: T[], mine: string, theirs: string) => {
    const assigned = rows.filter((r) => r.relation === "assigned").length;
    return `${assigned} ${theirs}, ${rows.length - assigned} ${mine}`;
  };
  return (
    <aside className="inbox-rail" aria-label="your tickets and pull requests, folded">
      <button
        className="quiet-icon"
        title={`open this sidebar (${keys.label})`}
        aria-keyshortcuts={keys.aria}
        onClick={onUnfold}
      >
        <Unfold aria-label="open" />
      </button>
      <button
        className="inbox-rail-count"
        title={`tickets: ${split(tasks, "created by you", "assigned to you")}`}
        onClick={onUnfold}
      >
        <Tracker />
        <span>{tasks.length}</span>
      </button>
      <button
        className="inbox-rail-count"
        title={`pull requests: ${split(pulls, "yours", "waiting on you")}`}
        onClick={onUnfold}
      >
        <PullRequest />
        <span>{pulls.length}</span>
      </button>
      {warned && (
        <span className="inbox-rail-warn" title="a tracker could not be read; open the sidebar to see which" />
      )}
    </aside>
  );
}

function Heading({
  icon: Mark,
  label,
  count,
}: {
  icon: React.ComponentType;
  label: string;
  count: number;
}) {
  return (
    <h3 className="inbox-heading">
      <Mark />
      <span>{label}</span>
      <span className="inbox-count">{count}</span>
    </h3>
  );
}

/// One group under a heading, shut by a click on its name.
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
      {open && count === 0 && !loading && <p className="inbox-none">nothing</p>}
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
    { label: "Open in the browser", icon: Elsewhere, run: () => openExternal(task.url) },
    ...(session ? [{ label: `Go to ${session.name}`, run: () => onSelect(session.name) }] : []),
    ...(starts.length > 0 ? (["separator", ...starts] as MenuItem[]) : []),
    "separator",
    { label: "Copy key", icon: Copy, hint: task.key, run: () => copy(task.key) },
    { label: "Copy link", icon: Copy, run: () => copy(task.url) },
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
        {!readsHere && <Elsewhere className="inbox-out" aria-label="opens in the browser" />}
        {task.item_type && <span className="inbox-type">{task.item_type}</span>}
        <span className="inbox-status">{task.status}</span>
      </div>
      <p className="inbox-row-title">{task.title}</p>
      <div className="inbox-meta">
        {updated && <span title={task.updated ?? undefined}>{updated}</span>}
        {comments > 0 && (
          <span
            className="inbox-comments"
            title={task.last_commenter ? `last comment by ${task.last_commenter}` : undefined}
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
                ? "make a project first"
                : projects.length === 1
                  ? `start a worktree for it in ${projects[0]!.name}`
                  : "start a worktree for it"
            }
            onClick={(e) => {
              if (projects.length === 1) onStart(projects[0]!, task);
              else startMenu(() => starts)(e);
            }}
          >
            <Start aria-label="start" />
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
          label: projects.length > 1 ? `Work on it in ${p.name}` : "Work on it",
          icon: Start,
          run: () => onStart(p, pull, "work"),
        },
        {
          label: projects.length > 1 ? `Review it in ${p.name}` : "Review it",
          icon: Review,
          run: () => onStart(p, pull, "review"),
        },
      ]);
  const menuProps = menu(() => [
    { label: "Open", run: () => onShow(pull) },
    { label: "Open in the browser", icon: Elsewhere, run: () => openExternal(pull.url) },
    ...(session ? [{ label: `Go to ${session.name}`, run: () => onSelect(session.name) }] : []),
    ...(starts.length > 0 && !session ? (["separator", ...starts] as MenuItem[]) : []),
    "separator",
    { label: "Copy branch", icon: Copy, hint: pull.branch, run: () => copy(pull.branch) },
    { label: "Copy link", icon: Copy, run: () => copy(pull.url) },
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
        <Mark className={`inbox-pull-mark${pull.draft ? " draft" : ""}`} aria-label={pull.draft ? "draft" : "pull request"} />
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
        {pull.fork && <span title="opened from a fork">fork</span>}
        {session ? (
          <Worktree session={session} onSelect={onSelect} />
        ) : (
          !pull.fork && (
            <button
              className="quiet-icon inbox-start"
              disabled={projects.length === 0}
              title={
                projects.length === 0
                  ? `no project is a clone of ${pull.repo}`
                  : "start a worktree on its branch"
              }
              onClick={(e) => startMenu(() => starts)(e)}
            >
              <Start aria-label="start" />
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
      return <Passing className="inbox-checks ok" aria-label="checks passed" />;
    case "failing":
      return <Failing className="inbox-checks bad" aria-label="checks failed" />;
    case "pending":
      return <Pending className="inbox-checks running" aria-label="checks running" />;
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
      title={`go to ${session.name}`}
      onClick={() => onSelect(session.name)}
    >
      <StateDot state={session.state} />
      <span>{session.name}</span>
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
