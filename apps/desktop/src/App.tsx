// The workspace: projects on the left, what is open in the middle, what is
// true about the worktree beside it, and what is yours on your trackers on the
// far right.
//
// Shaped like an editor rather than like the terminal interface it grew out of.
// The list of sessions was the right thing when a session was the unit of work;
// it stopped being right once there were four repositories in it, because a
// flat list sorted by name says nothing about which four. So: projects contain
// worktrees, a worktree contains what you have open in it, and the dock carries
// the two things no ADE built on git worktrees has -- the policy being enforced
// and the decisions it made.
//
// **The window has destinations now, and the workspace is one of them.** The
// integrations, the servers and the settings used to be modal
// dialogs stacked over this; they are screens, and `Screen.tsx` says at length
// why. What that costs here is the two things a router has to do: hold which
// screen is showing, and keep the workspace *mounted* while another one is --
// see `hidden` on `.workspace` below, which is the load-bearing half. The
// sidebar of tickets and pull requests is not a destination: it stays beside
// whatever is showing, which is the reason it replaced a tickets screen.

import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api, type HistoryHit, messageOf, type Paired, type ServerSummary, type Shells } from "./api";
import { forgetSession } from "./cache";
import { useConfirm } from "./Confirm";
import { Dock } from "./Dock";
import { Empty } from "./Empty";
import {
  Account,
  Branch,
  DrawerToggle,
  Find,
  Forget,
  Integrations,
  NewProject,
  NewSession,
  NoSandbox,
  NoServer,
  PanelToggle,
  PreviewHere,
  PullRequest as PullGlyph,
  Servers,
  Settings as SettingsGlyph,
  SettingsAppearance,
  SettingsKeys,
  UsageGlyph,
  SidebarToggle,
  Tracker,
  Working,
} from "./icons";
import { CommandPalette, type PaletteItem } from "./shell/CommandPalette";
import { useTriage } from "./triage";
import { useGroups } from "./groups";
import { noteServerOffset } from "./repeats";
import { type Script, useScripts } from "./scripts";
import { pullName } from "./inboxNotify";
import { InboxBar, pullProjects, pullSession, samePull, sessionPull } from "./Inbox";
import { IntegrationsScreen } from "./Integrations";
import { ServersScreen } from "./Connect";
import type { ConfiguredTracker } from "./gen/ConfiguredTracker";
import type { Inbox } from "./gen/Inbox";
import type { InboxChange } from "./gen/InboxChange";
import type { Project } from "./gen/Project";
import type { Session } from "./gen/Session";
import type { PullRequest } from "./gen/PullRequest";
import type { Task } from "./gen/Task";
import type { Poll } from "./gen/Poll";
import { onSessions, reset as resetNotifications } from "./notify";
import { onCloudChanges } from "./inboxNotify";
import { close, nextChannelId, open } from "./stream";
import { NewProjectDialog } from "./NewProject";
import { forgetChat, sayFirst } from "./panes/Chat";
import { NewWorktreeDraft } from "./NewWorktree";
import { fit, type Folding, useRoom } from "./layout";
import { Sidebar } from "./shell/Sidebar";
import { WorkspaceHeader } from "./shell/WorkspaceHeader";
import { SCRIPT_GLYPHS, ScriptsControl } from "./shell/ScriptsControl";
import { DEFAULTS, FLYOUT_GAP, flyoutMax, LIMITS, type Prefs, usePrefs } from "./prefs";
import { SearchScreen } from "./Search";
import { GeneralPage, SessionsPage, TrackersPage } from "./Settings";
import { AppearancePage, DiffsPage, KeybindingsPage } from "./AppearanceSettings";
import { UsagePage } from "./UsagePage";
import { ProjectsPage } from "./ProjectsPage";
import { SettingsModal, type SettingsPageId } from "./settings/SettingsModal";
import { AccountScreen } from "./Account";
import { PullPanel, type PullStart } from "./Pull";
import { TicketPanel } from "./Ticket";
import { Split } from "./Split";
import type { Against } from "./gen/Against";
import type { QueueView } from "./gen/QueueView";
import { keyOf, Tabs, type Tab } from "./Tabs";
import { group, Tree } from "./Tree";
import { UpdateBadge } from "./Update";
import { AboutScreen } from "./About";
import { refreshPreviews } from "./previews";
import { Select } from "./Select";
import { applyAppearance, applyTerminalsDark } from "./theme";
import { applyCustomTheme, applyDiffs, applyFonts, applyZoom } from "./look";
import { applyKeys, keysOf, pressed, useKeys } from "./keys";
import { TOGGLE_DRAWER } from "./Tabs";

/// Where in the window you are.
///
/// `null` is the workspace, rather than a `"workspace"` member, and the
/// asymmetry is deliberate: the workspace is not a screen you navigate to, it
/// is what the window *is* when nothing is over it. Writing it as one of six
/// equal names would invite code that tears it down to show another, which is
/// the one thing that must not happen -- the terminals live in it.
type Screen = "search" | "integrations" | "servers" | "account" | "settings" | "about" | "tickets" | "pulls";

/// How often your tickets and pull requests are read. See the effect that
/// uses it.
const INBOX_POLL_MS = 3 * 60_000;

/// How often the worktree list is re-read, and the events feed with it. Fixed
/// rather than a preference: it was one, and three seconds was the answer
/// everybody left it at.
const REFRESH_MS = 3000;

// The shortcuts are `keys.ts`'s, where Settings can change them.

/// The tabs a worktree has open.
///
/// Derived from the sandbox rather than remembered here: what shells exist is a
/// fact about the sandbox, and one that outlives this window. A tab list kept
/// in the client would show a shell that had been closed from elsewhere and
/// hide one opened from elsewhere.
///
/// A chat session's agent is its first conversation rather than a terminal, and
/// the others it has are asked of the host the same way the shells are asked
/// of tmux. Either agent can have been closed. A terminal agent's tab waits
/// for the answer, because attaching to a closed one starts another; a chat's
/// is assumed until then, since attaching to a closed conversation is only
/// refused.
function tabsFor(
  session: Session,
  shells: Shells | undefined,
  chats: string[] | undefined,
  open: Tab[],
  /// What a shell a script was run in is called, by its tmux name.
  named: Record<string, string> = {},
): Tab[] {
  const agent: Tab[] =
    session.interface === "chat"
      ? (chats ?? ["agent"]).map((conv): Tab => ({ kind: "chat", conv, label: conv === "agent" ? "Agent" : conv }))
      : shells?.agent
        ? [{ kind: "terminal", tmux: null, label: "Agent" }]
        : [];
  return [
    ...agent,
    ...(shells?.shells ?? []).map((tmux): Tab => ({ kind: "terminal", tmux, label: named[tmux] ?? tmux })),
    // Files and diffs last, in the order they were opened -- the one thing here
    // that is genuinely this window's state. A file is open because someone in
    // *this* window clicked it, and nothing in the sandbox knows that.
    ...open,
  ];
}

/// Put the tabs in the order the user dragged them into.
///
/// An overlay on `tabsFor` rather than a replacement for it, because the two
/// know different things: that one knows which tabs *exist*, and it rebuilds
/// them from the sandbox on every poll, so a remembered list of tabs would show
/// shells that had been closed from elsewhere. This knows only an order, and a
/// key in it that no longer exists is simply skipped.
///
/// Anything the order has never seen keeps its place from `tabsFor` -- appended
/// here, which is where a new shell or a newly opened file belongs anyway.
function arrange(tabs: Tab[], order: string[]): Tab[] {
  const byKey = new Map(tabs.map((t) => [keyOf(t), t]));
  const placed = order.flatMap((key) => {
    const tab = byKey.get(key);
    if (!tab) return [];
    byKey.delete(key);
    return [tab];
  });
  return [...placed, ...byKey.values()];
}

/// The order after `moved` is dropped onto `onto`.
///
/// Taken from the tabs as they are currently shown rather than from the stored
/// order, so the first drag in a worktree -- where nothing is stored yet --
/// starts from what the user can actually see.
function reordered(tabs: Tab[], moved: string, onto: string): string[] {
  const keys = tabs.map(keyOf).filter((key) => key !== moved);
  const at = keys.indexOf(onto);
  if (at < 0) return [...keys, moved];
  return [...keys.slice(0, at), moved, ...keys.slice(at)];
}


/// One catch-up at a time, and a session asked for again while it is waiting
/// asked once: two turns ending together would otherwise open two connections
/// to replay the same entries.
let catching: Promise<void> = Promise.resolve();
const queued = new Set<string>();
function catchUp(server: string, name: string) {
  const key = `${server}\n${name}`;
  if (queued.has(key)) return;
  queued.add(key);
  catching = catching.then(() => {
    queued.delete(key);
    return api.historyCatchUp(server, name).catch(() => {});
  });
}

export default function App() {
  // How wide the sidebars are, how often the list is re-read, and whether the
  // OS is told when an agent waits. This window's own, kept on this machine:
  // see `prefs.ts` for why none of it is on the server beside the branch
  // prefix.
  const [prefs, setPrefs] = usePrefs();

  // Light, dark or the system's. Applied once already in `main.tsx`, before
  // the first paint; this is for when it is changed in Settings.
  // A theme taken in from VS Code is in its own light or dark, and its colours
  // go over the palette that picks; see `look.ts`.
  useEffect(() => applyAppearance(prefs.theme?.scheme ?? prefs.appearance), [prefs.appearance, prefs.theme]);
  useEffect(() => applyTerminalsDark(prefs.terminalsDark), [prefs.terminalsDark]);
  useEffect(() => applyCustomTheme(prefs.theme), [prefs.theme]);
  useEffect(
    () => applyFonts({ ui: prefs.uiFont, mono: prefs.monoFont, codeSize: prefs.codeSize }),
    [prefs.uiFont, prefs.monoFont, prefs.codeSize],
  );
  const zoomed = useRef(prefs.zoom);
  useEffect(() => {
    // Not on the first render: `main.tsx` has, and 1 needs nothing.
    if (zoomed.current === prefs.zoom) return;
    zoomed.current = prefs.zoom;
    applyZoom(prefs.zoom);
  }, [prefs.zoom]);
  useEffect(() => applyKeys(prefs.keys), [prefs.keys]);
  useEffect(
    () => applyDiffs({ stacked: prefs.diffStacked, hideWhitespace: prefs.diffHideWhitespace }),
    [prefs.diffStacked, prefs.diffHideWhitespace],
  );
  useKeys();

  /// Asking before something irreversible, in a dialog of this window's own.
  /// `dialog` is rendered at the end, beside the others.
  const { ask, dialog: confirmation } = useConfirm();

  const [servers, setServers] = useState<ServerSummary[] | null>(null);
  const [server, setServer] = useState<string | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [sessions, setSessions] = useState<Session[]>([]);
  // What is waiting on the server: sessions to start and messages to say.
  // Null from a server too old to have a queue, which then shows none.
  const [queue, setQueue] = useState<QueueView | null>(null);
  // The server's clock, for a repeating worktree's time of day. See `repeats.ts`.
  useEffect(() => noteServerOffset(queue?.utc_offset), [queue?.utc_offset]);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  /// Which destination is showing, if any. One value rather than the four
  /// booleans this used to be: they were four states that had to be false
  /// together, and nothing enforced it -- opening the tickets from the settings
  /// screen put both on screen at once, each with its own scrim.
  const [screen, setScreen] = useState<Screen | null>(null);
  // Which of the two pages the tickets shortcut opens: the one last shown.
  const inboxViewRef = useRef<"tickets" | "pulls">("tickets");
  /// The settings screen opened at its trackers, because the sidebar's "add a
  /// tracker" is what opened it.
  /// The settings card's page, kept while it is shut so it opens where it
  /// was left, as T3 Code's does. The card is `screen === "settings"`.
  const [settingsPage, setSettingsPage] = useState<SettingsPageId>("general");
  const openSettings = (page: SettingsPageId) => {
    setSettingsPage(page);
    setScreen("settings");
  };
  /// Whether a screen is in the workspace's place. Settings are not: they
  /// float over it, and it stays drawn behind them.
  const covered = screen !== null && screen !== "settings";
  /// This window's own version, for the wordmark. Asked once: it cannot change
  /// without a restart.
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    api.about(null).then((a) => setVersion(a.desktop), () => {});
  }, []);
  // The previews this window is running, for the tree's globes. Re-read on
  // a slow timer as well, since one ends on its own when the connection to
  // the server does.
  useEffect(() => {
    if (!server) return;
    void refreshPreviews(server).catch(() => {});
    const id = setInterval(() => void refreshPreviews(server).catch(() => {}), 10_000);
    return () => clearInterval(id);
  }, [server]);
  const [creatingProject, setCreatingProject] = useState(false);
  // Pinned, settled and snoozed worktrees: see `triage.ts`.
  const [triage, setTriageOf] = useTriage(server);
  const worktreeGroups = useGroups(server);
  // The command palette (`shell/CommandPalette.tsx`), on Command+K or Ctrl+K.
  const [palette, setPalette] = useState(false);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (!pressed(e, "palette")) return;
      // Ctrl+K in a terminal is the shell's kill-line, and stays its own.
      if (e.ctrlKey && (document.activeElement as Element | null)?.closest(".xterm")) return;
      e.preventDefault();
      e.stopPropagation();
      setPalette((open) => !open);
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, []);
  /// The project a new worktree is being drafted in: the draft takes the
  /// workspace's place, as T3 Code's new thread does (`NewWorktree.tsx`).
  const [creatingIn, setCreatingIn] = useState<Project | null>(null);
  const startDraft = (project: Project) => {
    setScreen(null);
    setCreatingIn(project);
  };
  /// Leave the draft: for a worktree, or because it was sent.
  const leaveDraft = () => {
    setCreatingIn(null);
    setFromTask(null);
    setFromPull(null);
  };
  const goToWorktree = (name: string) => {
    leaveDraft();
    setSelected(name);
  };
  // A draft taken to another server goes with it: to the same repository's
  // project there, or the first one, or away when the server has none.
  useEffect(() => {
    if (!creatingIn || projects.some((p) => p.name === creatingIn.name && p.repo === creatingIn.repo)) return;
    const there = projects.find((p) => p.repo === creatingIn.repo) ?? projects[0] ?? null;
    if (there) setCreatingIn(there);
    else setCreatingIn(null);
    // Only when the projects change: picking one is `creatingIn` changing.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projects]);
  /// The ticket a create was started from, carried from the sidebar to the form.
  const [fromTask, setFromTask] = useState<Task | null>(null);
  /// Or the pull request, and whether the session is to work on it or review it.
  const [fromPull, setFromPull] = useState<{ pull: PullRequest; mode: PullStart } | null>(null);
  // Every worktree's last poll: what its agent is doing, what it has spent, and
  // how full its context is. From a status channel each rather than a request
  // on a timer -- the server polls the sandbox and sends a frame only when
  // something changed, which is the whole reason that channel exists.
  //
  // **Every session and not only the selected one**, because the state that
  // matters most is the one you are *not* looking at: a session waiting on a
  // permission prompt is the thing this window exists to tell you about, and
  // the session list itself cannot say it. `Ls` reports what the record says --
  // `ready`, `idle` -- and the agent's own state is only ever in a poll.
  const [polls, setPolls] = useState<Record<string, Poll>>({});

  /// The shells each worktree has, and which tab is in front of it. Both are
  /// per worktree: switching away and back finds it as you left it, because a
  /// shell you opened in one is not a shell in another.
  const [shells, setShells] = useState<Record<string, Shells>>({});
  /// The shells scripts were run in, by worktree and then tmux name: the
  /// drawer calls each by its script's name.
  const [scriptShells, setScriptShells] = useState<Record<string, Record<string, string>>>({});
  /// A chat session's conversations beside the agent's, the same way.
  const [chats, setChats] = useState<Record<string, string[]>>({});
  const [files, setFiles] = useState<Record<string, Tab[]>>({});
  const [active, setActive] = useState<Record<string, string>>({});
  /// Tab keys per worktree, in the order they were dragged into. Window state
  /// like `files` and not persisted: see `arrange`.
  const [order, setOrder] = useState<Record<string, string[]>>({});
  /// A search result being opened: the conversation's pane takes it, opens at
  /// the entry and says so, which clears it. See `ChatPane`.
  const [jump, setJump] = useState<{ server: string; session: string; conv: string; seq: number; nonce: number } | null>(
    null,
  );
  const jumped = useCallback(() => setJump(null), []);

  useEffect(() => {
    api
      .servers()
      .then((list) => {
        setServers(list);
        setServer((current) => current ?? list[0]?.name ?? null);
      })
      .catch((e) => setError(messageOf(e)));
  }, []);

  // Escape leaves whatever screen is open, in one place rather than in each of
  // them. Four copies of this listener is how two of them end up disagreeing
  // about what Escape does -- and the dialogs keep their own, because a dialog
  // can be open *over* a screen and has to be the one that closes.
  //
  // An Escape something inside has already answered -- a context menu, a
  // dropdown, the ticket beside the board -- is marked `defaultPrevented` and
  // left alone: closing a menu must not also close the screen under it.
  useEffect(() => {
    // The settings card closes itself, after its way out.
    if (screen === null || screen === "settings") return;
    const onKey = (e: KeyboardEvent) =>
      e.key === "Escape" && !e.defaultPrevented && setScreen(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [screen]);

  // The diff each worktree carries, lifted out of the polls for the tree.
  //
  // A projection rather than the whole record: `Poll` also carries the agent's
  // captured screen, and handing the tree a prop it does not read would make
  // the tree's own types claim it might. `polls` is a new object on every
  // status frame, so this recomputes about as often either way -- the point is
  // the narrower prop, not a saved comparison.
  const session = sessions.find((s) => s.name === selected) ?? null;
  // Selected but not ready to be looked at: asked for and not in the list yet,
  // or in it and still being made. Every pane asks the sandbox about a working
  // copy, and before the clone there is none -- so they are not mounted until
  // there is, rather than each drawing its own error for half a minute.
  const preparing =
    (selected !== null && session === null) ||
    session?.state === "creating" ||
    session?.state === "seeding";
  // Selected and never going to be ready: its sandbox is gone, or was never
  // made. Every pane would ask the runtime about it and draw `sandbox not
  // found`, which says what is missing and not why, so they are not mounted
  // and the reason is shown instead.
  const gone = session?.state === "dead";

  // How wide each sidebar is drawn, and which are folded: what was asked for,
  // fitted to the window. See `layout.ts`, and why none of it is written back.
  const room = useRoom();
  // Which of the dock and the tickets sidebar stays open when only one fits:
  // the one opened last, so that a click on a rail is never undone by the
  // layout. The dock until then, since it is about the worktree in the middle.
  const [keep, setKeep] = useState<Folding>("dock");
  // Whether a dock is drawn at all: the branches of the workspace below that
  // end in one.
  const hasDock = !covered && creatingIn === null && session !== null && server !== null && !preparing && !gone;
  const fitted = fit({
    room,
    workspace: !covered,
    tree: prefs.treeOpen ? prefs.treeWidth : null,
    dock: hasDock ? { open: prefs.dockOpen, width: prefs.dockWidth } : null,
    // Tickets and pull requests are pages now (`screen`), not a column.
    inbox: { open: false, width: prefs.inboxWidth },
    keep,
  });
  // Open as drawn, not as asked for. With no dock on screen, the asking is all
  // there is.
  const inboxOpen = screen === "tickets" || screen === "pulls";
  if (screen === "tickets" || screen === "pulls") inboxViewRef.current = screen;
  const dockOpen = hasDock ? fitted.dock !== null : prefs.dockOpen;
  // Where the ticket and pull request flyouts stand: against the sidebar, or
  // against its folded rail.
  const flyoutRight = fitted.inbox ?? 0;

  // A fold button, a rail's way back, or a shortcut. Opening one makes it the
  // one kept when only one fits, so the other folds rather than it. Folding one
  // also folds for good the other, if that was folded only for room: asking for
  // more middle must not bring it back to take the room just given.
  const toggle = useCallback(
    (which: Folding) => {
      if (!(which === "dock" ? dockOpen : inboxOpen)) {
        setKeep(which);
        setPrefs(which === "dock" ? { dockOpen: true } : { inboxOpen: true });
        return;
      }
      setPrefs({
        ...(which === "dock" ? { dockOpen: false } : { inboxOpen: false }),
        ...(fitted.folded === "dock" && { dockOpen: false }),
        ...(fitted.folded === "inbox" && { inboxOpen: false }),
      });
    },
    [dockOpen, inboxOpen, fitted.folded, setPrefs],
  );
  // The sidebar's Tickets and Pull Requests: the tickets sidebar on that view,
  // or put away when it is already showing it.
  const showInbox = (view: typeof prefs.inboxView) => {
    setPrefs({ inboxView: view });
    setScreen((now) => (now === view ? null : view));
  };
  // Where the sidebar's new-worktree button starts one: the project in front,
  // or the first there is.
  const newIn = projects.find((p) => p.name === session?.project) ?? projects[0] ?? null;

  // A handle dragged. The other sidebars are written at the widths they are
  // drawn at, so that the one dragged follows the pointer: left at what was
  // asked for, they would be narrowed afresh against its new width, and a drag
  // outwards could draw it narrower.
  const resize = (change: Partial<Prefs>) =>
    setPrefs({
      ...(fitted.tree !== null && { treeWidth: fitted.tree }),
      ...(fitted.dock !== null && { dockWidth: fitted.dock }),
      ...(fitted.inbox !== null && { inboxWidth: fitted.inbox }),
      ...change,
    });
  // As far as a handle may take its sidebar: what the middle can spare.
  const widest = (limit: { min: number; max: number }, drawn: number) =>
    Math.max(limit.min, Math.min(limit.max, drawn + Math.max(0, fitted.spare)));

  // The sidebars' shortcuts. In the capture phase, so it is this window's
  // before a focused terminal takes the key and sends it to the sandbox. Not
  // while a dialog is up: that is asking a question, and the window would
  // change under it.
  const dialogOpen = creatingProject || confirmation !== null;
  /// What the shortcuts without a place of their own do, as of the last
  /// render: set below, once what they need is defined.
  const keyActions = useRef({ sidebar: () => {}, newWorktree: () => {}, runScript: () => {} });
  useEffect(() => {
    if (!server || dialogOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (pressed(e, "search")) {
        e.preventDefault();
        e.stopPropagation();
        setScreen((now) => (now === "search" ? null : "search"));
        return;
      }
      if (pressed(e, "tickets")) {
        e.preventDefault();
        e.stopPropagation();
        setScreen((now) => (now === "tickets" || now === "pulls" ? null : inboxViewRef.current));
        return;
      }
      // Unbound until somebody binds them in Settings.
      for (const command of ["sidebar", "newWorktree", "runScript"] as const) {
        if (pressed(e, command)) {
          e.preventDefault();
          e.stopPropagation();
          keyActions.current[command]();
          return;
        }
      }
      const which: Folding | null = pressed(e, "panel") ? "dock" : null;
      if (!which) return;
      e.preventDefault();
      e.stopPropagation();
      toggle(which);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [server, dialogOpen, toggle]);

  // Keyed on the *names*, joined, rather than on the array: the list is a new
  // array every few seconds and re-subscribing to four sandboxes that often
  // would be worse than not subscribing at all.
  //
  // Not a dead session's: there is no sandbox to poll, and the server would
  // ask the runtime about it every two seconds for as long as the row is
  // listed, to hear `sandbox not found` each time.
  const names = sessions
    .filter((s) => s.state !== "dead")
    .map((s) => s.name)
    .join("\u0000");
  useEffect(() => {
    if (!server) return;
    let live = true;
    const open_ids: number[] = [];
    for (const name of names.split("\u0000").filter(Boolean)) {
      const id = nextChannelId();
      open_ids.push(id);
      void open(server, id, { kind: "status", session: name }, (frame) => {
        if (live && frame.is === "status") {
          setPolls((all) => ({ ...all, [name]: frame.poll }));
        }
      }).catch(() => {
        // A channel that will not open is not worth a message of its own: the
        // list beside it already says whether the server can be reached.
      });
    }
    return () => {
      live = false;
      for (const id of open_ids) void close(id);
    };
  }, [server, names]);

  // What the agent is doing, which is what the tree shows and what decides
  // whether you are interrupted. The record's own state is the fallback: it is
  // what a session that has never been polled has, and the only thing that can
  // say `creating`, `failed` or `dead`.
  const live = useMemo(
    () =>
      sessions.map((s) => {
        const state = polls[s.name]?.status?.state;
        return state && s.state === "ready" ? { ...s, state } : s;
      }),
    [sessions, polls],
  );

  // A chat session's conversations kept, each time a turn in one ends: what
  // arrived in a conversation no pane had open is otherwise only in the
  // sandbox. On the end of a turn rather than a timer, because asking execs
  // into the sandbox, and one that has stopped would be started by being
  // asked -- an agent that has just finished a turn is in one that is running.
  // Removing a session and ending a conversation catch up on the Rust side.
  const turns = useRef<Record<string, string>>({});
  useEffect(() => {
    turns.current = {};
  }, [server]);
  useEffect(() => {
    if (!server) return;
    for (const s of live) {
      if (s.interface !== "chat") continue;
      const was = turns.current[s.name];
      turns.current[s.name] = s.state;
      if (was === "running" && s.state !== "running") catchUp(server, s.name);
    }
  }, [server, live]);

  /// A session that has been asked for but is not in the list yet.
  ///
  /// A ref rather than state: `refresh` reads it, and making it a dependency
  /// would rebuild the poll timer every time a session is created.
  const pending = useRef<string | null>(null);

  const refresh = useCallback(async () => {
    if (!server) return null;
    try {
      const [list, known, waiting] = await Promise.all([
        api.sessions(server),
        api.projects(server),
        api.queue(server).catch(() => null),
      ]);
      setSessions(list);
      setProjects(known);
      setQueue(waiting);
      setError(null);
      // A worktree that has gone should not leave the panes showing its last
      // known state, which is indistinguishable from it still being there.
      // A session still being created is the exception: it is selected before
      // its record exists, and moving off it would take the window away from
      // the thing that was just asked for.
      setSelected((current) =>
        current && (current === pending.current || list.some((s) => s.name === current))
          ? current
          : (list[0]?.name ?? null),
      );
      return list;
    } catch (e) {
      setError(messageOf(e));
      return null;
    }
  }, [server]);

  /// Ask for the list until the new session is in it.
  ///
  /// `create` answers as soon as the request is accepted, and the sandbox is
  /// seconds of runtime after that, on the server's own thread, so the record
  /// appears a moment later and the ordinary poll is seconds away again. Left
  /// to the timer, the sidebar stays exactly as it was for those seconds and a
  /// click that worked looks like one that did not.
  ///
  /// Bounded, and it gives up quietly: a create that fails before it writes
  /// anything says so in the server's log, and the sidebar is then telling the
  /// truth by not listing it.
  const refreshUntil = useCallback(
    async (name: string) => {
      pending.current = name;
      try {
        for (let tries = 0; tries < 20; tries++) {
          const list = await refresh();
          if (list?.some((s) => s.name === name)) return;
          await new Promise((done) => setTimeout(done, 300));
        }
      } finally {
        pending.current = null;
      }
    },
    [refresh],
  );

  useEffect(() => {
    // A different server is a different set of sessions, and the states of the
    // last one's say nothing about them.
    resetNotifications();
    void refresh();
    const timer = setInterval(() => void refresh(), REFRESH_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  /// Pairing from the window, which used to be a terminal and a restart.
  ///
  /// The paired server is selected immediately: someone who has just pasted a
  /// pairing string is asking to look at that server, and the poll below finds
  /// its worktrees within the second. The screen closes with it, because
  /// pairing is the one thing on it that is finished when it works.
  const paired = (p: Paired) => {
    setServers(p.servers);
    setServer(p.server.name);
    setScreen(null);
  };

  /// Forgetting one. Selection moves off it rather than staying on a name that
  /// no longer resolves -- `remote()` on the Rust side would answer "no server
  /// named" to every request after it.
  const forgot = (list: ServerSummary[]) => {
    setServers(list);
    setServer((current) =>
      current && list.some((s) => s.name === current) ? current : (list[0]?.name ?? null),
    );
  };

  // What has just started waiting, which is the whole reason the window
  // subscribes to every session rather than the selected one. Here rather than
  // in the fetch, because the state that matters arrives on the status channel
  // and not in the list.
  useEffect(() => {
    // Handed the list either way, and told whether to say anything about it:
    // `notify.ts` has to keep watching the states while it is turned off, or
    // turning it back on would announce everything that has been waiting since.
    onSessions(live, prefs.notify);
  }, [live, prefs.notify]);

  // What is yours on your trackers, for the sidebar, on one timer. Minutes
  // rather than the session list's seconds: every read is a few searches
  // against somebody else's API, and hura cloud says sooner when something
  // changed (below), which reads it again.
  //
  // Not gated on a server: the trackers are the hura cloud account's, and
  // signed out there are none. A server only lends its branch prefix to the
  // suggested branch names, so the read waits for the list of servers to say
  // which one that is.
  /// Whether this computer is signed in to hura cloud. `null` until the
  /// session file has been read.
  const [signedIn, setSignedIn] = useState<boolean | null>(null);
  const [inbox, setInbox] = useState<Inbox | null>(null);
  const [inboxError, setInboxError] = useState<string | null>(null);
  const [inboxReading, setInboxReading] = useState(false);
  const [trackers, setTrackers] = useState<ConfiguredTracker[] | null>(null);
  /// The ticket open beside the sidebar.
  const [showing, setShowing] = useState<Task | null>(null);
  /// Or the pull request: one at a time, since they open in the same place.
  const [showingPull, setShowingPull] = useState<PullRequest | null>(null);
  // A ticket or pull request read in the panel belongs to the page it was
  // opened from: going to another screen, or back to the work, puts it away.
  const shownOn = useRef(screen);
  useEffect(() => {
    if (shownOn.current === screen) return;
    shownOn.current = screen;
    setShowing(null);
    setShowingPull(null);
  }, [screen]);
  // Read through a ref: turning notifications off is not a reason to ask
  // every tracker again, which a dependency would make it.
  const notifyTickets = useRef(prefs.notifyTickets);
  notifyTickets.current = prefs.notifyTickets;
  // One read at a time: a refresh pressed while the timer's read is out would
  // otherwise be two answers racing to be the one shown.
  const inFlight = useRef(false);
  const readInbox = useCallback(() => {
    if (inFlight.current) return;
    inFlight.current = true;
    setInboxReading(true);
    api
      .tickets(server)
      .then(
        (v) => {
          setInbox(v);
          setInboxError(null);
        },
        (e) => setInboxError(messageOf(e)),
      )
      .finally(() => {
        inFlight.current = false;
        setInboxReading(false);
      });
  }, [server]);
  const ready = servers !== null;
  useEffect(() => {
    api.cloudStatus().then(
      (s) => setSignedIn(s !== null),
      () => setSignedIn(false),
    );
  }, []);
  useEffect(() => {
    if (!ready || signedIn === null) return;
    if (!signedIn) {
      setTrackers([]);
      setInbox(null);
      setInboxError(null);
      return;
    }
    api.trackers().then(setTrackers, () => setTrackers([]));
    readInbox();
    const timer = setInterval(readInbox, INBOX_POLL_MS);
    return () => clearInterval(timer);
  }, [ready, signedIn, readInbox]);

  // What hura cloud finds on the trackers arrives as it finds it: said, and
  // the sidebar read again so it shows the same thing.
  useEffect(() => {
    const off = listen<InboxChange[]>("tracker-changes", (e) => {
      onCloudChanges(e.payload, notifyTickets.current);
      readInbox();
    });
    return () => {
      void off.then((unlisten) => unlisten());
    };
  }, [readInbox]);

  // What reviewers said on your pull requests, to the agents of the worktrees
  // that own them, each time the sidebar is read, when that is switched on.
  // Each pull request is read again in full, because the sidebar's row counts
  // comments only on GitHub and only some of them; the server leaves out what
  // each agent has been sent, so asking every few minutes says each thing
  // once. Yours only: on somebody else's pull request the reviewer is you, or
  // the worktree is reviewing it. Silent when it fails, as a timer has nobody
  // to tell; the panel's button says why.
  const sessionsNow = useRef(live);
  sessionsNow.current = live;
  const telling = useRef(false);
  useEffect(() => {
    if (!prefs.pullCommentsToAgent || !server || !inbox || telling.current) return;
    const owned = inbox.pulls
      .filter((p) => p.relation === "created")
      .flatMap((p) => {
        const s = pullSession(p, sessionsNow.current);
        return s ? [{ pull: p, session: s }] : [];
      });
    if (owned.length === 0) return;
    telling.current = true;
    void (async () => {
      for (const { pull, session } of owned) {
        await api
          .sendPullComments(server, session.name, pull.tracker, pull.repo, pull.id)
          .catch(() => undefined);
      }
    })().finally(() => {
      telling.current = false;
    });
  }, [inbox, server, prefs.pullCommentsToAgent]);

  // Looking at the inbox here is looking at it everywhere: with the window in
  // front and the sidebar open, what is waiting on you (the phone's badge
  // counts the same) is seen, and signed in, the phones clear their badge and
  // the tracker notifications they showed. Sent when that changes.
  const lastSeen = useRef<string | null>(null);
  useEffect(() => {
    const look = () => {
      if (!inboxOpen || !inbox || !document.hasFocus()) return;
      const items = [
        ...inbox.tasks.filter((t) => t.relation === "assigned").map((t) => `${t.tracker}/${t.id}`),
        ...inbox.pulls.filter((p) => p.review_requested).map((p) => `${p.tracker}/${p.id}`),
      ].sort();
      const key = items.join("\n");
      if (key === lastSeen.current) return;
      lastSeen.current = key;
      api.cloudSetSeen(items).catch(() => {
        lastSeen.current = null;
      });
    };
    look();
    window.addEventListener("focus", look);
    return () => window.removeEventListener("focus", look);
  }, [inbox, inboxOpen]);

  const groups = useMemo(
    () => group(projects, live, queue?.sessions ?? []),
    [projects, live, queue],
  );
  // What the sidebar's search field narrows the list to: worktrees whose
  // title, name, branch or task has the words in it, and a project by its
  // name. The groups keep their order and their projects.
  const [query, setQuery] = useState("");
  const shownGroups = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return groups;
    const has = (...values: (string | null | undefined)[]) => values.some((v) => v?.toLowerCase().includes(q));
    return groups.flatMap((g) => {
      if (g.project && has(g.project.name)) return [g];
      const worktrees = g.worktrees.filter((s) => has(s.title, s.name, s.work_branch, s.task));
      const waiting = g.waiting.filter((w) => has(w.new.name, w.new.task));
      return worktrees.length || waiting.length ? [{ ...g, worktrees, waiting }] : [];
    });
  }, [groups, query]);

  // The two asks below follow the name and not the record. The list is
  // fetched again every few seconds and each fetch is a new object, so
  // following `session` asked again on every refresh and threw away the answer
  // still on its way. On a loaded host an answer takes longer than a refresh:
  // the asks piled up in the sandbox, each a process of its own, and none
  // landed.
  const name = session?.name ?? null;

  // Asked once per worktree as it is selected. Not polled: a shell appears
  // because someone in this window asked for one, and paying an exec a second
  // to hear that nothing changed is what the stream exists to avoid.
  useEffect(() => {
    if (!server || !name || preparing || gone || shells[name]) return;
    let live = true;
    api
      .shells(server, name)
      .then((list) => live && setShells((all) => ({ ...all, [name]: list })))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [server, name, preparing, gone, shells]);

  // The conversations, asked once per worktree like the shells, and only of a
  // chat session: a terminal session has no host to ask.
  const chatSession = session?.interface === "chat";
  useEffect(() => {
    if (!server || !name || !chatSession || preparing || gone || chats[name]) return;
    let live = true;
    api
      .chats(server, name)
      .then((list) => live && setChats((all) => ({ ...all, [name]: list.chats })))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [server, name, chatSession, preparing, gone, chats]);

  const openTabs = session
    ? arrange(
        tabsFor(
          session,
          shells[session.name],
          chats[session.name],
          files[session.name] ?? [],
          scriptShells[session.name],
        ),
        order[session.name] ?? [],
      )
    : [];

  /// Destroy a worktree, from its row in the tree or from the page a dead one
  /// shows.
  const destroy = (s: Session) => {
    if (!server) return;
    // Asked, and asked with the consequence spelled out, because this is the
    // one thing in the window that cannot be undone: the sandbox goes, and with
    // it whatever the agent had not pushed. A dead one has nothing left to
    // lose, and saying so is the difference between a click and a hesitation.
    const dead = s.state === "dead";
    ask({
      title: dead ? `Remove ${s.title ?? s.name}?` : `Destroy ${s.title ?? s.name}?`,
      body: dead ? (
        <>Its sandbox is already gone, so this only takes it off the list.</>
      ) : (
        <>Its sandbox goes with it, and anything the agent has not pushed is lost.</>
      ),
      confirm: dead ? "Remove" : "Destroy",
      onConfirm: () => {
        api
          .destroy(server, s.name)
          .then((left) => {
            setSessions(left);
            forgetSession(server, s.name);
            // Whatever was showing is gone. Left selected, the panes would go
            // on asking the server about a session it no longer has.
            if (selected === s.name) setSelected(null);
          })
          .catch((e) => setError(messageOf(e)));
      },
    });
  };

  /// A search result, opened where it happened: its server, its worktree, its
  /// conversation in front, at the message.
  const openHit = (hit: HistoryHit) => {
    if (hit.server !== server) setServer(hit.server);
    goToWorktree(hit.session);
    setActive((a) => ({ ...a, [hit.session]: `chat:${hit.conv}` }));
    setJump({ server: hit.server, session: hit.session, conv: hit.conv, seq: hit.seq, nonce: Date.now() });
    setScreen(null);
  };

  const [scripts] = useScripts(server, session?.project ?? null);
  /// Run one of the project's scripts in a new shell in the worktree in
  /// front, shown in the drawer and called by the script's name.
  const runScript = (script: Script) => {
    if (!server || !session) return;
    const worktree = session.name;
    api
      .newShell(server, worktree, script.command)
      .then((list) => {
        const opened = list.shells.find((t) => !(shells[worktree]?.shells ?? []).includes(t)) ?? list.shells.at(-1);
        if (opened) setScriptShells((all) => ({ ...all, [worktree]: { ...(all[worktree] ?? {}), [opened]: script.name } }));
        setShells((all) => ({ ...all, [worktree]: list }));
        if (opened) setActive((a) => ({ ...a, [worktree]: `terminal:${opened}` }));
      })
      .catch((e) => setError(messageOf(e)));
  };

  /// Open a tab if it is not already open, and bring it to the front either way.
  const openTab = (worktree: string, tab: Tab) => {
    const key = keyOf(tab);
    setFiles((all) => {
      const open = all[worktree] ?? [];
      return open.some((t) => keyOf(t) === key) ? all : { ...all, [worktree]: [...open, tab] };
    });
    setActive((a) => ({ ...a, [worktree]: key }));
  };
  /// The tab in front: the one chosen, while it is still open, and otherwise
  /// the first. Nothing is always there to fall back to -- the agent can be
  /// closed like the rest -- so a closed tab is forgotten rather than replaced.
  const chosen = session ? active[session.name] : undefined;
  const activeTab = openTabs.some((t) => keyOf(t) === chosen)
    ? (chosen as string)
    : openTabs[0]
      ? keyOf(openTabs[0])
      : "";
  /// Stop remembering `key` as the tab in front of `worktree`, if it was.
  const unchoose = (worktree: string, key: string) =>
    setActive((a) => {
      if (a[worktree] !== key) return a;
      const { [worktree]: _, ...rest } = a;
      return rest;
    });

  /// Start a worktree for a ticket: straight into the create form, filled in.
  /// The sidebar's whole point is that starting work on a ticket is one step.
  /// Whatever was over the workspace goes, because the form answers back to
  /// the workspace, which is where the session will appear.
  const startFrom = (project: Project, task: Task) => {
    setScreen(null);
    setShowing(null);
    setShowingPull(null);
    setFromPull(null);
    setFromTask(task);
    setCreatingIn(project);
  };

  /// Read a pull request beside the sidebar, or put it away if it is the one
  /// already open. From its row, or from a worktree's mark in the tree, which
  /// turns the sidebar to pull requests so its row is there, marked.
  const showPull = (pull: PullRequest) => {
    setPrefs({ inboxView: "pulls" });
    setShowing(null);
    setShowingPull((current) => (current && samePull(current, pull) ? null : pull));
  };

  /// The same, for a pull request: a worktree on its branch, to carry on with
  /// it or to review it.
  const startFromPull = (project: Project, pull: PullRequest, mode: PullStart) => {
    setScreen(null);
    setShowing(null);
    setShowingPull(null);
    setFromTask(null);
    setFromPull({ pull, mode });
    setCreatingIn(project);
  };

  // Nothing paired: the one screen a new install is guaranteed to see, and the
  // only empty state in the window that has to be instructions. The glyph
  // carries "there is no server", which is the part that used to be a heading;
  // what is left in words is the two commands, because no picture of a terminal
  // is going to tell anybody to type `hurad pair`.
  //
  // The servers screen is reachable from here and from nowhere else at this
  // point -- the toolbar is empty, because a toolbar of destinations that all
  // need a server would be five disabled buttons. It is there at all because
  // on macOS it is the title bar: the traffic lights sit in it, and it is the
  // one place the window can be dragged by.
  if (servers !== null && servers.length === 0) {
    return (
      <div className="app">
        <header data-tauri-drag-region />
        {screen === "settings" && settingsPage === "servers" ? (
          <ServersScreen
            servers={[]}
            onClose={() => setScreen(null)}
            onPaired={paired}
            onForgot={forgot}
          />
        ) : (
          <Empty size="page" icon={NoServer} note="No server paired">
            <pre>
              hurad serve{"\n"}
              hurad pair desktop --host 127.0.0.1
            </pre>
            {/* Without --host the string carries the machine's own hostname,
                which on a Debian-family box resolves to 127.0.1.1 while the
                server is bound to 127.0.0.1 -- a connection refused from a
                server that is running perfectly well. */}
            <p className="hint">
              <code>--host</code> is the address this window should dial, and
              leaving it out is the usual reason a paired server cannot be
              reached.
            </p>
            <button className="go" onClick={() => openSettings("servers")}>
              <Servers />
              Paste a Pairing String
            </button>
          </Empty>
        )}
      </div>
    );
  }

  // Everything the palette offers, built when it opens from what is here now.
  keyActions.current = {
    sidebar: () => setPrefs({ treeOpen: !prefs.treeOpen }),
    newWorktree: () => newIn && startDraft(newIn),
    runScript: () => {
      if (scripts[0] && !preparing && !gone) runScript(scripts[0]);
    },
  };

  const paletteItems = (): PaletteItem[] => {
    const items: PaletteItem[] = [];
    const go = (to: Screen | null) => () => setScreen(to);
    for (const s of live) {
      items.push({
        id: `wt:${s.name}`,
        group: "Worktrees",
        label: s.title ?? s.name,
        hint: [s.project, polls[s.name]?.branch ?? s.work_branch].filter(Boolean).join(" · "),
        keywords: `${s.name} ${s.task ?? ""}`,
        icon: Branch,
        run: () => {
          setScreen(null);
          goToWorktree(s.name);
        },
      });
    }
    for (const p of projects) {
      items.push({
        id: `new:${p.name}`,
        group: "Projects",
        label: `New Worktree in ${p.name}`,
        hint: p.repo,
        icon: NewSession,
        run: () => startDraft(p),
      });
    }
    for (const t of inbox?.tasks ?? []) {
      items.push({
        id: `t:${t.tracker}:${t.key}`,
        group: "Tickets and Pull Requests",
        label: t.title,
        hint: t.key,
        icon: Tracker,
        run: () => {
          setShowingPull(null);
          setShowing(t);
        },
      });
    }
    for (const pr of inbox?.pulls ?? []) {
      items.push({
        id: `p:${pr.tracker}:${pr.id}`,
        group: "Tickets and Pull Requests",
        label: pr.title,
        hint: pullName(pr),
        icon: PullGlyph,
        run: () => showPull(pr),
      });
    }
    // The project's scripts, to run in the worktree in front.
    if (session && server && !preparing && !gone) {
      for (const sc of scripts) {
        items.push({
          id: `script:${sc.id}`,
          group: "Do",
          label: `Run ${sc.name}`,
          hint: sc.command,
          keywords: "script",
          icon: SCRIPT_GLYPHS[sc.icon],
          run: () => runScript(sc),
        });
      }
    }
    // What the worktree in front is listening on, to look at in a tab.
    if (session && server) {
      for (const l of (selected ? polls[selected]?.ports : undefined) ?? []) {
        items.push({
          id: `preview:${l.port}`,
          group: "Do",
          label: `Preview localhost:${l.port}`,
          hint: session.title ?? session.name,
          keywords: "port browser page",
          icon: PreviewHere,
          run: () => {
            api
              .previewOpen(server, session.name, l.port, l.host)
              .then((p) => openTab(session.name, { kind: "preview", port: l.port, url: p.url }))
              .catch((e) => setError(messageOf(e)));
          },
        });
      }
    }
    const act = (id: string, label: string, run: () => void, icon?: PaletteItem["icon"], keys?: string, keywords?: string) =>
      items.push({ id: `do:${id}`, group: "Do", label, run, icon, keys, keywords });
    act("new-project", "New Project", () => setCreatingProject(true), NewProject);
    if (newIn) act("new-worktree", `New Worktree in ${newIn.name}`, () => startDraft(newIn), NewSession, keysOf("newWorktree").label);
    act("sidebar", prefs.treeOpen ? "Hide the Sidebar" : "Show the Sidebar", () => setPrefs({ treeOpen: !prefs.treeOpen }), SidebarToggle, keysOf("sidebar").label);
    if (hasDock) act("panel", dockOpen ? "Hide the Panel" : "Show the Panel", () => toggle("dock"), PanelToggle, keysOf("panel").label);
    act("terminal", "Show or Hide the Terminal", () => window.dispatchEvent(new Event(TOGGLE_DRAWER)), DrawerToggle, keysOf("terminal").label, "shell drawer");
    act("tickets", "Tickets", go("tickets"), Tracker, keysOf("tickets").label);
    act("pulls", "Pull Requests", go("pulls"), PullGlyph);
    act("search", "Search Conversations", go("search"), Find, keysOf("search").label);
    act("settings", "Settings", () => openSettings("general"), SettingsGlyph);
    act("appearance", "Appearance Settings", () => openSettings("appearance"), SettingsAppearance, undefined, "theme font zoom");
    act("keybindings", "Keybindings", () => openSettings("keybindings"), SettingsKeys, undefined, "shortcuts keys");
    act("usage", "Usage", () => openSettings("usage"), UsageGlyph, undefined, "cost limits spent tokens");
    act("integrations", "Integrations", () => openSettings("integrations"), Integrations, undefined, "mcp skills secrets");
    act("servers", "Servers", () => openSettings("servers"), Servers);
    act("account", "Account", () => openSettings("account"), Account);
    act("about", "About hura", () => openSettings("about"), undefined, undefined, "version update");
    for (const a of ["dark", "light", "system"] as const) {
      if (prefs.appearance !== a) {
        act(`theme-${a}`, `Appearance: ${a === "system" ? "Follow the System" : a === "dark" ? "Dark" : "Light"}`, () => setPrefs({ appearance: a }), undefined, undefined, "theme");
      }
    }
    return items;
  };

  return (
    <div className="app">
      {/* No bar of its own: the sidebar's head and the header over the
          conversation are the top of the window, as in T3 Code. An error
          that used to sit in the bar floats at the top instead. */}
      {error && (
        <div
          role="alert"
          className="fixed top-3 left-1/2 z-50 max-w-[60%] -translate-x-1/2 truncate rounded-lg bg-card px-3 py-1.5 text-xs text-destructive-foreground shadow-lg ring-1 ring-border"
          title={error}
        >
          {error}
        </div>
      )}

      <main>
        {fitted.tree !== null && (
          <>
            <Sidebar
              width={fitted.tree}
              version={version}
              aboutOn={screen === "settings" && settingsPage === "about"}
              onAbout={() =>
                screen === "settings" && settingsPage === "about" ? setScreen(null) : openSettings("about")
              }
              onCollapse={() => setPrefs({ treeOpen: false })}
              update={<UpdateBadge onOpen={() => openSettings("about")} />}
              query={query}
              onQuery={setQuery}
              newProject={{ disabled: !server, onClick: () => setCreatingProject(true) }}
              newSession={{
                project: newIn?.name ?? null,
                onClick: () => newIn && startDraft(newIn),
              }}
              // The foot's settings-like buttons each open the card at their
              // page, and are lit while it shows it.
              place={
                screen === "settings"
                  ? settingsPage === "integrations" ||
                    settingsPage === "servers" ||
                    settingsPage === "account" ||
                    settingsPage === "usage"
                    ? settingsPage
                    : "settings"
                  : screen
              }
              onPlace={(to) => {
                if (to === null || to === "search") setScreen(to);
                else openSettings(to === "settings" ? "general" : to);
              }}
              serverReady={!!server}
              searchKeys={keysOf("search")}
              tickets={{
                count: inbox?.tasks.length ?? 0,
                on: screen === "tickets",
                warned: (inbox?.warnings.length ?? 0) > 0 || inboxError !== null,
                keys: keysOf("tickets"),
                onClick: () => showInbox("tickets"),
              }}
              pulls={{
                count: inbox?.pulls.length ?? 0,
                on: screen === "pulls",
                onClick: () => showInbox("pulls"),
              }}
              servers={
                servers && servers.length > 1 ? (
                  <Select
                    className="server-pick"
                    aria-label="Server"
                    value={server ?? ""}
                    onChange={setServer}
                    options={servers.map((s) => ({ value: s.name, label: s.name, hint: s.address }))}
                  />
                ) : null
              }
            >
              <Tree
                triage={triage}
                onTriage={setTriageOf}
                groups={shownGroups}
                polls={polls}
                selected={creatingIn ? null : selected}
                sets={worktreeGroups}
                onSelect={goToWorktree}
                pulls={inbox?.pulls ?? []}
                onPull={showPull}
                onNewWorktree={setCreatingIn}
                onForget={(p) => {
                  if (!server) return;
                  api
                    .forgetProject(server, p.name)
                    .then(setProjects)
                    .catch((e) => setError(messageOf(e)));
                }}
                onDestroy={destroy}
                onRename={(s, title) => {
                  if (!server) return;
                  api
                    .retitle(server, s.name, title)
                    .then(setSessions)
                    .catch((e) => setError(messageOf(e)));
                }}
                // Drawn in the new order at once, rather than after the round
                // trip: a row that springs back for a moment and then lands where
                // it was dropped reads as a drop that failed. The answer, or the
                // next refresh after a refusal, settles it either way.
                onReorderProjects={(names) => {
                  if (!server) return;
                  setProjects((all) =>
                    [...all].sort((a, b) => names.indexOf(a.name) - names.indexOf(b.name)),
                  );
                  api
                    .reorderProjects(server, names)
                    .then(setProjects)
                    .catch((e) => setError(messageOf(e)));
                }}
                onReorderWorktrees={(names) => {
                  if (!server) return;
                  setSessions((all) =>
                    all.map((s) => (names.includes(s.name) ? { ...s, rank: names.indexOf(s.name) } : s)),
                  );
                  api
                    .reorderSessions(server, names)
                    .then(setSessions)
                    .catch((e) => setError(messageOf(e)));
                }}
                queueLimit={queue?.limit ?? 0}
                messages={queue?.messages ?? []}
                onUnqueue={(w) => {
                  if (!server) return;
                  api
                    .unqueue(server, w.id)
                    .then(setQueue)
                    .catch((e) => setError(messageOf(e)));
                }}
                onStartQueued={(w) => {
                  if (!server) return;
                  api
                    .startQueued(server, w.id)
                    .then(setQueue)
                    .catch((e) => setError(messageOf(e)));
                }}
              />
            </Sidebar>
            <Split
              label="Projects sidebar width"
              value={fitted.tree}
              min={LIMITS.treeWidth.min}
              max={widest(LIMITS.treeWidth, fitted.tree)}
              reset={DEFAULTS.treeWidth}
              grows="right"
              onChange={(treeWidth) => resize({ treeWidth })}
            />
          </>
        )}
        {/* **Hidden, not unmounted, and this is the line that matters.**
            Every terminal in the window lives under here, and a terminal that
            unmounts closes its channel and detaches from tmux -- so opening
            the settings screen would drop the agent's stream and every shell's
            beside it, and coming back would re-attach four of them and repaint.
            Nothing would be lost, because tmux is holding the screen; it would
            just be a window that flickers every time you look at a setting.
            `Tabs.tsx` hides its inactive tabs for the same reason, and this is
            that decision one level up.

            It is also why `Screen` is a sibling here rather than something the
            workspace renders: a screen has to be able to fill the middle
            without the middle being torn down to make room. */}
        <div className="workspace" hidden={covered}>
          {/* The conversation's column, T3 Code's: the header over it and the
              tabs under that. The panel stands beside it at full height. */}
          <div className="workspace-main">
          <WorkspaceHeader
            sidebarOpen={fitted.tree !== null}
            onSidebar={() => setPrefs({ treeOpen: true })}
            project={creatingIn ? creatingIn.name : (session?.project ?? null)}
            onProject={
              !creatingIn && session?.project
                ? () => {
                    const p = projects.find((p) => p.name === session.project);
                    if (p) startDraft(p);
                  }
                : null
            }
            title={creatingIn ? "New Worktree" : session ? (session.title ?? session.name) : null}
            titleHint={!creatingIn && session?.title ? session.name : undefined}
            panel={hasDock ? { open: dockOpen, keys: keysOf("panel"), onToggle: () => toggle("dock") } : null}
          >
            {!creatingIn && session?.project && server && !preparing && !gone && (
              <ScriptsControl server={server} project={session.project} onRun={runScript} />
            )}
          </WorkspaceHeader>
          <div className="workspace-body">
            {creatingIn && server ? (
              <NewWorktreeDraft
                // Another server is a fresh draft, with that server's answers.
                key={server}
                server={server}
                servers={(servers ?? []).map((s) => s.name)}
                onServer={setServer}
                projects={projects}
                project={creatingIn}
                onProject={setCreatingIn}
                canRepeat={queue?.utc_offset != null}
                from={fromTask}
                fromPull={fromPull}
                onClose={leaveDraft}
                onCreated={(name) => {
                  // Selected before it exists, on purpose: the record is
                  // written as soon as the server's thread reaches it, and the
                  // burst below is what puts it in the sidebar rather than the
                  // next poll.
                  goToWorktree(name);
                  void refreshUntil(name);
                }}
                onQueued={(waiting) => {
                  leaveDraft();
                  setQueue(waiting);
                }}
              />
            ) : preparing ? (
              // The create's own progress, said once in the middle: the steps
              // are seconds of runtime and then however long the clone takes,
              // and a spinner with the step beside it is all there is to know.
              <Empty size="page" icon={Working} note={preparingNote(session)} />
            ) : gone && session ? (
              // Why there is nothing here, which is what used to be missing: the
              // panes each said `sandbox not found`, and the reason was on the
              // server's terminal. The mark is grey because it only says what is
              // absent; the reason is red because it is an error.
              <Empty size="page" icon={NoSandbox} note="No sandbox behind this worktree">
                {session.failure && <p className="error">{session.failure}</p>}
                <button className="go" onClick={() => destroy(session)}>
                  <Forget />
                  Remove
                </button>
              </Empty>
            ) : session && server ? (
              <>
                <Tabs
                  server={server}
                  name={session.name}
                  agent={session.agent}
                  onBranch={(from, at, text) => {
                    // A new conversation forked from just before the message
                    // (or from nothing, before the first), in front, with the
                    // edited words said in it.
                    api
                      .newChat(server, session.name, from, at)
                      .then(({ chats: list, opened }) => {
                        setChats((all) => ({ ...all, [session.name]: list }));
                        if (!opened) return;
                        sayFirst(server, session.name, opened, text);
                        setActive((a) => ({ ...a, [session.name]: `chat:${opened}` }));
                      })
                      .catch((e) => setError(messageOf(e)));
                  }}
                  tabs={openTabs}
                  active={activeTab}
                  failure={session.state === "failed" ? session.failure : null}
                  listed={!!shells[session.name] && (session.interface !== "chat" || !!chats[session.name])}
                  usage={polls[session.name]?.usage ?? null}
                  pull={sessionPull(session, inbox?.pulls ?? [])}
                  onPull={showPull}
                  jump={jump && jump.server === server && jump.session === session.name ? jump : null}
                  onJumped={jumped}
                  onActivate={(key) => setActive((a) => ({ ...a, [session.name]: key }))}
                  onReorder={(moved, onto) =>
                    setOrder((all) => ({
                      ...all,
                      [session.name]: reordered(openTabs, moved, onto),
                    }))
                  }
                  onNewShell={() => {
                    api
                      .newShell(server, session.name)
                      .then((list) => {
                        setShells((all) => ({ ...all, [session.name]: list }));
                        // Opened in front, because asking for a shell is asking
                        // to use one.
                        const opened = list.shells[list.shells.length - 1];
                        if (opened) {
                          setActive((a) => ({ ...a, [session.name]: `terminal:${opened}` }));
                        }
                      })
                      .catch((e) => setError(messageOf(e)));
                  }}
                  onNewChat={
                    session.interface === "chat"
                      ? () => {
                          api
                            .newChat(server, session.name)
                            .then(({ chats: list, opened }) => {
                              setChats((all) => ({ ...all, [session.name]: list }));
                              // In front, for the reason a new shell is.
                              if (opened) setActive((a) => ({ ...a, [session.name]: `chat:${opened}` }));
                            })
                            .catch((e) => setError(messageOf(e)));
                        }
                      : undefined
                  }
                  onNewAgent={
                    session.interface !== "chat" && shells[session.name]?.agent === false
                      ? () => {
                          api
                            .newAgent(server, session.name)
                            .then((list) => {
                              setShells((all) => ({ ...all, [session.name]: list }));
                              // In front, for the reason a new shell is.
                              setActive((a) => ({ ...a, [session.name]: "terminal:agent" }));
                            })
                            .catch((e) => setError(messageOf(e)));
                        }
                      : undefined
                  }
                  onCloseAgent={() => {
                    ask({
                      title: "Stop the agent?",
                      body: (
                        <>
                          Claude Code stops, along with whatever it is doing. The worktree stays, and{" "}
                          <b>+</b> starts a new agent in it.
                        </>
                      ),
                      confirm: "Stop It",
                      onConfirm: () => {
                        api
                          .killShell(server, session.name, session.tmux)
                          .then((list) => {
                            setShells((all) => ({ ...all, [session.name]: list }));
                            unchoose(session.name, "terminal:agent");
                          })
                          .catch((e) => setError(messageOf(e)));
                      },
                    });
                  }}
                  onCloseChat={(conv) => {
                    ask({
                      title: conv === "agent" ? "End the agent's conversation?" : `End ${conv}?`,
                      body: <>The conversation stops, and its transcript is forgotten.</>,
                      confirm: "End It",
                      onConfirm: () => {
                        api
                          .closeChat(server, session.name, conv)
                          .then(({ chats: list }) => {
                            setChats((all) => ({ ...all, [session.name]: list }));
                            forgetChat(server, session.name, conv);
                            unchoose(session.name, `chat:${conv}`);
                          })
                          .catch((e) => setError(messageOf(e)));
                      },
                    });
                  }}
                  onCloseFile={(key) => {
                    setFiles((all) => ({
                      ...all,
                      [session.name]: (all[session.name] ?? []).filter((t) => keyOf(t) !== key),
                    }));
                    unchoose(session.name, key);
                  }}
                  onCloseShell={(tmux) => {
                    api
                      .killShell(server, session.name, tmux)
                      .then((list) => {
                        setShells((all) => ({ ...all, [session.name]: list }));
                        // Whatever was in front may have just been killed.
                        unchoose(session.name, `terminal:${tmux}`);
                      })
                      .catch((e) => setError(messageOf(e)));
                  }}
                />
              </>
            ) : projects.length === 0 ? (
              // A glyph that is the button that fixes it, which is as close as an
              // empty state gets to explaining itself: the mark here and the mark
              // on `new project` in the header are the same picture.
              <Empty size="page" icon={NewProject} note="No projects yet">
                <button className="go" disabled={!server} onClick={() => setCreatingProject(true)}>
                  <NewProject />
                  New Project
                </button>
              </Empty>
            ) : (
              // The note survives because it names the control: a `+` that only
              // appears when a project row is hovered is the one thing in the
              // tree that a glyph here cannot point at.
              <Empty
                size="page"
                icon={Branch}
                note={
                  sessions.length === 0
                    ? "Start a worktree with + beside a project"
                    : "Pick a worktree, or start one with + beside a project"
                }
              />
            )}
          </div>
          </div>
          {hasDock && session && server && (
            <>
            {/* Between the editor and the dock, so a drag on it takes width
                from the middle and gives it to the right, which is why
                this one `grows="left"`. */}
            {fitted.dock !== null && (
              <Split
                label="Dock width"
                value={fitted.dock}
                min={LIMITS.dockWidth.min}
                max={widest(LIMITS.dockWidth, fitted.dock)}
                reset={DEFAULTS.dockWidth}
                grows="left"
                onChange={(dockWidth) => resize({ dockWidth })}
              />
            )}
            <Dock
              width={fitted.dock ?? prefs.dockWidth}
              open={dockOpen}
              server={server}
              session={session}
              usage={(selected ? polls[selected]?.usage : undefined) ?? null}
              listening={(selected ? polls[selected]?.ports : undefined) ?? []}
              refreshMs={REFRESH_MS}
              onOpenFile={(path) => openTab(session.name, { kind: "file", path })}
              onOpenDiff={(path, against: Against) =>
                openTab(session.name, { kind: "filediff", path, against })
              }
              onPreview={(port, url) => openTab(session.name, { kind: "preview", port, url })}
            />
            </>
          )}
        </div>

        {screen === "search" && (
          <SearchScreen server={server} sessions={sessions} onOpen={openHit} onClose={() => setScreen(null)} />
        )}

        {screen === "settings" && server && (
          <SettingsModal page={settingsPage} onPage={setSettingsPage} onClose={() => setScreen(null)}>
            {settingsPage === "general" && <GeneralPage prefs={prefs} onPrefs={setPrefs} signedIn={signedIn} />}
            {settingsPage === "appearance" && <AppearancePage prefs={prefs} onPrefs={setPrefs} />}
            {settingsPage === "diffs" && <DiffsPage prefs={prefs} onPrefs={setPrefs} />}
            {settingsPage === "keybindings" && <KeybindingsPage prefs={prefs} onPrefs={setPrefs} />}
            {settingsPage === "usage" && (
              <UsagePage
                sessions={live}
                polls={polls}
                onOpen={(name) => {
                  setScreen(null);
                  goToWorktree(name);
                }}
              />
            )}
            {settingsPage === "sessions" && <SessionsPage server={server} />}
            {settingsPage === "projects" && (
              <ProjectsPage server={server} projects={projects} onProjects={setProjects} />
            )}
            {settingsPage === "integrations" && (
              <IntegrationsScreen server={server} onClose={() => setScreen(null)} />
            )}
            {settingsPage === "servers" && (
              <ServersScreen
                servers={servers ?? []}
                onClose={() => setScreen(null)}
                onPaired={paired}
                onForgot={forgot}
              />
            )}
            {settingsPage === "trackers" && (
              <TrackersPage
                signedIn={signedIn}
                onSignIn={() => setSettingsPage("account")}
                onTrackers={(list) => {
                  setTrackers(list);
                  readInbox();
                }}
              />
            )}
            {settingsPage === "account" && (
              <AccountScreen onChanged={setSignedIn} onClose={() => setScreen(null)} />
            )}
            {settingsPage === "about" && (
              <AboutScreen
                server={server}
                agent={(selected ? polls[selected]?.usage : null) ?? null}
                onClose={() => setScreen(null)}
              />
            )}
          </SettingsModal>
        )}

        {/* Your tickets and pull requests, as T3 Code's Issues and Pull
            requests are: a page each in the middle, opened from the sidebar's
            foot, and one of them read in the panel on the right. */}
        {inboxOpen && (
        <InboxBar
          shut={prefs.inboxShut}
          view={screen === "pulls" ? "pulls" : "tickets"}
          inbox={inbox}
          reading={inboxReading}
          error={inboxError}
          trackers={trackers}
          signedIn={signedIn}
          sessions={live}
          projects={projects}
          currentProject={session?.project ?? null}
          showing={showing}
          showingPull={showingPull}
          keys={keysOf("tickets")}
          onFold={() => setScreen(null)}
          onShut={(inboxShut) => setPrefs({ inboxShut })}
          onView={(view) => {
            setPrefs({ inboxView: view });
            setScreen(view);
          }}
          onRefresh={readInbox}
          // Signed out, the way to trackers is signing in; signed in, it is
          // adding one.
          onSetup={() => {
            openSettings(signedIn === false ? "account" : "trackers");
          }}
          onSelect={(name) => {
            // Going to a worktree is going back to the workspace.
            setScreen(null);
            goToWorktree(name);
          }}
          onShow={(task) => {
            setShowingPull(null);
            setShowing((current) =>
              current?.tracker === task.tracker && current.key === task.key ? null : task,
            );
          }}
          onShowPull={showPull}
          onStart={startFrom}
          onStartPull={startFromPull}
        />
        )}

        {/* The ticket, read in full beside the sidebar and over whatever is
            under it, rather than in a column of its own: it is read and put
            away, and the workspace should not reflow for it. */}
        {showing && (
          <div
            className="inbox-flyout"
            style={{
              right: flyoutRight,
              width: prefs.ticketWidth,
              maxWidth: `calc(100% - ${flyoutRight + FLYOUT_GAP}px)`,
            }}
          >
            {/* On the flyout's left edge, so a drag left widens it. */}
            <Split
              label="Ticket width"
              value={prefs.ticketWidth}
              min={LIMITS.ticketWidth.min}
              max={flyoutMax(LIMITS.ticketWidth, flyoutRight)}
              reset={DEFAULTS.ticketWidth}
              grows="left"
              onChange={(ticketWidth) => setPrefs({ ticketWidth })}
            />
            <TicketPanel
              key={`${showing.tracker}:${showing.key}`}
              task={showing}
              projects={projects}
              currentProject={session?.project ?? null}
              onStart={startFrom}
              onClose={() => setShowing(null)}
              onChanged={readInbox}
            />
          </div>
        )}
        {showingPull && (
          <div
            className="inbox-flyout"
            style={{
              right: flyoutRight,
              width: prefs.pullWidth,
              maxWidth: `calc(100% - ${flyoutRight + FLYOUT_GAP}px)`,
            }}
          >
            <Split
              label="Pull request width"
              value={prefs.pullWidth}
              min={LIMITS.pullWidth.min}
              max={flyoutMax(LIMITS.pullWidth, flyoutRight)}
              reset={DEFAULTS.pullWidth}
              grows="left"
              onChange={(pullWidth) => setPrefs({ pullWidth })}
            />
            <PullPanel
              key={`${showingPull.tracker}:${showingPull.repo}:${showingPull.id}`}
              server={server}
              pull={showingPull}
              projects={pullProjects(showingPull, projects)}
              session={pullSession(showingPull, live)}
              onStart={startFromPull}
              onSelect={(name) => {
                setScreen(null);
                goToWorktree(name);
              }}
              onClose={() => setShowingPull(null)}
              onChanged={readInbox}
            />
          </div>
        )}
      </main>

      {palette && <CommandPalette items={paletteItems()} onClose={() => setPalette(false)} />}
      {creatingProject && server && (
        <NewProjectDialog
          server={server}
          onClose={() => setCreatingProject(false)}
          onCreated={(list) => {
            setProjects(list);
            setCreatingProject(false);
          }}
        />
      )}

      {confirmation}

    </div>
  );
}

/// What a session that is not ready yet is doing.
function preparingNote(session: Session | null) {
  if (session?.state === "seeding") return `Cloning into ${session.title ?? session.name} and starting the agent`;
  return `Creating the sandbox${session ? ` for ${session.title ?? session.name}` : ""}`;
}

