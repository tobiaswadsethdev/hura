// How the window's width is shared out when it is narrower than the sidebars
// want.
//
// The tree, the dock and the tickets sidebar each have a width somebody chose,
// and none of them gives way on its own: they are `flex: none`, so that a long
// branch name in the middle cannot take width off them. That left nothing to
// give way when the *window* is narrow except the middle, and it did, all the
// way: with wide sidebars in a window of 1280 pixels the chat was squeezed to
// nothing, and the dock ran on underneath the tickets sidebar with its fold
// button drawn on top of the tickets' header.
//
// So the widths drawn are worked out here, from the ones wanted and the
// window's own, in this order:
//
// 1. Everything as wanted, when that leaves the middle `MIDDLE_MIN` wide.
// 2. Otherwise the open sidebars narrow, each in proportion to how far it is
//    above its narrowest, until the middle has that.
// 3. When even at their narrowest they leave it less, whichever of the dock
//    and the tickets sidebar was opened less recently folds to its rail.
// 4. The other one is never folded for room. A sidebar somebody has just
//    opened stays open, and in a window too narrow for even that, the middle
//    has what is left.
//
// None of this is written back. The preferences are what was asked for, and
// widening the window again brings back exactly that: a laptop's screen does
// not get to overwrite the layout chosen on a wider one.

import { useEffect, useState } from "react";

import { LIMITS } from "./prefs";

/// The narrowest the middle may get before a sidebar gives way. Exactly what
/// the window has always left it at its default size with the default widths
/// (1280 less 260, 400 and 300), so a window that fitted before is drawn as it
/// was; and still a chat's composer with its controls on one line.
export const MIDDLE_MIN = 320;

/// A folded sidebar: the rail of glyphs `.dock-rail` and `.inbox-rail` draw.
/// Borders are inside every width here (`box-sizing: border-box`), and the
/// handles between the panes take none, so these numbers add up to the
/// window's.
export const RAIL = 40;

/// The dock and the tickets sidebar: the two that fold.
export type Folding = "dock" | "inbox";

export type Wanted = {
  /// The window's width, which is `main`'s.
  room: number;
  /// Whether the workspace is showing. While a screen has its place, the tree
  /// and the dock are hidden and take no room.
  workspace: boolean;
  tree: number;
  /// `null` when there is no dock to draw: no worktree ready to look at.
  dock: { open: boolean; width: number } | null;
  inbox: { open: boolean; width: number };
  /// The one of the two that stays open when there is room for only one.
  keep: Folding;
};

export type Fitted = {
  tree: number;
  /// How wide the dock is drawn, or `null` when it is folded or not there.
  dock: number | null;
  /// How wide the tickets sidebar is drawn, or `null` when it is folded.
  inbox: number | null;
  /// The one folded here for want of room, though it was asked to be open.
  folded: Folding | null;
  /// How much wider than `MIDDLE_MIN` the middle is: what a handle may give
  /// one sidebar before the middle would have less. Below zero when it
  /// already does.
  spare: number;
};

type Pane = { width: number; min: number };

/// The panes narrowed to fit in `room`, each in proportion to how far it is
/// above its narrowest, and no further than that.
function narrow(panes: Pane[], room: number): number[] {
  const want = panes.reduce((sum, p) => sum + p.width, 0);
  if (want <= room) return panes.map((p) => p.width);
  const floor = panes.reduce((sum, p) => sum + p.min, 0);
  if (floor >= room) return panes.map((p) => p.min);
  const share = (room - floor) / (want - floor);
  // Down, so a pixel lost to rounding goes to the middle and never past the
  // window's edge.
  return panes.map((p) => p.min + Math.floor((p.width - p.min) * share));
}

export function fit(wanted: Wanted): Fitted {
  const { room, workspace } = wanted;
  const dock = workspace ? wanted.dock : null;

  const lay = (dockOpen: boolean, inboxOpen: boolean, folded: Folding | null): Fitted => {
    // In the order they are drawn, though only the sum matters.
    const panes: (Pane & { key: "tree" | Folding })[] = [];
    if (workspace) panes.push({ key: "tree", width: wanted.tree, ...LIMITS.treeWidth });
    if (dock && dockOpen) panes.push({ key: "dock", width: dock.width, ...LIMITS.dockWidth });
    if (inboxOpen) panes.push({ key: "inbox", width: wanted.inbox.width, ...LIMITS.inboxWidth });
    const rails = (dock && !dockOpen ? RAIL : 0) + (inboxOpen ? 0 : RAIL);
    const widths = narrow(panes, room - rails - MIDDLE_MIN);
    const drawn = (key: "tree" | Folding) => {
      const at = panes.findIndex((p) => p.key === key);
      return at < 0 ? null : widths[at];
    };
    const used = widths.reduce((sum, w) => sum + w, 0) + rails;
    return {
      // Hidden behind a screen, the tree keeps the width it will come back at.
      tree: drawn("tree") ?? wanted.tree,
      dock: drawn("dock"),
      inbox: drawn("inbox"),
      folded,
      spare: room - used - MIDDLE_MIN,
    };
  };

  const dockOpen = dock?.open ?? false;
  const inboxOpen = wanted.inbox.open;
  const all = lay(dockOpen, inboxOpen, null);
  if (all.spare >= 0 || !(dockOpen && inboxOpen)) return all;
  return wanted.keep === "dock" ? lay(true, false, "inbox") : lay(false, true, "dock");
}

/// The window's width, as it changes.
export function useRoom(): number {
  const [room, setRoom] = useState(() => window.innerWidth);
  useEffect(() => {
    const resized = () => setRoom(window.innerWidth);
    window.addEventListener("resize", resized);
    return () => window.removeEventListener("resize", resized);
  }, []);
  return room;
}
