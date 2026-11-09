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

import { useState } from "react";

import type { DiffStat } from "./gen/DiffStat";
import type { Poll } from "./gen/Poll";
import type { Usage } from "./gen/Usage";
import type { Project } from "./gen/Project";
import type { PullRequest as Pull } from "./gen/PullRequest";
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
  Forget,
  Plus,
  Ports,
  PullRequest,
  StateDot,
  Unsandboxed,
} from "./icons";

export type Group = {
  /// The project, or `null` for the by-repository groups at the bottom.
  project: Project | null;
  label: string;
  hint: string;
  worktrees: Session[];
};

/// Sort sessions into their projects.
export function group(projects: Project[], sessions: Session[]): Group[] {
  const groups: Group[] = projects.map((project) => ({
    project,
    label: project.name,
    hint: project.repo,
    worktrees: sessions.filter((s) => s.project === project.name),
  }));

  // Anything whose project is gone counts as unassigned too, not just anything
  // that never had one: forgetting a project leaves its worktrees alive on
  // purpose, and they would otherwise vanish from the tree with it.
  const known = new Set(projects.map((p) => p.name));
  const loose = sessions.filter((s) => !s.project || !known.has(s.project));
  for (const repo of [...new Set(loose.map((s) => s.repo))].sort()) {
    groups.push({
      project: null,
      label: shortRepo(repo),
      hint: repo,
      worktrees: loose.filter((s) => s.repo === repo),
    });
  }
  return groups;
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
}) {
  // Which groups are shut. Held as the collapsed set rather than the open one
  // so a project that appears while the window is open -- created here, or by
  // someone else against the same server -- comes in expanded: the reason it
  // just appeared is usually that you asked for it.
  const [shut, setShut] = useState<ReadonlySet<string>>(new Set());
  const menu = useContextMenu();
  const toggle = (key: string) =>
    setShut((all) => {
      const next = new Set(all);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  return (
    <div className="sidebar" style={{ width }}>
      <nav className="tree scrollbar-sleek" aria-label="projects and worktrees">
        {groups.map((g) => {
          const key = keyOf(g);
          const open = !shut.has(key);
          return (
            <section key={key} className="group">
              {/* The toggle and the actions are siblings rather than nested: a
                  button inside a button is not markup a browser will honour, and
                  the alternative -- a div with a click handler -- gives up the
                  keyboard and the focus ring that make the reveal below safe. */}
              {/* Everything you can do to a project is in its menu; the `+`
                  stays on the row as well, because starting a worktree is the
                  reason to be pointing at a project and it has no other door. */}
              <header
                className="group-head"
                {...menu(() => [
                  ...(g.project
                    ? [
                        {
                          label: "New worktree",
                          icon: Plus,
                          run: () => onNewWorktree(g.project!),
                        },
                        "separator" as const,
                      ]
                    : []),
                  { label: "Copy repository URL", icon: Copy, hint: g.hint, run: () => copy(g.hint) },
                  ...(g.project
                    ? [
                        "separator" as const,
                        {
                          label: "Forget project",
                          icon: Forget,
                          hint: "worktrees stay",
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
                      title="new worktree in this project"
                      onClick={() => onNewWorktree(g.project!)}
                    >
                      <Plus aria-label="new worktree" />
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
                  {g.worktrees.length === 0 ? (
                    <Empty size="row" icon={Branch} />
                  ) : (
                    g.worktrees.map((s) => (
                      <Worktree
                        key={s.name}
                        session={s}
                        stat={polls[s.name]?.stat ?? null}
                        usage={polls[s.name]?.usage ?? null}
                        listening={polls[s.name]?.ports.map((l) => l.port) ?? []}
                        on={s.name === selected}
                        pull={sessionPull(s, pulls)}
                        onSelect={onSelect}
                        onPull={onPull}
                        onDestroy={onDestroy}
                      />
                    ))
                  )}
                </div>
              )}
            </section>
          );
        })}
      </nav>
    </div>
  );
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
    <span className="wt-context" title={`context ${Math.round(pct)}% full${of}`}>
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
  session: s,
  stat,
  usage,
  listening,
  on,
  pull,
  onSelect,
  onPull,
  onDestroy,
}: {
  session: Session;
  stat: DiffStat | null;
  usage: Usage | null;
  listening: number[];
  on: boolean;
  /// The open pull request on this worktree's branch, if the sidebar knows it.
  pull: Pull | null;
  onSelect: (name: string) => void;
  onPull: (pull: Pull) => void;
  onDestroy: (session: Session) => void;
}) {
  const menu = useContextMenu();
  // Previewed, not merely listening: the globe lights up while this machine
  // is forwarding one of the session's ports, so which sessions have
  // something open is readable from the tree without opening each one.
  const previewed = usePreviews().filter((p) => p.session === s.name);
  return (
    // Destroying is in the menu rather than an icon on the card: an X on every
    // row is an X you stop seeing, and this one ends an agent.
    <div
      className={`worktree-row${on ? " on" : ""}`}
      {...menu(() => [
        { label: "Copy name", icon: Copy, hint: s.name, run: () => copy(s.name) },
        { label: "Copy branch", icon: Branch, hint: s.work_branch, run: () => copy(s.work_branch) },
        ...(pull
          ? [{ label: `Open ${pullName(pull)}`, icon: PullRequest, run: () => onPull(pull) }]
          : []),
        "separator",
        {
          label: "Destroy worktree",
          icon: Forget,
          hint: s.backend === "worktree" ? "and its directory" : "and its sandbox",
          danger: true,
          run: () => onDestroy(s),
        },
      ])}
    >
    <button
      className={`worktree${on ? " on" : ""}`}
      // `aria-current` rather than `aria-pressed`: this is which of several
      // things is being shown, not a control that is held down.
      aria-current={on ? "page" : undefined}
      onClick={() => onSelect(s.name)}
    >
      <span className="wt-state">
        <StateDot state={s.state} />
      </span>

      <span className="wt-body">
        <span className="wt-head">
          <span className="wt-name">{s.name}</span>
          <span className="wt-signals">
            {s.backend === "worktree" && (
              <span className="wt-unsandboxed" title="a worktree on the server: not isolated">
                <Unsandboxed />
              </span>
            )}
            {(listening.length > 0 || previewed.length > 0) && (
              <span
                className={`wt-ports${previewed.length > 0 ? " on" : ""}`}
                title={
                  previewed.length > 0
                    ? `previewing ${previewed.map((p) => `${p.port} → localhost:${p.local}`).join(", ")}`
                    : `listening on ${listening.join(", ")}`
                }
              >
                <Ports />
              </span>
            )}
            {usage && <Context usage={usage} />}
          </span>
          <span className="wt-age" title="how long this worktree has existed">
            {age(s.created_at)}
          </span>
        </span>

        <span className="wt-meta">
          <span className="wt-branch" title={s.work_branch}>
            {s.work_branch}
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
    </div>
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
