// Projects, and the worktrees in them.
//
// A project is a repository someone has decided to work on; a worktree is one
// sandbox with one branch and one agent in it. The tree is that sentence as a
// shape, which is the whole reason it replaced a flat list: sessions across
// four repositories sorted by name told you nothing about which four
// repositories they were.
//
// Worktrees whose session records no project -- everything created from the
// terminal, which has none -- are grouped by their clone URL at the bottom
// rather than hidden or forced into one. `hura new` is not going away, and a
// worktree started from it should still be reachable here.
//
// The shape of a row is a Mail message's, and deliberately: two lines, with
// the agent's state in a fixed column to the left. The name and how old it is
// on the first, as a message's sender and date; the branch and how far it has
// got on the second, as its subject. A line per session is the right thing
// when a session is a name; it stops being right once a session is a name, a
// state, a branch, a diff and an age, which on one line is five columns
// fighting over 240 pixels. Only the state and the name are at full contrast,
// so the list is scannable without being read, and anything that is not
// needed on every row -- the context meter, the files not yet added -- appears
// only when it says something.
//
// The order is whoever's using it: projects and the worktrees in each can be
// dragged into place, or moved a step from the menu or with Option and an
// arrow. The order is kept on the server, so the phone shows the same one.

import { useState } from "react";

import type { DiffStat } from "./gen/DiffStat";
import type { Poll } from "./gen/Poll";
import type { Usage } from "./gen/Usage";
import type { Project } from "./gen/Project";
import type { PullRequest as Pull } from "./gen/PullRequest";
import type { QueuedMessage } from "./gen/QueuedMessage";
import type { QueuedSession } from "./gen/QueuedSession";
import type { Session } from "./gen/Session";
import { copy, useContextMenu } from "./ContextMenu";
import { ChecksMark, sessionPull } from "./Inbox";
import { pullName } from "./inboxNotify";
import { usePreviews } from "./previews";
import { Empty } from "./Empty";
import {
  Branch,
  Chevron,
  Copy,
  Edit,
  Forget,
  MoveDown,
  MoveUp,
  Plus,
  Ports,
  PullRequest,
  Queued,
  Scheduled,
  Start,
  StateDot,
  Unsandboxed,
} from "./icons";

export type Group = {
  /// The project, or `null` for the by-repository groups at the bottom.
  project: Project | null;
  label: string;
  hint: string;
  worktrees: Session[];
  /// Sessions on the server's queue that will start in this group, at a time
  /// or once there is room. Drawn after the worktrees, which is where they
  /// will appear.
  waiting: QueuedSession[];
};

/// Sort sessions, and the ones waiting to start, into their projects.
export function group(projects: Project[], sessions: Session[], queued: QueuedSession[] = []): Group[] {
  const groups: Group[] = projects.map((project) => ({
    project,
    label: project.name,
    hint: project.repo,
    worktrees: sessions.filter((s) => s.project === project.name).sort(byPlace),
    waiting: queued.filter((q) => q.new.project === project.name),
  }));

  // Anything whose project is gone counts as unassigned too, not just anything
  // that never had one: forgetting a project leaves its worktrees alive on
  // purpose, and they would otherwise vanish from the tree with it.
  const known = new Set(projects.map((p) => p.name));
  const loose = sessions.filter((s) => !s.project || !known.has(s.project));
  const looseQueued = queued.filter((q) => !q.new.project || !known.has(q.new.project));
  const repos = [...loose.map((s) => s.repo), ...looseQueued.map((q) => q.new.repo)];
  for (const repo of [...new Set(repos)].sort()) {
    groups.push({
      project: null,
      label: shortRepo(repo),
      hint: repo,
      worktrees: loose.filter((s) => s.repo === repo).sort(byPlace),
      waiting: looseQueued.filter((q) => q.new.repo === repo),
    });
  }
  return groups;
}

/// The order a group's worktrees are drawn in, the same as the phone's: the
/// ones nobody has placed first, newest first, since a worktree just started
/// is the one you are about to look for; then the placed ones, in the order
/// they were put.
export function byPlace(a: Session, b: Session): number {
  const ra = a.rank ?? null;
  const rb = b.rank ?? null;
  if (ra !== rb) {
    if (ra === null) return -1;
    if (rb === null) return 1;
    return ra - rb;
  }
  return b.created_at - a.created_at || a.name.localeCompare(b.name);
}

/// `names` with `name` moved to sit before or after `target`.
function placed(names: string[], name: string, target: string, after: boolean): string[] {
  const rest = names.filter((n) => n !== name);
  rest.splice(rest.indexOf(target) + (after ? 1 : 0), 0, name);
  return rest;
}

/// `names` with `name` moved one place up or down, or null at the end it is
/// already at.
function stepped(names: string[], name: string, by: -1 | 1): string[] | null {
  const i = names.indexOf(name);
  const j = i + by;
  if (i < 0 || j < 0 || j >= names.length) return null;
  const next = [...names];
  [next[i], next[j]] = [next[j], next[i]];
  return next;
}

/// What is being dragged: a project among the projects, or a worktree among
/// the others in its group. Never across groups -- a worktree belongs to the
/// project it was started in, and dropping it on another would not move it.
type Drag = { group: string; name: string };

/// The handlers that make an element something to drag, a place to drop, or
/// both, for one list of names. `group` is the list's identity: a drag is
/// only taken by targets in the list it started from.
function useSortable() {
  const [drag, setDrag] = useState<Drag | null>(null);
  const [drop, setDrop] = useState<{ name: string; after: boolean } | null>(null);
  const end = () => {
    setDrag(null);
    setDrop(null);
  };
  const ours = (group: string, name: string) => drag?.group === group && drag.name !== name;
  const half = (e: React.DragEvent<HTMLElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    return e.clientY > r.top + r.height / 2;
  };
  return {
    source: (group: string, name: string) => ({
      draggable: true,
      onDragStart: (e: React.DragEvent<HTMLElement>) => {
        // Set, or WebKit starts no drag at all; the name is also what a drop
        // anywhere else would say.
        e.dataTransfer.setData("text/plain", name);
        e.dataTransfer.effectAllowed = "move";
        // So a worktree's drag does not also pick up its project.
        e.stopPropagation();
        setDrag({ group, name });
      },
      onDragEnd: end,
    }),
    target: (group: string, name: string, names: string[], commit: (names: string[]) => void) => ({
      "data-drop":
        drop?.name === name && ours(group, name) ? (drop.after ? "after" : "before") : undefined,
      onDragOver: (e: React.DragEvent<HTMLElement>) => {
        if (drag?.group !== group) return;
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = "move";
        const after = half(e);
        if (drop?.name !== name || drop.after !== after) setDrop({ name, after });
      },
      onDrop: (e: React.DragEvent<HTMLElement>) => {
        if (drag?.group !== group) return;
        e.preventDefault();
        e.stopPropagation();
        const next = ours(group, name) ? placed(names, drag.name, name, half(e)) : names;
        end();
        if (next.join("\n") !== names.join("\n")) commit(next);
      },
    }),
  };
}

/// `https://github.com/o/thing.git` -> `o/thing`. Enough to tell two apart
/// without a line of URL per group.
function shortRepo(repo: string): string {
  const trimmed = repo.replace(/\.git$/, "").replace(/\/+$/, "");
  const parts = trimmed.split("/").filter(Boolean);
  return parts.slice(-2).join("/") || repo;
}

/// A group's key, and the identity the collapsed set is kept under. The same
/// expression the `key` prop uses, named because two places must agree on it.
function keyOf(g: Group): string {
  return g.project?.name ?? `repo:${g.hint}`;
}

export function Tree({
  width,
  groups,
  polls,
  selected,
  onSelect,
  pulls,
  onPull,
  onNewWorktree,
  onForget,
  onDestroy,
  onRename,
  onReorderProjects,
  onReorderWorktrees,
  queueLimit,
  messages,
  onUnqueue,
  onStartQueued,
}: {
  /// How wide, in pixels. From the window's own preferences and written by the
  /// handle on this sidebar's right edge -- see `prefs.ts` and `Split.tsx`.
  width: number;
  groups: Group[];
  /// Each worktree's diff against its base, by session name, as the last poll
  /// reported it. Absent for a session that has not been polled yet, which is
  /// a different thing from a session with no changes and is why the row shows
  /// nothing rather than `+0/-0`.
  ///
  /// The rest of the poll comes too: what each session has spent, how full
  /// its context is, and whether anything in it is listening.
  polls: Record<string, Poll>;
  selected: string | null;
  onSelect: (name: string) => void;
  /// The open pull requests the sidebar on the right knows of. A worktree
  /// on one of their branches says so, and opens it.
  pulls: Pull[];
  onPull: (pull: Pull) => void;
  onNewWorktree: (project: Project) => void;
  onForget: (project: Project) => void;
  onDestroy: (session: Session) => void;
  /// Give a worktree a new title. Blank takes it away, and the slug shows.
  onRename: (session: Session, title: string) => void;
  /// Put the projects in this order, by name.
  onReorderProjects: (names: string[]) => void;
  /// Put the worktrees of one group in this order, by name.
  onReorderWorktrees: (names: string[]) => void;
  /// How many sessions the server's queue lets run at once.
  queueLimit: number;
  /// Every message waiting on the server to be said to a session: a row with
  /// some says so, since nothing else would until the chat is opened.
  messages: QueuedMessage[];
  /// Take a waiting session off the queue, or start it now.
  onUnqueue: (waiting: QueuedSession) => void;
  onStartQueued: (waiting: QueuedSession) => void;
}) {
  // Which groups are shut. Held as the collapsed set rather than the open one
  // so a project that appears while the window is open -- created here, or by
  // someone else against the same server -- comes in expanded: the reason it
  // just appeared is usually that you asked for it.
  const [shut, setShut] = useState<ReadonlySet<string>>(new Set());
  const menu = useContextMenu();
  const sortable = useSortable();
  // Only projects move: a by-repository group is where worktrees with no
  // project end up, below them all, and has nowhere to be kept.
  const projectNames = groups.flatMap((g) => (g.project ? [g.project.name] : []));
  const moveProject = (name: string, by: -1 | 1) => {
    const next = stepped(projectNames, name, by);
    if (next) onReorderProjects(next);
  };
  const toggle = (key: string) =>
    setShut((all) => {
      const next = new Set(all);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  return (
    <div className="sidebar" style={{ width }}>
      <nav className="tree" aria-label="Projects and worktrees">
        {groups.map((g) => {
          const key = keyOf(g);
          const open = !shut.has(key);
          const project = g.project?.name;
          const at = project ? projectNames.indexOf(project) : -1;
          const names = g.worktrees.map((s) => s.name);
          return (
            <section
              key={key}
              className="group"
              {...(project ? sortable.target("projects", project, projectNames, onReorderProjects) : {})}
            >
              {/* The toggle and the actions are siblings rather than nested: a
                  button inside a button is not markup a browser will honour, and
                  the alternative -- a div with a click handler -- gives up the
                  keyboard and the focus ring that make the reveal below safe. */}
              {/* Everything you can do to a project is in its menu; the `+`
                  stays on the row as well, because starting a worktree is the
                  reason to be pointing at a project and it has no other door. */}
              <header
                className="group-head"
                {...(project ? sortable.source("projects", project) : {})}
                {...menu(() => [
                  ...(g.project
                    ? [
                        {
                          label: "New Worktree",
                          icon: Plus,
                          run: () => onNewWorktree(g.project!),
                        },
                        "separator" as const,
                      ]
                    : []),
                  { label: "Copy Repository URL", icon: Copy, hint: g.hint, run: () => copy(g.hint) },
                  ...(project
                    ? [
                        "separator" as const,
                        {
                          label: "Move Up",
                          icon: MoveUp,
                          disabled: at === 0,
                          run: () => moveProject(project, -1),
                        },
                        {
                          label: "Move Down",
                          icon: MoveDown,
                          disabled: at === projectNames.length - 1,
                          run: () => moveProject(project, 1),
                        },
                        "separator" as const,
                        {
                          label: "Forget Project",
                          icon: Forget,
                          hint: "Worktrees stay",
                          danger: true,
                          run: () => onForget(g.project!),
                        },
                      ]
                    : []),
                ])}
              >
                <button
                  className="group-toggle"
                  aria-expanded={open}
                  title={g.hint}
                  onClick={() => toggle(key)}
                  onKeyDown={(e) => {
                    const by = moveKey(e);
                    if (!project || !by) return;
                    e.preventDefault();
                    moveProject(project, by);
                  }}
                >
                  <Chevron open={open} className="group-twist" />
                  <span className={`group-label${g.project ? "" : " loose"}`}>{g.label}</span>
                  <span className="group-count">{g.worktrees.length}</span>
                </button>

                {/* Hidden until the group is hovered or something inside it has
                    focus -- see `style.css`. A row of controls beside every
                    project is a row of controls you read past; the ones here are
                    for the moment you have decided to act on *this* project and
                    are already pointing at it.

                    Nothing at all for a by-repository group, and no `+`: there
                    is no project to start a worktree in, and making one from
                    here would have to guess which checkout on the server the URL
                    meant, of which there may be several.

                    There used to be an `external` badge in its place, and it was
                    a word on every row of the bottom half of the sidebar that
                    told you something the label above it already says: a group
                    with no project is drawn in the mono face at the dimmer rank,
                    which is the difference. A badge earns its space when it
                    marks an exception, and `external` marked a whole category. The full URL is
                    still one hover away on the group's own title. */}
                {g.project && (
                  <span className="group-actions">
                    <button
                      className="quiet-icon"
                      title="New worktree in this project"
                      onClick={() => onNewWorktree(g.project!)}
                    >
                      <Plus aria-label="New Worktree" />
                    </button>
                  </span>
                )}
              </header>

              {/* Indented, and with a rule running down the indent -- see
                  `.group-body` in `style.css`. Two things a flat list could not
                  say: which project a card belongs to once the header above it
                  has scrolled out from under the sticky one, and where a group
                  ends. The project row keeps its own alignment at the sidebar's
                  edge, so the twisty and the name are still the column your eye
                  runs down. */}
              {open && (
                <div className="group-body">
                  {/* A branch glyph on the row a worktree would occupy, and
                      no words: `no worktrees yet` was three of them restating
                      the `0` already sitting in the group's own header two
                      lines above. */}
                  {g.worktrees.length === 0 && g.waiting.length === 0 ? (
                    <Empty size="row" icon={Branch} />
                  ) : (
                    g.worktrees.map((s, i) => (
                      <Worktree
                        key={s.name}
                        sortable={{
                          ...sortable.source(key, s.name),
                          ...sortable.target(key, s.name, names, onReorderWorktrees),
                        }}
                        first={i === 0}
                        last={i === names.length - 1}
                        onMove={(by) => {
                          const next = stepped(names, s.name, by);
                          if (next) onReorderWorktrees(next);
                        }}
                        session={s}
                        stat={polls[s.name]?.stat ?? null}
                        branch={polls[s.name]?.branch ?? s.work_branch}
                        usage={polls[s.name]?.usage ?? null}
                        listening={polls[s.name]?.ports.map((l) => l.port) ?? []}
                        waiting={messages.filter((m) => m.session === s.name)}
                        on={s.name === selected}
                        pull={sessionPull(s, pulls)}
                        onSelect={onSelect}
                        onPull={onPull}
                        onDestroy={onDestroy}
                        onRename={onRename}
                      />
                    ))
                  )}
                  {g.waiting.map((w) => (
                    <Waiting
                      key={w.id}
                      waiting={w}
                      place={placeOf(w, groups)}
                      limit={queueLimit}
                      onUnqueue={onUnqueue}
                      onStartQueued={onStartQueued}
                    />
                  ))}
                </div>
              )}
            </section>
          );
        })}
      </nav>
    </div>
  );
}

/// Option and an arrow, which is how a Mac moves a thing in a list a place:
/// -1 for up, 1 for down, null for any other key.
function moveKey(e: React.KeyboardEvent): -1 | 1 | null {
  if (!e.altKey || e.metaKey || e.ctrlKey || e.shiftKey) return null;
  return e.key === "ArrowUp" ? -1 : e.key === "ArrowDown" ? 1 : null;
}

/// How full the context is, as a ring, once it is nearly full: near the top
/// the agent compacts and forgets, and a ring says so without being read.
/// Below that it is not drawn at all -- a small ring on every row read as
/// eleven loading spinners, and a context a third full is not news.
const CONTEXT_HIGH = 80;

function Context({ usage }: { usage: Usage }) {
  const pct = usage.context_used_percentage;
  if (pct === null || pct < CONTEXT_HIGH) return null;
  const r = 5;
  const c = 2 * Math.PI * r;
  const filled = Math.max(0, Math.min(100, pct)) / 100;
  const of = usage.context_size ? ` of ${Math.round(usage.context_size / 1000)}k` : "";
  return (
    <span className="wt-context" title={`Context ${Math.round(pct)}% full${of}`}>
      <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
        <circle cx="6" cy="6" r={r} className="track" />
        <circle
          cx="6"
          cy="6"
          r={r}
          className="fill"
          strokeDasharray={`${c * filled} ${c}`}
          transform="rotate(-90 6 6)"
        />
      </svg>
    </span>
  );
}

function Worktree({
  sortable,
  first,
  last,
  onMove,
  session: s,
  stat,
  branch,
  usage,
  listening,
  waiting,
  on,
  pull,
  onSelect,
  onPull,
  onDestroy,
  onRename,
}: {
  /// What makes the row draggable among its group's, and a place to drop.
  sortable: React.HTMLAttributes<HTMLDivElement>;
  /// Whether it is at either end of its group, where it cannot move further.
  first: boolean;
  last: boolean;
  onMove: (by: -1 | 1) => void;
  session: Session;
  stat: DiffStat | null;
  /// The branch the working copy is on as of the last poll, which the record
  /// learns a poll later: an agent may have moved to one of its own.
  branch: string;
  usage: Usage | null;
  listening: number[];
  /// Messages waiting to be said to it, oldest first.
  waiting: QueuedMessage[];
  on: boolean;
  /// The open pull request on this worktree's branch, if the sidebar knows it.
  pull: Pull | null;
  onSelect: (name: string) => void;
  onPull: (pull: Pull) => void;
  onDestroy: (session: Session) => void;
  onRename: (session: Session, title: string) => void;
}) {
  const menu = useContextMenu();
  // Renamed in place, where the name is, rather than in a dialog: it is one
  // line of text, and the row is where your eye already is.
  const [renaming, setRenaming] = useState(false);
  // Previewed, not merely listening: the globe lights up while this machine
  // is forwarding one of the session's ports, so which sessions have
  // something open is readable from the tree without opening each one.
  const previewed = usePreviews().filter((p) => p.session === s.name);
  return (
    // Destroying is in the menu rather than an icon on the card: an X on every
    // row is an X you stop seeing, and this one ends an agent.
    <div
      className={`worktree-row${on ? " on" : ""}`}
      // Not while renaming, or a drag to select the name's text moves the row.
      {...(renaming ? {} : sortable)}
      {...menu(() => [
        { label: "Rename…", icon: Edit, run: () => setRenaming(true) },
        "separator",
        { label: "Move Up", icon: MoveUp, disabled: first, run: () => onMove(-1) },
        { label: "Move Down", icon: MoveDown, disabled: last, run: () => onMove(1) },
        "separator",
        { label: "Copy Name", icon: Copy, hint: s.name, run: () => copy(s.name) },
        { label: "Copy Branch", icon: Branch, hint: branch, run: () => copy(branch) },
        ...(pull
          ? [{ label: `Open ${pullName(pull)}`, icon: PullRequest, run: () => onPull(pull) }]
          : []),
        "separator",
        {
          label: "Destroy Worktree",
          icon: Forget,
          hint: s.backend === "worktree" ? "and its directory" : "and its sandbox",
          danger: true,
          run: () => onDestroy(s),
        },
      ])}
    >
    {renaming ? (
      <Rename
        session={s}
        onDone={(title) => {
          setRenaming(false);
          if (title !== null) onRename(s, title);
        }}
      />
    ) : (
    <button
      className={`worktree${on ? " on" : ""}`}
      // `aria-current` rather than `aria-pressed`: this is which of several
      // things is being shown, not a control that is held down.
      aria-current={on ? "page" : undefined}
      onClick={() => onSelect(s.name)}
      onKeyDown={(e) => {
        const by = moveKey(e);
        if (!by) return;
        e.preventDefault();
        onMove(by);
      }}
    >
      <span className="wt-state">
        <StateDot state={s.state} />
      </span>

      <span className="wt-body">
        <span className="wt-head">
          {/* The title, which is what the session is called; the slug it is
              keyed by is in the tooltip and the menu, for the moments it is
              the thing you want -- a terminal, a path. */}
          <span className="wt-name" title={s.title ? s.name : undefined}>
            {s.title ?? s.name}
          </span>
          <span className="wt-signals">
            {s.backend === "worktree" && (
              <span className="wt-unsandboxed" title="A worktree on the server: not isolated">
                <Unsandboxed />
              </span>
            )}
            {(listening.length > 0 || previewed.length > 0) && (
              <span
                className={`wt-ports${previewed.length > 0 ? " on" : ""}`}
                title={
                  previewed.length > 0
                    ? `Previewing ${previewed.map((p) => `${p.port} → localhost:${p.local}`).join(", ")}`
                    : `Listening on ${listening.join(", ")}`
                }
              >
                <Ports />
              </span>
            )}
            <WaitingMessages messages={waiting} />
            {usage && <Context usage={usage} />}
          </span>
          <span className="wt-age" title="How long this worktree has existed">
            {age(s.created_at)}
          </span>
        </span>

        <span className="wt-meta">
          <span className="wt-branch" title={branch}>
            {branch}
          </span>
          {/* Inside the row's button, so a span and not a button of its own:
              a click on it opens the pull request instead of selecting the
              worktree, and the row's menu has the same for the keyboard. */}
          {pull && (
            <span
              className="wt-pull"
              title={`${pullName(pull)} ${pull.title}`}
              onClick={(e) => {
                e.stopPropagation();
                onPull(pull);
              }}
            >
              <PullRequest />
              {pull.key}
              <ChecksMark checks={pull.checks} />
            </span>
          )}
          {stat && <Stat stat={stat} />}
        </span>
      </span>
    </button>
    )}
    </div>
  );
}

/// That something is waiting to be said to a session, and when the first of
/// it will be: after the turn when any is waiting only for that, or else at
/// the soonest time. Nothing at all when nothing is waiting.
function WaitingMessages({ messages }: { messages: QueuedMessage[] }) {
  if (!messages.length) return null;
  const times = messages.map((m) => m.at);
  const soonest = times.includes(null) ? null : Math.min(...(times as number[]));
  const what = messages.length === 1 ? "A message waiting" : `${messages.length} messages waiting`;
  const when = soonest === null ? "after this turn" : startsAt(soonest);
  return (
    <span className="wt-waiting" title={`${what}, ${messages.length === 1 ? "" : "the first "}${when}`}>
      {soonest === null ? <Queued /> : <Scheduled />}
      {messages.length > 1 && messages.length}
    </span>
  );
}

/// Where a queued session stands among every one waiting for room: 1 is
/// next. Null for one waiting for a time instead.
function placeOf(w: QueuedSession, groups: Group[]): number | null {
  if (w.at !== null) return null;
  const line = groups
    .flatMap((g) => g.waiting)
    .filter((q) => q.at === null && !q.started && q.failed === null)
    .sort((a, b) => a.id - b.id);
  const i = line.findIndex((q) => q.id === w.id);
  return i < 0 ? null : i + 1;
}

/// A session on the server's queue: the shape of a worktree's row, dimmed,
/// with when it starts where the branch would be. It is not a session yet,
/// so there is nothing to select; what there is to do is in its menu.
function Waiting({
  waiting: w,
  place,
  limit,
  onUnqueue,
  onStartQueued,
}: {
  waiting: QueuedSession;
  place: number | null;
  limit: number;
  onUnqueue: (waiting: QueuedSession) => void;
  onStartQueued: (waiting: QueuedSession) => void;
}) {
  const menu = useContextMenu();
  const name = w.new.name ?? "";
  const when = w.started
    ? "Starting…"
    : w.failed !== null
      ? `Could not start: ${w.failed}`
      : w.at !== null
        ? `Starts ${startsAt(w.at)}`
        : place === 1
          ? `Next, when fewer than ${limit} are running`
          : `Queued, ${ordinal(place ?? 0)} in line`;
  return (
    <div
      className="worktree-row waiting"
      {...menu(() =>
        w.started
          ? [{ label: "Copy Name", icon: Copy, hint: name, run: () => copy(name) }]
          : [
              { label: "Start Now", icon: Start, run: () => onStartQueued(w) },
              "separator",
              { label: "Copy Name", icon: Copy, hint: name, run: () => copy(name) },
              "separator",
              { label: "Remove from Queue", icon: Forget, danger: true, run: () => onUnqueue(w) },
            ],
      )}
    >
      <div className="worktree" title={w.new.task || undefined}>
        <span className="wt-state">{w.at !== null ? <Scheduled /> : <Queued />}</span>
        <span className="wt-body">
          <span className="wt-head">
            <span className="wt-name" title={w.new.title ? name : undefined}>
              {w.new.title ?? name}
            </span>
          </span>
          <span className="wt-meta">
            <span className={`wt-branch${w.failed !== null ? " failed" : ""}`} title={when}>
              {when}
            </span>
          </span>
        </span>
      </div>
    </div>
  );
}

/// When a scheduled session starts, as briefly as is unambiguous: a time
/// alone today, a weekday within the week, and a date beyond that.
export function startsAt(at: number): string {
  const t = new Date(at * 1000);
  const now = new Date();
  const time = t.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (t.toDateString() === now.toDateString()) return `at ${time}`;
  const days = (t.getTime() - now.getTime()) / 86_400_000;
  if (days >= 0 && days < 6) return `${t.toLocaleDateString([], { weekday: "short" })} at ${time}`;
  return `${t.toLocaleDateString([], { day: "numeric", month: "short" })} at ${time}`;
}

function ordinal(n: number): string {
  const tail = n % 100 >= 11 && n % 100 <= 13 ? "th" : ["th", "st", "nd", "rd"][n % 10] ?? "th";
  return `${n}${tail}`;
}

/// The row's name, as a field. Enter keeps it, Escape or leaving puts the row
/// back as it was; `null` to `onDone` is the second.
function Rename({ session: s, onDone }: { session: Session; onDone: (title: string | null) => void }) {
  const [text, setText] = useState(s.title ?? "");
  return (
    <form
      className="worktree on wt-rename"
      onSubmit={(e) => {
        e.preventDefault();
        onDone(text.trim());
      }}
    >
      <input
        autoFocus
        value={text}
        placeholder={s.name}
        aria-label={`Rename ${s.title ?? s.name}`}
        spellCheck={false}
        onFocus={(e) => e.target.select()}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onDone(null);
        }}
        onBlur={() => onDone(null)}
      />
    </form>
  );
}

/// How far this worktree has diverged: lines added and removed, coloured the
/// way they are everywhere else in the window, and nothing at all when there is
/// nothing -- `+0 −0` is a fact nobody needs on eleven rows at once.
///
/// Files not yet added are in the tooltip rather than on the row: `?3` was git's
/// shorthand and nobody else's. They are said on the row only when they are all
/// there is, as "3 new", because a row with no diff would otherwise claim the
/// agent had done nothing.
function Stat({ stat }: { stat: DiffStat }) {
  const lines = stat.added > 0 || stat.removed > 0;
  if (!lines && stat.untracked === 0) return null;
  const said = [
    stat.added > 0 && `${stat.added} ${plural(stat.added, "line")} added`,
    stat.removed > 0 && `${stat.removed} removed`,
    stat.untracked > 0 && `${stat.untracked} new ${plural(stat.untracked, "file")} not yet added`,
  ].filter(Boolean);
  return (
    <span className="wt-stat" title={`${said.join(", ")}, against the base branch`}>
      {stat.added > 0 && <span className="added">+{stat.added}</span>}
      {stat.removed > 0 && <span className="removed">−{stat.removed}</span>}
      {!lines && <span className="untracked">{stat.untracked} new</span>}
    </span>
  );
}

function plural(n: number, word: string): string {
  return n === 1 ? word : `${word}s`;
}

/// Relative, not absolute: the question the tree answers is "how long has this
/// been going", and the record stores epoch seconds precisely so the display
/// can choose.
function age(createdAt: number): string {
  const secs = Math.max(0, Math.floor(Date.now() / 1000) - createdAt);
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h`;
  return `${Math.floor(secs / 86400)}d`;
}
