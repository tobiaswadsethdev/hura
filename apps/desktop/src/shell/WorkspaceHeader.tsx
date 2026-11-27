// The header over the conversation, as T3 Code draws it (its `ChatHeader` and
// `PanelLayoutControls`): where you are as a breadcrumb, the project and then
// the worktree, and on the right the toggle for the panel beside it. With the
// sidebar collapsed, the toggle that brings it back sits at its left.
//
// 52 pixels, the sidebar's head beside it, and the window's drag region on
// macOS: the title bar is these two, side by side.

import type { ReactNode } from "react";

import { PanelToggle, SidebarToggle } from "../icons";
import { cn } from "../lib/cn";
import { Button } from "../ui/button";
import { Toggle } from "../ui/toggle";

type Keys = { label: string; aria: string };

export function WorkspaceHeader({
  sidebarOpen,
  onSidebar,
  project,
  onProject,
  title,
  titleHint,
  panel,
  children,
}: {
  sidebarOpen: boolean;
  onSidebar: () => void;
  /// The project the worktree is in, and starting another worktree there.
  project: string | null;
  onProject: (() => void) | null;
  title: string | null;
  /// What the title stands for, when it is not the name: the slug.
  titleHint?: string;
  /// The panel's toggle, when there is a worktree for it to be about.
  panel: { open: boolean; keys: Keys; onToggle: () => void } | null;
  /// Anything else that belongs in the header, at its right.
  children?: ReactNode;
}) {
  return (
    <header
      data-tw
      data-tauri-drag-region
      className={cn(
        "flex h-[52px] shrink-0 items-center gap-3 bg-background px-3 sm:px-5",
        !sidebarOpen && "in-[.mac]:pl-[86px]",
      )}
    >
      {!sidebarOpen && (
        <Button
          variant="ghost"
          size="icon-sm"
          className="-ml-2 text-muted-foreground"
          title="Show the Sidebar"
          aria-label="Show the sidebar"
          onClick={onSidebar}
        >
          <SidebarToggle />
        </Button>
      )}
      <ol className="flex min-w-0 flex-1 items-center gap-3 text-sm font-medium" aria-label="Where you are">
        {project && (
          <>
            <li className="min-w-0 shrink">
              {onProject ? (
                <button
                  className="block max-w-full truncate text-muted-foreground hover:text-foreground"
                  title={`New Worktree in ${project}`}
                  onClick={onProject}
                >
                  {project}
                </button>
              ) : (
                <span className="block truncate text-muted-foreground">{project}</span>
              )}
            </li>
            <li aria-hidden className="text-icon-muted">
              /
            </li>
          </>
        )}
        {title && (
          <li className="min-w-0 truncate text-foreground" title={titleHint} aria-current="page">
            {title}
          </li>
        )}
      </ol>
      {children}
      {panel && (
        <Toggle
          size="sm"
          variant="ghost"
          pressed={panel.open}
          onPressedChange={panel.onToggle}
          title={`${panel.open ? "Hide" : "Show"} the Panel${panel.keys.label ? ` (${panel.keys.label})` : ""}`}
          aria-label="Panel"
          aria-keyshortcuts={panel.keys.aria || undefined}
          className="-mr-2"
        >
          <PanelToggle />
        </Toggle>
      )}
    </header>
  );
}
