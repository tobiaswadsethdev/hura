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

import { type ComponentType, useEffect, useState } from "react";

import type { DiffStat } from "./gen/DiffStat";
import type { Poll } from "./gen/Poll";
import type { Usage } from "./gen/Usage";
import type { Project } from "./gen/Project";
import type { PullRequest as Pull } from "./gen/PullRequest";
import type { QueuedMessage } from "./gen/QueuedMessage";
import type { QueuedSession } from "./gen/QueuedSession";
import type { Session } from "./gen/Session";
import { copy, useButtonMenu, useContextMenu } from "./ContextMenu";
import { snoozeChoices, type Triage } from "./triage";
import { describeRepeat } from "./repeats";
import type { Groups, WorktreeGroup } from "./groups";
import { ChecksMark, sessionPull } from "./Inbox";
import { pullName } from "./inboxNotify";
import { usePreviews } from "./previews";
import { cn } from "./lib/cn";
import { ProjectMonogram } from "./shell/ProjectMonogram";
import { AgentMark } from "./agents";
import { Button } from "./ui/button";
import {
  Branch,
  Copy,
  Disclosure,
  Edit,
  Forget,
  GroupGlyph,
  MoveDown,
  MoveUp,
  Plus,
  Ports,
  PullRequest,
  Queued,
  Recurring,
  Scheduled,
  Pin,
  Settle,
  Snooze,
  Start,
  StatusFailed,
  Undo,
  StatusNeedsYou,
  StatusWorking,
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
    target: (group: string, name: string, names: string[], commit: (names: string[], moved: string) => void) => ({
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
        const moved = drag.name;
        const next = ours(group, name) ? placed(names, moved, name, half(e)) : names;
        end();
        if (next.join("\n") !== names.join("\n")) commit(next, moved);
      },
    }),
    /// A group's header as a place to drop a worktree of the same project:
    /// into the group, wherever it was.
    into: (group: string, id: string, onInto: (name: string) => void) => ({
      "data-into": drop?.name === `into:${id}` && drag?.group === group ? "" : undefined,
      onDragOver: (e: React.DragEvent<HTMLElement>) => {
        if (drag?.group !== group) return;
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = "move";
        if (drop?.name !== `into:${id}`) setDrop({ name: `into:${id}`, after: false });
      },
      onDrop: (e: React.DragEvent<HTMLElement>) => {
        if (drag?.group !== group) return;
        e.preventDefault();
        e.stopPropagation();
        const moved = drag.name;
        end();
        onInto(moved);
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
  triage,
  onTriage,
  sets,
}: {
  groups: Group[];
  /// The groups of related worktrees within each project: see `groups.ts`.
  sets: Groups;
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
  /// Which worktrees are pinned, settled or snoozed: see `triage.ts`.
  triage: Record<string, Triage>;
  onTriage: (name: string, to: Triage) => void;
}) {
  // The group whose name is being typed: a new one, or one being renamed.
  const [naming, setNaming] = useState<string | null>(null);
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

  // A snooze runs out while nobody touches the list, so it is looked at again
  // now and then.
  const [, tick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(timer);
  }, []);
  const now = Date.now();
  const [shelves, setShelves] = useState({ snoozed: false, settled: false });
  /// Where a worktree is drawn. One waiting on you is never out of sight.
  const placeOfSession = (s: Session): "pinned" | "active" | "snoozed" | "settled" => {
    const t = triage[s.name];
    if (!t) return "active";
    const needsYou = s.state === "waiting";
    if (t.snoozedUntil && t.snoozedUntil > now && !needsYou) return "snoozed";
    if (t.settled && !needsYou) return "settled";
    return t.pinned ? "pinned" : "active";
  };
  const everyone = groups.flatMap((g) => g.worktrees);
  const pinned = everyone.filter((s) => placeOfSession(s) === "pinned");
  const snoozed = everyone.filter((s) => placeOfSession(s) === "snoozed");
  const settled = everyone.filter((s) => placeOfSession(s) === "settled");
  /// The groups a worktree can be put in, and putting it there.
  const grouping = (s: Session, project: string) => ({
    groups: sets.groupsOf(project),
    current: sets.groupOf(s.name, project),
    onGroup: (id: string | null) => sets.assign(s.name, id),
    onNewGroup: () => setNaming(sets.create(project, "New group", [s.name])),
  });
  const card = (s: Session, extra: { project?: string; grouping?: ReturnType<typeof grouping> } = {}) => ({
    session: s,
    stat: polls[s.name]?.stat ?? null,
    branch: polls[s.name]?.branch ?? s.work_branch,
    usage: polls[s.name]?.usage ?? null,
    listening: polls[s.name]?.ports.map((l) => l.port) ?? [],
    waiting: messages.filter((m) => m.session === s.name),
    on: s.name === selected,
    pull: sessionPull(s, pulls),
    triage: triage[s.name] ?? {},
    onTriage: (to: Triage) => onTriage(s.name, to),
    onSelect,
    onPull,
    onDestroy,
    onRename,
    ...extra,
  });

  return (
    // The list alone: the sidebar around it, with its head and foot, is
    // `shell/Sidebar.tsx`. T3 Code's project folders: a 32 pixel row per
    // project, and under it the worktrees as cards.
    <nav className="flex min-h-0 flex-1 flex-col overflow-y-auto px-2 pt-1 pb-6" aria-label="Projects and worktrees">
      {pinned.length > 0 && (
        <section className="mb-2">
          <Shelf label="Pinned" />
          <ul className="flex flex-col">
            {pinned.map((s) => (
              <Worktree key={s.name} sortable={{}} first last onMove={() => {}} {...card(s, { project: s.project ?? undefined })} />
            ))}
          </ul>
        </section>
      )}
      {groups.map((g) => {
        const key = keyOf(g);
        const open = !shut.has(key);
        const project = g.project?.name;
        const at = project ? projectNames.indexOf(project) : -1;
        const here = g.worktrees.filter((s) => placeOfSession(s) === "active");
        const names = here.map((s) => s.name);
        const count = here.length + g.waiting.length;
        return (
          <section
            key={key}
            className={cn("group/folder relative mt-2 first:mt-0", DROP)}
            {...(project ? sortable.target("projects", project, projectNames, onReorderProjects) : {})}
          >
            {/* Everything you can do to a project is in its menu; the `+`
                stays on the row as well, shown when the row is pointed at,
                because starting a worktree is the reason to be pointing at a
                project. A group with no project (worktrees started from the
                terminal, grouped by their repository) has neither. */}
            <header
              className="relative"
              {...(project ? sortable.source("projects", project) : {})}
              {...menu(() => [
                ...(g.project
                  ? [
                      { label: "New Worktree", icon: Plus, run: () => onNewWorktree(g.project!) },
                      {
                        label: "New Group",
                        icon: GroupGlyph,
                        hint: "For related worktrees",
                        run: () => {
                          setShut((all) => {
                            const next = new Set(all);
                            next.delete(key);
                            return next;
                          });
                          setNaming(sets.create(g.project!.name, "New group"));
                        },
                      },
                      "separator" as const,
                    ]
                  : []),
                { label: "Copy Repository URL", icon: Copy, hint: g.hint, run: () => copy(g.hint) },
                ...(project
                  ? [
                      "separator" as const,
                      { label: "Move Up", icon: MoveUp, disabled: at === 0, run: () => moveProject(project, -1) },
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
                className="flex h-8 w-full cursor-pointer items-center gap-2.5 rounded-(--control-radius) px-2.5 text-left outline-none hover:bg-sidebar-row-hover focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
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
                {project ? (
                  <ProjectMonogram name={project} />
                ) : (
                  <Branch className="size-4 shrink-0 text-sidebar-muted-foreground" />
                )}
                <span
                  className={cn(
                    "min-w-0 flex-1 truncate",
                    project ? "text-sm font-medium text-sidebar-foreground/90" : "font-mono text-xs text-sidebar-muted-foreground",
                  )}
                >
                  {g.label}
                </span>
                {/* How many, while folded: what the fold is hiding. */}
                {!open && count > 0 && (
                  <span className="shrink-0 text-xs text-sidebar-muted-foreground tabular-nums group-hover/folder:invisible">
                    {count}
                  </span>
                )}
                <Disclosure
                  aria-hidden
                  className={cn("size-3 shrink-0 text-sidebar-muted-foreground transition-transform", !open && "-rotate-90")}
                />
              </button>
              {g.project && (
                <span className="absolute inset-y-0 right-6 flex items-center opacity-0 group-hover/folder:opacity-100 focus-within:opacity-100">
                  <Button
                    size="icon-xs"
                    variant="ghost-muted"
                    title={`New Worktree in ${g.project.name}`}
                    aria-label={`New worktree in ${g.project.name}`}
                    onClick={() => onNewWorktree(g.project!)}
                  >
                    <Plus />
                  </Button>
                </span>
              )}
            </header>
            {open && (
              <ul className="flex flex-col">
                {g.worktrees.length === 0 && g.waiting.length === 0 && sets.groupsOf(project ?? "").length === 0 ? (
                  <li className="px-2.5 py-1.5 text-xs text-sidebar-muted-foreground">No worktrees yet</li>
                ) : (
                  <>
                    {/* The project's groups first, as a folder's folders are,
                        each with its worktrees under it; then the rest. */}
                    {(project ? sets.groupsOf(project) : []).map((wg) => {
                      const members = here.filter((s) => sets.groupOf(s.name, project) === wg.id);
                      const groupKey = `group:${wg.id}`;
                      const groupOpen = !shut.has(groupKey);
                      const needsYou = members.filter((s) => s.state === "waiting").length;
                      return (
                        <li key={wg.id} className="list-none">
                          <div
                            className="relative rounded-md data-[into]:bg-primary/10 data-[into]:ring-1 data-[into]:ring-primary/40"
                            {...sortable.into(key, wg.id, (moved) => sets.assign(moved, wg.id))}
                            {...menu(() => [
                              { label: "Rename…", icon: Edit, run: () => setNaming(wg.id) },
                              "separator",
                              {
                                label: "Remove Group",
                                icon: Forget,
                                hint: "Worktrees stay",
                                danger: true,
                                run: () => sets.remove(project!, wg.id),
                              },
                            ])}
                          >
                            {naming === wg.id ? (
                              <GroupName
                                name={wg.name}
                                onDone={(name) => {
                                  setNaming(null);
                                  if (name) sets.rename(project!, wg.id, name);
                                }}
                              />
                            ) : (
                              <button
                                className="flex h-7 w-full cursor-pointer items-center gap-2 rounded-md px-2.5 text-xs font-medium text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
                                aria-expanded={groupOpen}
                                onClick={() => toggle(groupKey)}
                                onDoubleClick={() => setNaming(wg.id)}
                              >
                                <Disclosure
                                  aria-hidden
                                  className={cn("size-3 shrink-0 transition-transform", !groupOpen && "-rotate-90")}
                                />
                                <span className="min-w-0 truncate">{wg.name}</span>
                                <span className="h-px flex-1 bg-sidebar-border/50" aria-hidden />
                                {/* What the fold hides that needs you, as T3 Code's shut
                                    folders say it. */}
                                {!groupOpen && needsYou > 0 && (
                                  <span className="shrink-0 text-attention">
                                    {needsYou} {needsYou === 1 ? "needs" : "need"} you
                                  </span>
                                )}
                                <span className="shrink-0 font-normal tabular-nums">{members.length}</span>
                              </button>
                            )}
                          </div>
                          {groupOpen && (
                            <ul className="ml-3.5 flex flex-col border-l border-sidebar-border/60 pl-1.5">
                              {members.length === 0 && (
                                <li className="px-2 py-1.5 text-xs text-sidebar-muted-foreground/80">
                                  Drag worktrees here, or use a worktree's menu
                                </li>
                              )}
                              {members.map((s) => {
                                const i = names.indexOf(s.name);
                                return (
                                  <Worktree
                                    key={s.name}
                                    sortable={{
                                      ...sortable.source(key, s.name),
                                      ...sortable.target(key, s.name, names, (next, moved) => {
                                        onReorderWorktrees(next);
                                        sets.assign(moved, wg.id);
                                      }),
                                    }}
                                    first={i === 0}
                                    last={i === names.length - 1}
                                    onMove={(by) => {
                                      const next = stepped(names, s.name, by);
                                      if (next) onReorderWorktrees(next);
                                    }}
                                    {...card(s, { grouping: grouping(s, project!) })}
                                  />
                                );
                              })}
                            </ul>
                          )}
                        </li>
                      );
                    })}
                    {here
                      .filter((s) => !sets.groupOf(s.name, project))
                      .map((s) => {
                        const i = names.indexOf(s.name);
                        return (
                          <Worktree
                            key={s.name}
                            sortable={{
                              ...sortable.source(key, s.name),
                              // Dropped among the project's own: out of any group.
                              ...sortable.target(key, s.name, names, (next, moved) => {
                                onReorderWorktrees(next);
                                sets.assign(moved, null);
                              }),
                            }}
                            first={i === 0}
                            last={i === names.length - 1}
                            onMove={(by) => {
                              const next = stepped(names, s.name, by);
                              if (next) onReorderWorktrees(next);
                            }}
                            {...card(s, project ? { grouping: grouping(s, project) } : {})}
                          />
                        );
                      })}
                  </>
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
              </ul>
            )}
          </section>
        );
      })}
      {/* Pushed to the foot, T3 Code's shelves: what is put off, and what is
          done. Shut until opened; a row in either opens its worktree. */}
      <div className="mt-auto pt-3">
        {(
          [
            ["snoozed", "Snoozed", snoozed],
            ["settled", "Settled", settled],
          ] as const
        ).map(([key, label, list]) =>
          list.length === 0 ? null : (
            <section key={key}>
              <Shelf label={label} count={list.length} open={shelves[key]} onToggle={() => setShelves((all) => ({ ...all, [key]: !all[key] }))} />
              {shelves[key] && (
                <ul className="flex flex-col">
                  {list.map((s) => (
                    <ShelfRow
                      key={s.name}
                      session={s}
                      on={s.name === selected}
                      note={key === "snoozed" ? `back ${startsAt(Math.round((triage[s.name]?.snoozedUntil ?? 0) / 1000))}` : age(Math.round((triage[s.name]?.settled ?? now) / 1000))}
                      undo={key === "snoozed" ? "Wake It" : "Unsettle"}
                      onUndo={() => onTriage(s.name, key === "snoozed" ? { snoozedUntil: 0 } : { settled: 0 })}
                      onSelect={onSelect}
                    />
                  ))}
                </ul>
              )}
            </section>
          ),
        )}
      </div>
    </nav>
  );
}

/// Option and an arrow, which is how a Mac moves a thing in a list a place:
/// -1 for up, 1 for down, null for any other key.
/// A shelf's head, T3 Code's section header: its name, how many, and a
/// chevron when it folds.
function Shelf({ label, count, open, onToggle }: { label: string; count?: number; open?: boolean; onToggle?: () => void }) {
  const inner = (
    <>
      <span>{label}</span>
      <span className="h-px flex-1 bg-sidebar-border/50" aria-hidden />
      {count !== undefined && <span className="font-normal tabular-nums">{count}</span>}
      {onToggle && <Disclosure aria-hidden className={cn("size-3 shrink-0 transition-transform", !open && "-rotate-90")} />}
    </>
  );
  const cls = "flex h-8 w-full items-center gap-2 rounded-md px-2 text-xs font-medium text-sidebar-muted-foreground";
  return onToggle ? (
    <button className={cn(cls, "cursor-pointer hover:bg-sidebar-row-hover")} aria-expanded={open} onClick={onToggle}>
      {inner}
    </button>
  ) : (
    <div className={cls}>{inner}</div>
  );
}

/// A worktree on a shelf: T3 Code's slim row, its title and the note for the
/// shelf (when it comes back, how long since it was settled), and the way
/// off the shelf on hover.
function ShelfRow({
  session: s,
  on,
  note,
  undo,
  onUndo,
  onSelect,
}: {
  session: Session;
  on: boolean;
  note: string;
  undo: string;
  onUndo: () => void;
  onSelect: (name: string) => void;
}) {
  return (
    <li className="group/shelf relative list-none">
      <button
        className={cn(
          "flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-md px-2.5 text-left text-sm",
          on ? "bg-sidebar-row-active text-sidebar-foreground" : "text-sidebar-secondary-label hover:bg-sidebar-row-hover hover:text-sidebar-foreground",
        )}
        onClick={() => onSelect(s.name)}
      >
        {s.project ? <ProjectMonogram name={s.project} className="opacity-50 grayscale" /> : <Branch className="size-4 shrink-0" />}
        <span className="min-w-0 flex-1 truncate">{s.title ?? s.name}</span>
        <span className="shrink-0 text-xs tabular-nums group-hover/shelf:invisible">{note}</span>
      </button>
      <button
        className="absolute top-1/2 right-1.5 hidden -translate-y-1/2 cursor-pointer items-center gap-1 rounded-md px-1.5 py-0.5 text-xs text-sidebar-muted-foreground group-hover/shelf:inline-flex hover:text-sidebar-foreground"
        onClick={onUndo}
      >
        <Undo className="size-3.5" />
        {undo}
      </button>
    </li>
  );
}

/// Where a dragged row or project would land: a line across the top or the
/// bottom of the one under the pointer (`data-drop`, set by `useSortable`).
const DROP =
  "data-[drop]:after:pointer-events-none data-[drop]:after:absolute data-[drop]:after:inset-x-2 data-[drop]:after:h-0.5 data-[drop]:after:rounded-full data-[drop]:after:bg-primary data-[drop=before]:after:-top-px data-[drop=after]:after:-bottom-px";

/// A session's state as the sidebar says it, or `null` for one with nothing
/// to say (ready, or idle), whose row shows its age instead.
function statusOf(state: Session["state"]): { label: string; icon: ComponentType<{ className?: string; "aria-hidden"?: boolean }>; className: string } | null {
  switch (state) {
    case "creating":
    case "seeding":
      return { label: "Setting up", icon: StatusWorking, className: "text-info-foreground" };
    case "running":
      return { label: "Working", icon: StatusWorking, className: "text-info-foreground" };
    case "waiting":
      return { label: "Needs you", icon: StatusNeedsYou, className: "text-warning-foreground" };
    case "failed":
    case "dead":
      return { label: "Failed", icon: StatusFailed, className: "text-error-foreground" };
    default:
      return null;
  }
}

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
    <span className="inline-flex" title={`Context ${Math.round(pct)}% full${of}`}>
      <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
        <circle cx="6" cy="6" r={r} fill="none" stroke="var(--line-strong)" strokeWidth="1.5" />
        {/* Only drawn near the top, which is where the agent compacts:
            amber, because it is a warning about what happens next. */}
        <circle
          cx="6"
          cy="6"
          r={r}
          fill="none"
          stroke="var(--warn)"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeDasharray={`${c * filled} ${c}`}
          transform="rotate(-90 6 6)"
        />
      </svg>
    </span>
  );
}

function Worktree({
  project,
  triage,
  onTriage,
  grouping,
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
  sortable: React.HTMLAttributes<HTMLLIElement>;
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
  /// Its project, for a card outside its project's folder (on the Pinned
  /// shelf), where T3 Code puts a thread's: on its first line.
  project?: string;
  triage: Triage;
  onTriage: (to: Triage) => void;
  /// Its project's groups and the one it is in, when it is in its project's
  /// folder: see `groups.ts`.
  grouping?: {
    groups: WorktreeGroup[];
    current: string | null;
    onGroup: (id: string | null) => void;
    onNewGroup: () => void;
  };
}) {
  const menu = useContextMenu();
  const buttonMenu = useButtonMenu();
  const snoozes = () =>
    snoozeChoices().map((c) => ({ label: `Snooze ${c.label}`, icon: Snooze, run: () => onTriage({ snoozedUntil: c.until }) }));
  // Renamed in place, where the name is, rather than in a dialog: it is one
  // line of text, and the row is where your eye already is.
  const [renaming, setRenaming] = useState(false);
  // Previewed, not merely listening: the globe lights up while this machine
  // is forwarding one of the session's ports, so which sessions have
  // something open is readable from the tree without opening each one.
  const previewed = usePreviews().filter((p) => p.session === s.name);
  // The row's state as T3 Code says a thread's: a glyph and a word, in the
  // state's colour, where the time would be. A worktree with nothing to say
  // shows how old it is instead.
  const status = statusOf(s.state);
  return (
    // Destroying is in the menu rather than an icon on the card: an X on every
    // row is an X you stop seeing, and this one ends an agent.
    <li
      className={cn("group/row relative list-none py-0.5", DROP)}
      // Not while renaming, or a drag to select the name's text moves the row.
      {...(renaming ? {} : sortable)}
      {...menu(() => [
        { label: "Rename…", icon: Edit, run: () => setRenaming(true) },
        "separator",
        triage.pinned
          ? { label: "Unpin", icon: Pin, run: () => onTriage({ pinned: false }) }
          : { label: "Pin to the Top", icon: Pin, run: () => onTriage({ pinned: true }) },
        ...snoozes(),
        triage.settled
          ? { label: "Unsettle", icon: Undo, run: () => onTriage({ settled: 0 }) }
          : { label: "Settle", icon: Settle, hint: "Done for now", run: () => onTriage({ settled: Date.now() }) },
        ...(grouping
          ? [
              "separator" as const,
              { heading: "Group" },
              ...grouping.groups.map((g) => ({
                label: g.name,
                checked: g.id === grouping.current,
                run: () => grouping.onGroup(g.id === grouping.current ? null : g.id),
              })),
              { label: "New Group…", icon: GroupGlyph, run: grouping.onNewGroup },
              ...(grouping.current
                ? [{ label: "Take Out of the Group", icon: Undo, run: () => grouping.onGroup(null) }]
                : []),
            ]
          : []),
        "separator",
        { label: "Move Up", icon: MoveUp, disabled: first, run: () => onMove(-1) },
        { label: "Move Down", icon: MoveDown, disabled: last, run: () => onMove(1) },
        "separator",
        { label: "Copy Name", icon: Copy, hint: s.name, run: () => copy(s.name) },
        { label: "Copy Branch", icon: Branch, hint: branch, run: () => copy(branch) },
        ...(pull ? [{ label: `Open ${pullName(pull)}`, icon: PullRequest, run: () => onPull(pull) }] : []),
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
          className={cn(
            "relative w-full cursor-pointer overflow-hidden rounded-md px-2.5 py-2 text-left outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
            on
              ? "bg-sidebar-row-active text-sidebar-foreground light:ring-1 light:ring-sidebar-border/70 light:ring-inset"
              : "text-sidebar-foreground hover:bg-sidebar-row-hover",
          )}
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
          {/* The branch, where T3 Code puts the project: the project is the
              folder this card is in. */}
          <span className="flex h-5 min-w-0 items-center gap-1.5">
            {project ? (
              <>
                <ProjectMonogram name={project} />
                <span className="min-w-0 flex-1 truncate text-xs text-sidebar-secondary-label" title={branch}>
                  {project}
                </span>
              </>
            ) : (
              <>
                <Branch className="size-3.5 shrink-0 text-sidebar-secondary-label" />
                <span className="min-w-0 flex-1 truncate text-xs text-sidebar-secondary-label" title={branch}>
                  {branch}
                </span>
              </>
            )}
            {status ? (
              <span className={cn("ml-auto inline-flex shrink-0 items-center gap-1 text-xs font-medium group-hover/row:invisible", status.className)}>
                <status.icon aria-hidden className="size-4 shrink-0" />
                {status.label}
              </span>
            ) : (
              <span
                className="ml-auto shrink-0 text-xs text-sidebar-secondary-label tabular-nums group-hover/row:invisible"
                title="How long this worktree has existed"
              >
                {age(s.created_at)}
              </span>
            )}
          </span>
          {/* The title, which is what the session is called; the slug it is
              keyed by is in the tooltip and the menu, for the moments it is
              the thing you want: a terminal, a path. */}
          <span className="mt-1 flex min-w-0 items-center gap-1.5">
            <span className="min-w-0 flex-1 truncate text-sm font-medium" title={s.title ? s.name : undefined}>
              {s.title ?? s.name}
            </span>
          </span>
          {/* What is true about it beyond its state, each only when it says
              something: its pull request and checks, how far it has got, and
              the sandbox's signals; and at the end, always, the mark of the
              agent it runs, where T3 Code puts a thread's provider. */}
          <span className="mt-0.5 flex h-5 min-w-0 items-center gap-2 text-xs text-sidebar-secondary-label">
              {pull && (
                // Inside the row's button, so a span and not a button of its
                // own: a click on it opens the pull request instead of
                // selecting the worktree, and the row's menu has the same for
                // the keyboard.
                <span
                  className="inline-flex shrink-0 cursor-pointer items-center gap-1 rounded-sm hover:text-sidebar-foreground"
                  title={`${pullName(pull)} ${pull.title}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    onPull(pull);
                  }}
                >
                  <PullRequest className="size-3.5" />
                  {pull.key}
                  <ChecksMark checks={pull.checks} />
                </span>
              )}
              {stat && <Stat stat={stat} />}
              <span className="ml-auto inline-flex shrink-0 items-center gap-1.5">
                {s.backend === "worktree" && (
                  <span className="inline-flex text-warning-foreground" title="A worktree on the server: not isolated">
                    <Unsandboxed className="size-3.5" />
                  </span>
                )}
                {(listening.length > 0 || previewed.length > 0) && (
                  // Previewed, not merely listening: the globe lights up while
                  // this machine is forwarding one of the session's ports.
                  <span
                    className={cn("inline-flex", previewed.length > 0 && "text-success-foreground")}
                    title={
                      previewed.length > 0
                        ? `Previewing ${previewed.map((p) => `${p.port} → localhost:${p.local}`).join(", ")}`
                        : `Listening on ${listening.join(", ")}`
                    }
                  >
                    <Ports className="size-3.5" />
                  </span>
                )}
                <WaitingMessages messages={waiting} />
                {usage && <Context usage={usage} />}
                <AgentMark agent={s.agent} className="size-3.5 opacity-60" />
              </span>
          </span>
        </button>
      )}
      {/* T3 Code's controls on a card, shown where its state is while it is
          pointed at: put it off, pin it, or settle it as done for now. */}
      {!renaming && (
        <span className="absolute top-2.5 right-2 hidden items-center gap-0.5 group-hover/row:flex">
          <button className={TRIAGE_BUTTON} title="Snooze" aria-label="Snooze" onClick={buttonMenu(snoozes)}>
            <Snooze />
          </button>
          <button
            className={TRIAGE_BUTTON}
            title={triage.pinned ? "Unpin" : "Pin to the Top"}
            aria-label={triage.pinned ? "Unpin" : "Pin"}
            onClick={() => onTriage({ pinned: !triage.pinned })}
          >
            <Pin />
          </button>
          <button
            className={TRIAGE_BUTTON}
            title="Settle: Done for Now"
            aria-label="Settle"
            onClick={() => onTriage({ settled: Date.now() })}
          >
            <Settle />
          </button>
        </span>
      )}
    </li>
  );
}

const TRIAGE_BUTTON =
  "inline-flex size-6 cursor-pointer items-center justify-center rounded-md text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground [&_svg]:size-3.5";

/// That something is waiting to be said to a session, and when the first of
/// it will be: after the turn when any is waiting only for that, or else at
/// the soonest time. Nothing at all when nothing is waiting.
function WaitingMessages({ messages }: { messages: QueuedMessage[] }) {
  if (!messages.length) return null;
  const times = messages.map((m) => m.at);
  const soonest = times.includes(null) ? null : Math.min(...(times as number[]));
  const what = messages.length === 1 ? "A message waiting" : `${messages.length} messages waiting`;
  const when = soonest === null ? "after this turn" : startsAt(soonest);
  const Mark = soonest === null ? Queued : Scheduled;
  return (
    <span
      className="inline-flex items-center gap-0.5 tabular-nums"
      title={`${what}, ${messages.length === 1 ? "" : "the first "}${when}`}
    >
      <Mark className="size-3.5" />
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
  // A repeating one says its rule, and when it next comes round.
  const rule = w.repeat ? describeRepeat(w.repeat) : null;
  const when = w.started
    ? "Starting…"
    : w.failed !== null
      ? `Could not start: ${w.failed}`
      : rule && w.at !== null
        ? `${rule}; next ${startsAt(w.at)}${w.runs ? `. Started ${w.runs} ${w.runs === 1 ? "time" : "times"}, last as ${w.last}` : ""}`
        : w.at !== null
        ? `Starts ${startsAt(w.at)}`
        : place === 1
          ? `Next, when fewer than ${limit} are running`
          : `Queued, ${ordinal(place ?? 0)} in line`;
  const Mark = rule ? Recurring : w.at !== null ? Scheduled : Queued;
  // The same, in the few words the row has room for; the sentence above is
  // its tooltip.
  const short = w.started
    ? "Starting"
    : w.failed !== null
      ? "Could not start"
      : w.at !== null
        ? startsAt(w.at)
        : place === 1
          ? "Next"
          : `${ordinal(place ?? 0)} in line`;
  // T3 Code's slim row: what will start, and when, a step quieter than a
  // worktree that exists.
  return (
    <li
      className="list-none py-0.5"
      {...menu(() =>
        w.started
          ? [{ label: "Copy Name", icon: Copy, hint: name, run: () => copy(name) }]
          : [
              // A repeating one starts a copy now and keeps its time.
              { label: rule ? "Start One Now" : "Start Now", icon: Start, run: () => onStartQueued(w) },
              "separator",
              { label: "Copy Name", icon: Copy, hint: name, run: () => copy(name) },
              "separator",
              {
                label: rule ? "Stop Repeating" : "Remove from Queue",
                icon: Forget,
                danger: true,
                run: () => onUnqueue(w),
              },
            ],
      )}
    >
      <div
        className="flex h-9 min-w-0 items-center gap-2.5 rounded-md px-2.5 text-sidebar-muted-foreground"
        title={w.new.task || undefined}
      >
        <Mark className="size-4 shrink-0 opacity-60" />
        <span className="min-w-0 flex-1 truncate text-sm" title={w.new.title ? name : undefined}>
          {w.new.title ?? name}
        </span>
        <span
          className={cn("shrink-0 text-xs", w.failed !== null && "text-error-foreground")}
          title={when}
        >
          {short}
        </span>
      </div>
    </li>
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
/// A group's name, typed in place of its header: Enter keeps it, Escape or
/// leaving keeps the old one.
function GroupName({ name, onDone }: { name: string; onDone: (name: string | null) => void }) {
  const [text, setText] = useState(name);
  return (
    <form
      className="px-1 py-0.5"
      onSubmit={(e) => {
        e.preventDefault();
        onDone(text.trim() || null);
      }}
    >
      <input
        autoFocus
        className="h-7 w-full rounded-md border border-input bg-transparent px-2 text-xs font-medium text-sidebar-foreground outline-none focus:border-ring"
        value={text}
        aria-label="Group name"
        spellCheck={false}
        onFocus={(e) => e.target.select()}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => e.key === "Escape" && onDone(null)}
        onBlur={() => onDone(text.trim() || null)}
      />
    </form>
  );
}

function Rename({ session: s, onDone }: { session: Session; onDone: (title: string | null) => void }) {
  const [text, setText] = useState(s.title ?? "");
  return (
    <form
      className="rounded-md bg-sidebar-row-active px-2.5 py-2"
      onSubmit={(e) => {
        e.preventDefault();
        onDone(text.trim());
      }}
    >
      <input
        autoFocus
        className="h-7 w-full rounded-md border border-input bg-transparent px-2 text-sm font-medium text-sidebar-foreground outline-none focus:border-ring"
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
  // T3 Code's counts: mono, the additions in its emerald and the removals in
  // its red.
  return (
    <span className="inline-flex shrink-0 gap-1 font-mono tabular-nums" title={`${said.join(", ")}, against the base branch`}>
      {stat.added > 0 && <span className="text-success-foreground">+{stat.added}</span>}
      {stat.removed > 0 && <span className="text-error-foreground">−{stat.removed}</span>}
      {!lines && <span>{stat.untracked} new</span>}
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
