// The right-hand sidebar: everything about the selected worktree that is not a
// tab.
//
// Five views over one strip. Files and git are where you work *from* -- look at
// what changed, open it, come back -- and facts, policy and events are what is
// true about the worktree. They share a sidebar rather than a tab bar in the
// middle because the middle is for what you have open: a diff you are reading
// should not have to give up its place so you can see what else changed.
//
// Policy and events stay one click away and never behind a tab in the editor,
// which is the point: the isolation being visible is why this is worth building
// rather than adopting an ADE built on git worktrees, and a denial you have to
// go looking for is one you will not find.

import { useState } from "react";

import { cn } from "./lib/cn";

import { FileTree } from "./FileTree";
import type { Against } from "./gen/Against";
import type { Session } from "./gen/Session";
import type { Usage } from "./gen/Usage";
import { GitView } from "./GitView";
import { Branch, Events, Files, Policy, Ports, Record } from "./icons";
import { Facts } from "./panes/Facts";
import { PolicyPane } from "./panes/Policy";
import { EventsPane } from "./panes/Events";
import { PortsPane } from "./panes/Ports";
import type { Listening } from "./gen/Listening";

/// The panel's tabs, in the order the strip shows them, each its glyph and
/// its name as T3 Code's right panel has them.
///
/// The sandbox is one tab and not two: what it let through and refused
/// (Activity, the feed) and the rules it decided by (Rules, the policy) are
/// one subject, the reason this product exists rather than an ADE built on git
/// worktrees, and one place to look for it is easier to find than two. A
/// shield, because that is what the rules are.
const VIEWS = [
  { key: "files", label: "Files", icon: Files },
  { key: "git", label: "Git", icon: Branch },
  { key: "sandbox", label: "Sandbox", icon: Policy },
  { key: "ports", label: "Ports", icon: Ports },
  { key: "facts", label: "Facts", icon: Record },
] as const;
type View = (typeof VIEWS)[number]["key"];

/// What a worktree on the server has nothing to show for: no proxy decides
/// its requests, no policy holds it, and its ports are the server's own.
const SANDBOX_ONLY: readonly View[] = ["sandbox", "ports"];

export function Dock({
  width,
  open,
  server,
  session,
  usage,
  listening,
  refreshMs,
  onOpenFile,
  onOpenDiff,
  onPreview,
}: {
  /// How wide, in pixels. From the window's own preferences and written by the
  /// handle on this sidebar's left edge -- see `prefs.ts` and `Split.tsx`.
  width: number;
  /// Open, or folded to a rail of the panes' glyphs -- the way the tickets
  /// sidebar folds, and for the same reason: the middle wants the width back.
  open: boolean;
  server: string;
  session: Session;
  /// What this session has spent, from the status channel. `null` until its
  /// agent's status line has run once.
  usage: Usage | null;
  /// What the sandbox is listening on, from the status poll.
  listening: Listening[];
  /// The window's refresh interval, which the events feed and git re-read on.
  refreshMs: number;
  onOpenFile: (path: string) => void;
  onOpenDiff: (path: string, against: Against) => void;
  /// A port's page in a tab of the window: see `panes/Preview.tsx`.
  onPreview: (port: number, url: string) => void;
}) {
  // Files first: it is the one you reach for without having decided anything
  // yet.
  const [chosen, setView] = useState<View>("files");
  // Within the sandbox tab: what it did, or what it decides by.
  const [sandboxView, setSandboxView] = useState<"activity" | "rules">("activity");
  const sandboxed = session.backend !== "worktree";
  const views = VIEWS.filter(({ key }) => sandboxed || !SANDBOX_ONLY.includes(key));
  // A pane picked on a sandbox, then a worktree selected: back to the files.
  const view: View = views.some(({ key }) => key === chosen) ? chosen : "files";

  // Folded, nothing: the toggle that brings it back is in the header over the
  // conversation (`shell/WorkspaceHeader.tsx`). Still mounted, so the pane
  // picked before folding is the one showing after.
  if (!open) return null;

  return (
    <aside className="dock" style={{ width }}>
      {/* T3 Code's right panel strip: 52 pixels, level with the header beside
          it, a tab per pane with its glyph and its name, the one showing
          lifted by a fill. */}
      <nav
        data-tw
        data-tauri-drag-region
        className="flex h-[52px] shrink-0 items-center gap-1 overflow-x-auto px-2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
        aria-label="What is true about this worktree"
      >
        {views.map(({ key, label, icon: Mark }) => (
          <button
            key={key}
            className={cn(
              "inline-flex h-6 max-w-36 shrink-0 cursor-pointer items-center gap-1.5 rounded-md pr-2 pl-1.5 text-xs",
              key === view
                ? "bg-accent text-foreground"
                : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
            )}
            // Which one is showing, not a control held down.
            aria-current={key === view ? "page" : undefined}
            onClick={() => setView(key)}
          >
            <Mark className="size-3 shrink-0" />
            <span className="truncate">{label}</span>
            {key === "ports" && listening.length > 0 && (
              <span className="text-muted-foreground tabular-nums">{listening.length}</span>
            )}
          </button>
        ))}
      </nav>
      <div className="dock-body">
        {/* Keyed on the worktree so switching tears each down rather than
            repointing it: a file tree half-expanded into one worktree is not a
            file tree into another. */}
        {view === "files" && (
          <FileTree key={session.name} server={server} name={session.name} onOpen={onOpenFile} />
        )}
        {view === "git" && (
          <GitView
            key={session.name}
            server={server}
            name={session.name}
            refreshMs={refreshMs}
            onOpenDiff={onOpenDiff}
          />
        )}
        {view === "facts" && <Facts session={session} usage={usage} />}
        {view === "sandbox" && (
          <div data-tw className="mb-3 inline-flex rounded-lg bg-input/40 p-0.5" role="tablist" aria-label="Sandbox">
            {(
              [
                ["activity", "Activity", Events],
                ["rules", "Rules", Policy],
              ] as const
            ).map(([key, label, Mark]) => (
              <button
                key={key}
                role="tab"
                aria-selected={sandboxView === key}
                className={cn(
                  "inline-flex h-6 cursor-pointer items-center gap-1.5 rounded-md px-2.5 text-xs",
                  sandboxView === key
                    ? "bg-background text-foreground shadow-xs/10 dark:bg-input/72"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={() => setSandboxView(key)}
              >
                <Mark className="size-3" />
                {label}
              </button>
            ))}
          </div>
        )}
        {view === "sandbox" && sandboxView === "rules" && <PolicyPane server={server} name={session.name} />}
        {view === "ports" && (
          <PortsPane
            key={session.name}
            server={server}
            name={session.name}
            listening={listening}
            onPreview={onPreview}
          />
        )}
        {view === "sandbox" && sandboxView === "activity" && (
          <EventsPane
            key={session.name}
            server={server}
            name={session.name}
            refreshMs={refreshMs}
          />
        )}
      </div>
    </aside>
  );
}
