// The editor area: one tab bar per worktree, and whatever is open in it.
//
// Tabs are per worktree rather than global. A tab is a thing you have open *in*
// a working copy -- its terminal, its diff, and soon a file in it -- so
// switching worktree switches the set, and coming back finds it as you left it.
//
// The agent's tab is one of them rather than a fixture: it can be closed, which
// stops the agent, and another started from `+`. A worktree with nothing open
// says so, and offers the same.

import { useCallback, useEffect, useRef, useState } from "react";

import { useButtonMenu } from "./ContextMenu";
import { Empty } from "./Empty";
import { ChatPane } from "./panes/Chat";
import { FilePane } from "./panes/File";
import { FileDiffPane } from "./panes/FileDiff";
import type { Against } from "./gen/Against";
import type { PullRequest as Pull } from "./gen/PullRequest";
import type { Usage } from "./gen/Usage";
import { Agent, Chat, Close, DrawerToggle, Plus, PreviewHere, Shell } from "./icons";
import { PreviewPane } from "./panes/Preview";
import { TerminalPane } from "./panes/Terminal";
import { AgentMark } from "./agents";
import { keysOf, pressed } from "./keys";

export type Tab =
  /// A terminal in the sandbox. `tmux` names which session, and `null` is the
  /// agent's own -- so a shell is a second tab rather than a second pane.
  | { kind: "terminal"; tmux: string | null; label: string }
  /// A conversation in a chat session. `agent` is the one the task started,
  /// which is the agent in the way the terminal agent's tab is, and closes the
  /// same way as the rest.
  | { kind: "chat"; conv: string; label: string }
  | { kind: "file"; path: string }
  /// One file's diff, side by side. `against` is part of the key: the staged
  /// and unstaged diffs of one file are two different questions, and opening
  /// one should not replace the other.
  | { kind: "filediff"; path: string; against: Against }
  /// A port in the sandbox, its page drawn in the tab. `url` is this
  /// machine's end of the port's preview.
  | { kind: "preview"; port: number; url: string };

export function keyOf(tab: Tab): string {
  switch (tab.kind) {
    case "terminal":
      return `terminal:${tab.tmux ?? "agent"}`;
    case "chat":
      return `chat:${tab.conv}`;
    case "file":
      return `file:${tab.path}`;
    case "filediff":
      return `filediff:${tab.against}:${tab.path}`;
    case "preview":
      return `preview:${tab.port}`;
  }
}

export function labelOf(tab: Tab): string {
  switch (tab.kind) {
    case "terminal":
    case "chat":
      return tab.label;
    case "file":
      return tab.path.split("/").pop() ?? tab.path;
    case "filediff":
      return `${tab.path.split("/").pop() ?? tab.path} ~`;
    case "preview":
      return `localhost:${tab.port}`;
  }
}

/// Dispatched on the window to open or shut the drawer, from the palette.
export const TOGGLE_DRAWER = "hura:toggle-drawer";

/// A shell beside the agent: drawn in the drawer under the conversation, as
/// T3 Code's terminals are, rather than as a tab over it.
const isShell = (tab: Tab) => tab.kind === "terminal" && tab.tmux !== null;

/// The drawer's height, kept between runs: T3 Code's 280 to start, and never
/// less than 180 or more than three quarters of the window.
const DRAWER_KEY = "hura.drawerHeight";
const drawerLimits = () => ({ min: 180, max: Math.max(180, Math.round(window.innerHeight * 0.75)) });
const clampHeight = (h: number) => {
  const { min, max } = drawerLimits();
  return Math.min(max, Math.max(min, Math.round(h)));
};

export function Tabs({
  server,
  name,
  agent,
  onBranch,
  tabs,
  active,
  onActivate,
  onNewShell,
  onNewChat,
  onNewAgent,
  onCloseShell,
  onCloseAgent,
  onCloseChat,
  onCloseFile,
  onReorder,
  failure,
  listed,
  usage,
  pull,
  onPull,
  jump = null,
  onJumped,
}: {
  server: string;
  name: string;
  /// Which agent the worktree runs (`Session.agent`), for its mark.
  agent: string;
  /// "Edit from here" in a conversation: see `ChatPane`.
  onBranch: (from: string | null, at: string | null, text: string) => void;
  tabs: Tab[];
  active: string;
  /// Why the session failed, when its sandbox is still there to look into.
  /// Above the tabs rather than in place of them, unlike a dead session's:
  /// a clone the sandbox denied is explained in the events pane, and the
  /// reason says to go and look there.
  failure?: string | null;
  /// Whether the sandbox has said what is open in it yet. Until then no tabs
  /// is not yet nothing, and the empty editor would only flash past.
  listed: boolean;
  /// The session's, for a chat composer to show the account's limits.
  usage: Usage | null;
  /// The open pull request the worktree is on, for a chat to show over its
  /// composer.
  pull: Pull | null;
  onPull: (pull: Pull) => void;
  /// An entry a search found in one of this worktree's conversations, for
  /// that conversation's pane to open at. See `ChatPane`.
  jump?: { conv: string; seq: number; nonce: number } | null;
  onJumped?: () => void;
  onActivate: (key: string) => void;
  onNewShell: () => void;
  /// Another conversation, in a chat session. Absent in a terminal session,
  /// which has no host to hold one, and then `+` is a shell as it always was.
  onNewChat?: () => void;
  /// Another agent, in a terminal session whose last was closed. Absent while
  /// one is running, since there is only the one, and in a chat session, whose
  /// agents are its conversations.
  onNewAgent?: () => void;
  /// Closing a shell kills what is running in it.
  onCloseShell: (tmux: string) => void;
  /// Closing the agent's terminal stops the agent.
  onCloseAgent: () => void;
  /// Ending a conversation forgets it, the agent's own included.
  onCloseChat: (conv: string) => void;
  onCloseFile: (path: string) => void;
  /// Put `moved` where `onto` currently is. The order lives with the caller
  /// because the tab list is derived -- see `tabsFor` -- so this says what the
  /// user did and lets the caller decide what that means for a list it rebuilds
  /// from the sandbox on every poll.
  onReorder: (moved: string, onto: string) => void;
}) {
  /// The tab being dragged, and the one it is currently over. Local because
  /// nothing outside the bar can see a drag in progress: it ends in an
  /// `onReorder` or in nothing at all.
  const [dragging, setDragging] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);

  const endDrag = () => {
    setDragging(null);
    setOver(null);
  };

  const menu = useButtonMenu();
  const choices = [
    ...(onNewChat ? [{ label: "Chat", icon: Chat, run: onNewChat }] : []),
    ...(onNewAgent ? [{ label: "Agent", icon: Agent, run: onNewAgent }] : []),
    { label: "Shell", icon: Shell, run: onNewShell },
  ];
  const several = choices.length > 1;
  const add = several ? menu(() => choices) : onNewShell;

  // The tabs over the conversation, and the shells in the drawer under it.
  const mains = tabs.filter((t) => !isShell(t));
  const shells = tabs.filter(isShell);
  // `active` is one key for the worktree; a shell made active (a new one, say)
  // is shown in the drawer, and the tab over the conversation stays as it was.
  const lastMain = useRef<string | null>(null);
  const isMain = (key: string | null) => key !== null && mains.some((t) => keyOf(t) === key);
  if (isMain(active)) lastMain.current = active;
  const mainActive = isMain(active)
    ? active
    : isMain(lastMain.current)
      ? lastMain.current
      : mains[0]
        ? keyOf(mains[0])
        : null;

  // Whether the drawer is open, and which shell it shows, per worktree.
  const [drawers, setDrawers] = useState<Record<string, { open: boolean; shell: string | null }>>({});
  const drawer = drawers[name] ?? { open: false, shell: null };
  const setDrawer = useCallback(
    (change: Partial<{ open: boolean; shell: string | null }>) =>
      setDrawers((all) => ({ ...all, [name]: { ...(all[name] ?? { open: false, shell: null }), ...change } })),
    [name],
  );
  useEffect(() => {
    if (shells.some((t) => keyOf(t) === active)) setDrawer({ open: true, shell: active });
    // Only when the active key changes: a shell opened is a shell to show.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);
  const shown = shells.find((t) => keyOf(t) === drawer.shell) ?? shells[0] ?? null;
  const shownKey = shown ? keyOf(shown) : null;
  const drawerShown = drawer.open && shells.length > 0;
  // Open with a shell in it: the first one opened if there are none yet.
  const toggleDrawer = useCallback(() => {
    if (drawerShown) setDrawer({ open: false });
    else if (shells.length) setDrawer({ open: true });
    else onNewShell();
  }, [drawerShown, shells.length, setDrawer, onNewShell]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (pressed(e, "terminal")) {
        e.preventDefault();
        toggleDrawer();
      }
    };
    window.addEventListener("keydown", key);
    window.addEventListener(TOGGLE_DRAWER, toggleDrawer);
    return () => {
      window.removeEventListener("keydown", key);
      window.removeEventListener(TOGGLE_DRAWER, toggleDrawer);
    };
  }, [toggleDrawer]);

  const [height, setHeight] = useState(() => clampHeight(Number(localStorage.getItem(DRAWER_KEY)) || 280));
  const grip = (e: React.PointerEvent<HTMLDivElement>) => {
    const from = e.clientY;
    const start = height;
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    let now = start;
    const move = (m: PointerEvent) => {
      now = clampHeight(start + from - m.clientY);
      setHeight(now);
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      try {
        localStorage.setItem(DRAWER_KEY, String(now));
      } catch {
        // The window works; it just will not remember.
      }
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  return (
    <section className="editor">
      <nav className="tabs">
        {mains.map((tab) => {
          const key = keyOf(tab);
          const agent_ = tab.kind === "terminal" && tab.tmux === null;
          const shell = tab.kind === "terminal" && tab.tmux !== null ? tab.tmux : null;
          const conv = tab.kind === "chat" ? tab.conv : null;
          // A preview closes as a file does: it is this window's alone.
          const file = tab.kind === "file" || tab.kind === "filediff" || tab.kind === "preview" ? keyOf(tab) : null;
          /// Middle click closes exactly what the cross closes, and nothing
          /// else. The agent and a conversation are asked about first, by the
          /// caller, so a stray middle click costs a dialog and not the work.
          const close = agent_
            ? onCloseAgent
            : shell
              ? () => onCloseShell(shell)
              : conv
                ? () => onCloseChat(conv)
                : file
                  ? () => onCloseFile(file)
                  : null;
          return (
            <span
              key={key}
              className={`tab${key === mainActive ? " on" : ""}${dragging === key ? " dragging" : ""}${
                over === key && dragging !== key ? " over" : ""
              }`}
              draggable
              onDragStart={(e) => {
                setDragging(key);
                // Move rather than copy, so the cursor says what will happen,
                // and some text on the clipboard because Firefox will not start
                // a drag without it.
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData("text/plain", key);
              }}
              onDragOver={(e) => {
                // Both needed: without the default prevented the drop never
                // fires, and this is also the only event that reports where the
                // pointer now is.
                e.preventDefault();
                e.dataTransfer.dropEffect = "move";
                setOver(key);
              }}
              onDrop={(e) => {
                e.preventDefault();
                if (dragging && dragging !== key) onReorder(dragging, key);
                endDrag();
              }}
              // Fires whether the drag ended in a drop or was abandoned, which
              // is what makes it the one place the highlight has to come off.
              onDragEnd={endDrag}
              onAuxClick={(e) => {
                if (e.button !== 1 || !close) return;
                // Middle click pastes the selection on X11 and autoscrolls on
                // Windows; neither is wanted on a tab.
                e.preventDefault();
                close();
              }}
            >
              {/* Draggable as well as the pill around it: a button does not
                  always start a drag of its draggable ancestor -- WebKit is the
                  one that matters here -- and the label is most of the tab, so
                  it is where a drag will actually be started from. The event
                  bubbles to the handler above either way. */}
              <button
                draggable
                onClick={() => onActivate(key)}
                title={tab.kind === "preview" ? tab.url : (file ?? undefined)}
              >
                {/* A conversation, or the agent's own terminal, wears its
                    agent's mark. */}
                {(conv !== null || agent_) && <AgentMark agent={agent} title={false} className="tab-mark" />}
                {tab.kind === "preview" && <PreviewHere aria-hidden className="tab-mark" />}
                {labelOf(tab)}
              </button>
              {agent_ && (
                <button className="close" title="Stop the agent, and close its terminal" onClick={onCloseAgent}>
                  <Close aria-label="Close" />
                </button>
              )}
              {shell && (
                <button
                  className="close"
                  title="Close this shell, and whatever is running in it"
                  onClick={() => onCloseShell(shell)}
                >
                  <Close aria-label="Close" />
                </button>
              )}
              {conv && (
                <button
                  className="close"
                  title="End this conversation, and forget it"
                  onClick={() => onCloseChat(conv)}
                >
                  <Close aria-label="Close" />
                </button>
              )}
              {file && (
                <button className="close" title="Close" onClick={() => onCloseFile(file)}>
                  <Close aria-label="Close" />
                </button>
              )}
            </span>
          );
        })}
        <button
          className="add"
          title={
            onNewChat
              ? "Another conversation, or a shell"
              : onNewAgent
                ? "An agent, or a shell"
                : "Another shell in this sandbox"
          }
          aria-haspopup={several ? "menu" : undefined}
          onClick={add}
        >
          <Plus aria-label={several ? "New Tab" : "New Shell"} />
        </button>
        <button
          className={`add tabs-drawer${drawerShown ? " on" : ""}`}
          title={`${drawerShown ? "Hide" : "Show"} the Terminal${keysOf("terminal").label ? ` (${keysOf("terminal").label})` : ""}`}
          aria-label="Terminal"
          aria-pressed={drawerShown}
          onClick={toggleDrawer}
        >
          <DrawerToggle />
        </button>
      </nav>
      {failure && <p className="error tabs-failure">{failure}</p>}

      {/* Everything closed: the agent and the shells. The sandbox is still
          there, and so is the way back into it. */}
      {listed && mains.length === 0 && (
        <Empty size="page" icon={onNewChat ? Chat : Agent} note="Nothing running here">
          <div className="tabs-empty">
            {(onNewChat ?? onNewAgent) && (
              <button className="go" onClick={onNewChat ?? onNewAgent}>
                {onNewChat ? <Chat /> : <Agent />}
                {onNewChat ? "New Chat" : "New Agent"}
              </button>
            )}
            <button className="quiet" onClick={onNewShell}>
              <Shell />
              New Shell
            </button>
          </div>
        </Empty>
      )}

      {mains.map((tab) => {
        const key = keyOf(tab);
        // Rendered and hidden rather than unmounted: a terminal that is
        // unmounted closes its channel and detaches, so switching to the diff
        // and back would lose the screen and re-attach. Only the terminal
        // actually needs this, but treating every tab the same means the next
        // kind added cannot get it wrong.
        return (
          <div key={key} className="tab-body" hidden={key !== mainActive}>
            {tab.kind === "terminal" && (
              <TerminalPane
                key={`${name}:${tab.tmux ?? "agent"}`}
                server={server}
                name={name}
                tmux={tab.tmux}
              />
            )}
            {tab.kind === "chat" && (
              <ChatPane
                key={`${name}:chat:${tab.conv}`}
                server={server}
                name={name}
                conv={tab.conv}
                agent={agent}
                onBranch={onBranch}
                usage={usage}
                pull={pull}
                onPull={onPull}
                jump={jump?.conv === tab.conv ? jump : null}
                onJumped={onJumped}
              />
            )}
            {tab.kind === "filediff" && (
              <FileDiffPane
                key={keyOf(tab)}
                server={server}
                name={name}
                path={tab.path}
                against={tab.against}
              />
            )}
            {tab.kind === "file" && (
              <FilePane key={`${name}:${tab.path}`} server={server} name={name} path={tab.path} />
            )}
            {tab.kind === "preview" && <PreviewPane key={`${name}:${tab.url}`} url={tab.url} port={tab.port} />}
          </div>
        );
      })}
      {/* The drawer, T3 Code's: the shells under the conversation, a strip of
          them across its top, and a grip on its edge to make it taller.
          Hidden rather than unmounted when shut, for the reason the tabs are:
          a terminal unmounted is a terminal detached from tmux. */}
      <div className="drawer" hidden={!drawerShown} style={{ height }}>
        <div
          className="drawer-grip"
          role="separator"
          aria-orientation="horizontal"
          aria-label="Terminal height"
          onPointerDown={grip}
          onDoubleClick={() => {
            setHeight(280);
            localStorage.removeItem(DRAWER_KEY);
          }}
        />
        <nav className="drawer-tabs" aria-label="Shells">
          {shells.map((tab) => {
            const key = keyOf(tab);
            const tmux = tab.kind === "terminal" ? tab.tmux : null;
            return (
              <span key={key} className={`drawer-tab${key === shownKey ? " on" : ""}`}>
                <button onClick={() => setDrawer({ shell: key })}>
                  <Shell aria-hidden />
                  {labelOf(tab)}
                </button>
                {tmux && (
                  <button
                    className="close"
                    title="Close this shell, and whatever is running in it"
                    onClick={() => onCloseShell(tmux)}
                  >
                    <Close aria-label="Close" />
                  </button>
                )}
              </span>
            );
          })}
          <button className="drawer-add" title="Another Shell" aria-label="New Shell" onClick={onNewShell}>
            <Plus />
          </button>
          <button
            className="drawer-add drawer-hide"
            title={`Hide the Terminal${keysOf("terminal").label ? ` (${keysOf("terminal").label})` : ""}`}
            aria-label="Hide the terminal"
            onClick={() => setDrawer({ open: false })}
          >
            <Close />
          </button>
        </nav>
        {shells.map((tab) => {
          const key = keyOf(tab);
          return (
            <div key={key} className="tab-body" hidden={key !== shownKey}>
              {tab.kind === "terminal" && (
                <TerminalPane
                  key={`${name}:${tab.tmux ?? "agent"}`}
                  server={server}
                  name={name}
                  tmux={tab.tmux}
                />
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}
