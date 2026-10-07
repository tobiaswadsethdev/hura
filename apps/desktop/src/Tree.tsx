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
// The shape of a row is Orca's, and deliberately: a card rather than a line,
// inset from the edge, with the agent's state in a fixed column to the left of
// the name and the facts about the branch on a second line under it. A line
// per session is the right thing when a session is a name; it stops being
// right once a session is a name, a state, a branch, a diff and an age, which
// on one line is five columns fighting over 240 pixels. What the card buys is
// that the state and the name are the only things at full contrast, so the list
// is scannable without being read -- see `style.css`, where every surface
// treatment on it lives.

import { useState } from "react";

import type { DiffStat } from "./gen/DiffStat";
import type { Poll } from "./gen/Poll";
import type { Usage } from "./gen/Usage";
import type { Project } from "./gen/Project";
import type { Session } from "./gen/Session";
import { copy, useContextMenu } from "./ContextMenu";
import { usePreviews } from "./previews";
import { Empty } from "./Empty";
import { Branch, Chevron, Copy, Forget, Plus, Ports, StateDot, Tracker } from "./icons";
import { useHeldTickets } from "./ticketNotify";

export type Group = {
  /// The project, or `null` for the by-repository groups at the bottom.
  project: Project | null;
  label: string;
  hint: string;
  worktrees: Session[];
};

/// The way to the tickets screen from the top of the sidebar. Only ever seen
/// with the screen shut: a screen covers the sidebar, so there is no lit state
/// and the way back is the header's button, Escape or the shortcut.
export type TicketsEntry = {
  disabled: boolean;
  /// The shortcut: as this platform writes it, and as `aria-keyshortcuts`
  /// spells it.
  keys: { label: string; aria: string };
  onOpen: () => void;
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
  onNewWorktree,
  onForget,
  onDestroy,
  tickets,
}: {
  /// How wide, in pixels. From the window's own preferences and written by the
  /// handle on this sidebar's right edge -- see `prefs.ts` and `Split.tsx`.
  width: number;
  tickets: TicketsEntry;
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
    // The tickets above the projects and outside their scroll, so the way to
    // them is in the same place however long the list of worktrees gets.
    <div className="sidebar" style={{ width }}>
      <TicketsRow {...tickets} />
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
                        onSelect={onSelect}
                        onDestroy={onDestroy}
                      />
                    ))
                  )}
                </div>
              )}
            </section>
          );
        })}
        <Spend groups={groups} polls={polls} />
      </nav>
    </div>
  );
}

/// The tickets, as a row the width of the sidebar.
///
/// The header's button is the same destination, and it stays: this is the
/// one people reach for many times a day, and a 28-pixel glyph in a strip of
/// five was a target to find rather than a place to go. The count is how many
/// tickets the filters held at the last read, so the row says whether there
/// is anything there before it is opened.
function TicketsRow({ disabled, keys, onOpen }: TicketsEntry) {
  const held = useHeldTickets();
  return (
    <button
      className="tickets-row"
      disabled={disabled}
      title={`tickets (${keys.label})`}
      aria-keyshortcuts={keys.aria}
      onClick={onOpen}
    >
      <Tracker />
      <span className="tickets-label">tickets</span>
      {held !== null && held > 0 && (
        <span className="tickets-count" title={`${held} in your filters at the last read`}>
          {held}
        </span>
      )}
    </button>
  );
}

/// What every session in the list has spent, together.
///
/// The per-session cost is in each row, and a row is where you look to decide
/// about *that* session; the total is the question nobody can answer by
/// adding up eleven rows in their head. Sessions that have not reported yet
/// are left out rather than counted as zero, and the footer says how many
/// that left in.
function Spend({ groups, polls }: { groups: Group[]; polls: Record<string, Poll> }) {
  const reported = groups
    .flatMap((g) => g.worktrees)
    .map((s) => ({ name: s.name, usage: polls[s.name]?.usage }))
    .filter((r): r is { name: string; usage: Usage } => r.usage?.cost_usd != null);
  if (reported.length === 0) return null;
  const total = reported.reduce((sum, r) => sum + (r.usage.cost_usd ?? 0), 0);
  const breakdown = [...reported]
    .sort((a, b) => (b.usage.cost_usd ?? 0) - (a.usage.cost_usd ?? 0))
    .map((r) => `${r.name}  ${money(r.usage.cost_usd)}`)
    .join("\n");
  return (
    <footer className="spend" title={breakdown}>
      <span className="spend-total">{money(total)}</span>
      <span className="hint">
        across {reported.length} session{reported.length === 1 ? "" : "s"}
      </span>
    </footer>
  );
}

function money(usd: number | null): string {
  if (usd === null) return "";
  return usd >= 100 ? `$${Math.round(usd)}` : `$${usd.toFixed(2)}`;
}

/// How full the context is, as a ring: a percentage is the one number on the
/// row that is about to *matter* -- near the top of it the agent compacts and
/// forgets -- and a ring says "nearly full" without being read.
function Context({ usage }: { usage: Usage }) {
  const pct = usage.context_used_percentage;
  if (pct === null) return null;
  const r = 5;
  const c = 2 * Math.PI * r;
  const filled = Math.max(0, Math.min(100, pct)) / 100;
  const of = usage.context_size ? ` of ${Math.round(usage.context_size / 1000)}k` : "";
  return (
    <span className={`wt-context${pct >= 80 ? " high" : ""}`} title={`context ${Math.round(pct)}%${of}`}>
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
  onSelect,
  onDestroy,
}: {
  session: Session;
  stat: DiffStat | null;
  usage: Usage | null;
  listening: number[];
  on: boolean;
  onSelect: (name: string) => void;
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
        "separator",
        {
          label: "Destroy worktree",
          icon: Forget,
          hint: "and its sandbox",
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
            {usage?.cost_usd != null && (
              <span className="wt-cost" title="spent by this session">
                {money(usage.cost_usd)}
              </span>
            )}
            {usage && <Context usage={usage} />}
          </span>
        </span>

        <span className="wt-meta">
          <Branch className="wt-branch-icon" />
          <span className="wt-branch">{s.work_branch}</span>
          {stat && <Stat stat={stat} />}
          <span className="wt-age" title="how long this worktree has existed">
            {age(s.created_at)}
          </span>
        </span>
      </span>
    </button>
    </div>
  );
}

/// How far this worktree has diverged, in the three numbers the poll already
/// pays for. Rendered as spans rather than one string so the added and removed
/// counts can be coloured the way they are everywhere else in the window, and
/// suppressed entirely when all three are zero: `+0/-0` is a fact nobody needs
/// on eleven rows at once.
function Stat({ stat }: { stat: DiffStat }) {
  if (stat.added === 0 && stat.removed === 0 && stat.untracked === 0) return null;
  return (
    <span className="wt-stat" title="against the base branch">
      {stat.added > 0 && <span className="added">+{stat.added}</span>}
      {stat.removed > 0 && <span className="removed">−{stat.removed}</span>}
      {stat.untracked > 0 && (
        <span className="untracked" title={`${stat.untracked} untracked`}>
          ?{stat.untracked}
        </span>
      )}
    </span>
  );
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
