// The sidebar, in T3 Code's shape: its `SidebarChrome` and `SidebarThreadHeader`
// (apps/web/src/components/sidebar there), with hura's contents.
//
// Top to bottom: a head with the toggle that collapses it and the name, which
// is the way to the about screen and says which build this is; a search field
// that narrows the list as it is typed into, with the two ways to start
// something beside it; the list (`Tree`, passed in); and a foot of icons for
// everything that is not a worktree. Tickets and pull requests are two of
// those icons, with their counts, as T3 Code has Issues and Pull requests
// there: they open the tickets sidebar on that view, and a second press puts
// it away.
//
// The width is the one the window's preferences hold, fitted by `layout.ts`;
// the handle on its right edge is `Split`, drawn by `App` beside it.

import type { ComponentType, ReactNode } from "react";

import {
  Account,
  Close,
  Find,
  Integrations,
  NewProject,
  NewSession,
  PullRequest,
  Servers,
  UsageGlyph,
  Settings,
  SidebarToggle,
  Tracker,
} from "../icons";
import { cn } from "../lib/cn";
import { Button } from "../ui/button";

/// The screens the foot opens, by the names `App` keeps them under.
export type Place = "search" | "integrations" | "servers" | "account" | "settings" | "usage";

type Keys = { label: string; aria: string };

export function Sidebar({
  width,
  version,
  aboutOn,
  onAbout,
  onCollapse,
  update,
  query,
  onQuery,
  newProject,
  newSession,
  place,
  onPlace,
  serverReady,
  searchKeys,
  tickets,
  pulls,
  servers,
  children,
}: {
  width: number;
  version: string | null;
  aboutOn: boolean;
  onAbout: () => void;
  onCollapse: () => void;
  /// The update badge, when there is one to show.
  update: ReactNode;
  query: string;
  onQuery: (query: string) => void;
  newProject: { disabled: boolean; onClick: () => void };
  /// A new worktree in the project in front, when there is one to put it in.
  newSession: { project: string | null; onClick: () => void };
  /// The screen showing, if it is one of the foot's.
  place: string | null;
  onPlace: (place: Place | null) => void;
  /// Whether a server is chosen, which three of the screens need.
  serverReady: boolean;
  searchKeys: Keys;
  tickets: Count;
  pulls: Count;
  /// The server picker, when there is more than one server to pick from.
  servers: ReactNode;
  children: ReactNode;
}) {
  const go = (to: Place) => () => onPlace(place === to ? null : to);
  return (
    <aside
      data-tw
      className="sidebar flex h-full min-w-0 shrink-0 flex-col bg-sidebar text-sidebar-foreground"
      style={{ width }}
      aria-label="Sidebar"
    >
      <div
        data-tauri-drag-region
        className="flex h-[52px] shrink-0 items-center gap-3 pr-3 pl-3 in-[.mac]:pl-[86px]"
      >
        <Button
          variant="ghost"
          size="icon-sm"
          className="text-sidebar-muted-foreground"
          title="Hide the Sidebar"
          aria-label="Hide the sidebar"
          onClick={onCollapse}
        >
          <SidebarToggle />
        </Button>
        <button
          className={cn(
            "flex h-7 min-w-0 items-center gap-1.5 rounded-md px-1.5 text-sm font-medium tracking-tight hover:bg-sidebar-row-hover",
            aboutOn && "bg-sidebar-row-active",
          )}
          title="About hura"
          onClick={onAbout}
        >
          hura
          {version && <span className="text-xs font-normal text-sidebar-muted-foreground tabular-nums">{version}</span>}
        </button>
        <span className="ml-auto flex items-center">{update}</span>
      </div>

      <div className="flex shrink-0 items-center gap-1 px-2 pb-2">
        <label className="flex h-8 min-w-0 flex-1 cursor-text items-center gap-2 rounded-md px-2 py-1.5 text-sm font-medium text-sidebar-muted-foreground focus-within:bg-sidebar-row-hover focus-within:text-sidebar-foreground focus-within:ring-1 focus-within:ring-ring/70 focus-within:ring-inset hover:bg-sidebar-row-hover hover:text-sidebar-foreground">
          <Find className="size-4 shrink-0" />
          <input
            className="h-full min-w-0 flex-1 rounded-none border-0 bg-transparent p-0 text-sm font-medium text-sidebar-foreground outline-none placeholder:text-sidebar-muted-foreground"
            placeholder="Search"
            aria-label="Search worktrees"
            spellCheck={false}
            value={query}
            onChange={(e) => onQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && query) {
                e.preventDefault();
                onQuery("");
              }
            }}
          />
          {query && (
            <button
              className="flex size-4 shrink-0 items-center justify-center rounded-sm hover:text-sidebar-foreground"
              title="Clear"
              aria-label="Clear the search"
              onClick={() => onQuery("")}
            >
              <Close className="size-3" />
            </button>
          )}
        </label>
        <Button
          variant="ghost"
          size="icon-sm"
          className="text-sidebar-muted-foreground"
          disabled={newProject.disabled}
          title="New Project"
          aria-label="New project"
          onClick={newProject.onClick}
        >
          <NewProject />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          className="text-sidebar-muted-foreground"
          disabled={newSession.project === null}
          title={newSession.project ? `New Worktree in ${newSession.project}` : "New Worktree"}
          aria-label="New worktree"
          onClick={newSession.onClick}
        >
          <NewSession />
        </Button>
      </div>

      {children}

      {servers && <div className="shrink-0 px-3 pt-1">{servers}</div>}
      <nav className="flex shrink-0 items-center gap-0.5 px-2 pt-1 pb-2" aria-label="Destinations">
        <Foot icon={Settings} label="Settings" on={place === "settings"} disabled={!serverReady} onClick={go("settings")} />
        <Foot
          icon={Tracker}
          label="Tickets"
          on={tickets.on}
          count={tickets.count}
          warned={tickets.warned}
          keys={tickets.keys}
          onClick={tickets.onClick}
        />
        <Foot
          icon={PullRequest}
          label="Pull Requests"
          on={pulls.on}
          count={pulls.count}
          keys={pulls.keys}
          onClick={pulls.onClick}
        />
        <Foot icon={Find} label="Search Conversations" keys={searchKeys} on={place === "search"} onClick={go("search")} />
        <Foot icon={UsageGlyph} label="Usage" on={place === "usage"} disabled={!serverReady} onClick={go("usage")} />
        <Foot
          icon={Integrations}
          label="Integrations"
          on={place === "integrations"}
          disabled={!serverReady}
          onClick={go("integrations")}
        />
        <Foot icon={Servers} label="Servers" on={place === "servers"} onClick={go("servers")} />
        <Foot icon={Account} label="Account" on={place === "account"} onClick={go("account")} />
      </nav>
    </aside>
  );
}

export type Count = {
  count: number;
  /// Whether the tickets sidebar is open on this view.
  on: boolean;
  /// A tracker could not be read.
  warned?: boolean;
  keys?: Keys;
  onClick: () => void;
};

/// One of the foot's icons: T3 Code's `SidebarUtilityItem`, a 32 pixel ghost
/// button, lit while what it opens is showing, with a count in its corner
/// when it has one.
function Foot({
  icon: Mark,
  label,
  on,
  disabled,
  count,
  warned,
  keys,
  onClick,
}: {
  icon: ComponentType<{ className?: string }>;
  label: string;
  on: boolean;
  disabled?: boolean;
  count?: number;
  warned?: boolean;
  keys?: Keys;
  onClick: () => void;
}) {
  return (
    <Button
      variant="ghost"
      size="icon"
      className={cn(
        "relative text-sidebar-muted-foreground/80 hover:text-sidebar-foreground",
        on && "bg-sidebar-row-active text-sidebar-foreground",
      )}
      disabled={disabled}
      title={keys?.label ? `${label} (${keys.label})` : label}
      aria-label={count !== undefined ? `${label}, ${count}` : label}
      aria-keyshortcuts={keys?.aria}
      aria-current={on ? "page" : undefined}
      onClick={onClick}
    >
      <Mark />
      {count !== undefined && count > 0 && (
        <span className="pointer-events-none absolute -top-0.5 -right-0.5 h-3.5 min-w-3.5 rounded-full bg-sidebar px-1 text-center text-[9px] leading-3.5 font-semibold text-sidebar-foreground tabular-nums ring-1 ring-sidebar-border">
          {count}
        </span>
      )}
      {warned && (
        <span
          className="pointer-events-none absolute right-1 bottom-1 size-1.5 rounded-full bg-warning"
          title="A tracker could not be read"
        />
      )}
    </Button>
  );
}
