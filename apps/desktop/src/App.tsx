// The workspace: projects on the left, what is open in the middle, and what is
// true about the worktree on the right.
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
// tickets, the integrations, the servers and the settings used to be modal
// dialogs stacked over this; they are screens, and `Screen.tsx` says at length
// why. What that costs here is the two things a router has to do: hold which
// screen is showing, and keep the workspace *mounted* while another one is --
// see `hidden` on `.workspace` below, which is the load-bearing half.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api, messageOf, type Paired, type ServerSummary } from "./api";
import { useConfirm } from "./Confirm";
import { Dock } from "./Dock";
import { Empty } from "./Empty";
import {
  Branch,
  Forget,
  Integrations,
  NewProject,
  NoSandbox,
  NoServer,
  Servers,
  Settings,
  Tracker,
  Working,
} from "./icons";
import { TicketsScreen } from "./Tickets";
import { IntegrationsScreen } from "./Integrations";
import { ServersScreen } from "./Connect";
import type { Project } from "./gen/Project";
import type { Session } from "./gen/Session";
import type { Task } from "./gen/Task";
import type { Poll } from "./gen/Poll";
import { onSessions, reset as resetNotifications } from "./notify";
import { onTickets } from "./ticketNotify";
import { close, nextChannelId, open } from "./stream";
import { NewProjectDialog } from "./NewProject";
import { NewWorktreeDialog } from "./NewWorktree";
import { DEFAULTS, LIMITS, usePrefs } from "./prefs";
import { SettingsScreen } from "./Settings";
import { Split } from "./Split";
import type { Against } from "./gen/Against";
import { keyOf, Tabs, type Tab } from "./Tabs";
import { group, Tree } from "./Tree";
import { UpdateBadge } from "./Update";
import { AboutScreen } from "./About";
import { refreshPreviews } from "./previews";
import { Select } from "./Select";

/// Where in the window you are.
///
/// `null` is the workspace, rather than a `"workspace"` member, and the
/// asymmetry is deliberate: the workspace is not a screen you navigate to, it
/// is what the window *is* when nothing is over it. Writing it as one of five
/// equal names would invite code that tears it down to show another, which is
/// the one thing that must not happen -- the terminals live in it.
type Screen = "tickets" | "integrations" | "servers" | "settings" | "about";

/// How often the tickets are read for changes. See the effect that uses it.
const TICKET_POLL_MS = 3 * 60_000;

const MAC = navigator.userAgent.includes("Mac");

/// The tickets screen from anywhere. With Shift, because Ctrl+T on its own is
/// a terminal's: bash and readline transpose two characters with it, and the
/// shortcut is taken before a terminal sees the key.
const TICKETS_KEYS = MAC
  ? { label: "⌘⇧T", aria: "Meta+Shift+T" }
  : { label: "Ctrl+Shift+T", aria: "Control+Shift+T" };

function isTicketsKey(e: KeyboardEvent): boolean {
  return (
    e.shiftKey &&
    !e.altKey &&
    (MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey) &&
    e.key.toLowerCase() === "t"
  );
}

/// The tabs a worktree has open.
///
/// Derived from the sandbox rather than remembered here: what shells exist is a
/// fact about the sandbox, and one that outlives this window. A tab list kept
/// in the client would show a shell that had been closed from elsewhere and
/// hide one opened from elsewhere.
///
/// A chat session's agent is its first conversation rather than a terminal, and
/// the others it has are asked of the host the same way the shells are asked
/// of tmux. Until that answer arrives the agent's own is there anyway, because
/// it always exists.
function tabsFor(session: Session, shells: string[], chats: string[], open: Tab[]): Tab[] {
  const agent: Tab[] =
    session.interface === "chat"
      ? [
          { kind: "chat", conv: "agent", label: "agent" },
          ...chats
            .filter((conv) => conv !== "agent")
            .map((conv): Tab => ({ kind: "chat", conv, label: conv })),
        ]
      : [{ kind: "terminal", tmux: null, label: "agent" }];
  return [
    ...agent,
    ...shells.map((tmux): Tab => ({ kind: "terminal", tmux, label: tmux })),
    // Files and diffs last, in the order they were opened -- the one thing here
    // that is genuinely this window's state. A file is open because someone in
    // *this* window clicked it, and nothing in the sandbox knows that.
    ...open,
  ];
}

/// The tab a worktree falls back to when the one in front goes away: the
/// agent, whichever kind of agent it has.
function home(session: Session): string {
  return session.interface === "chat" ? "chat:agent" : "terminal:agent";
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


export default function App() {
  // How wide the sidebars are, how often the list is re-read, and whether the
  // OS is told when an agent waits. This window's own, kept on this machine:
  // see `prefs.ts` for why none of it is on the server beside the branch
  // prefix.
  const [prefs, setPrefs] = usePrefs();

  /// Asking before something irreversible, in a dialog of this window's own.
  /// `dialog` is rendered at the end, beside the others.
  const { ask, dialog: confirmation } = useConfirm();

  const [servers, setServers] = useState<ServerSummary[] | null>(null);
  const [server, setServer] = useState<string | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  /// Which destination is showing, if any. One value rather than the four
  /// booleans this used to be: they were four states that had to be false
  /// together, and nothing enforced it -- opening the tickets from the settings
  /// screen put both on screen at once, each with its own scrim.
  const [screen, setScreen] = useState<Screen | null>(null);
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
  const [creatingIn, setCreatingIn] = useState<Project | null>(null);
  /// The ticket a create was started from, carried from the tickets screen to the form.
  const [fromTask, setFromTask] = useState<Task | null>(null);
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
  const [shells, setShells] = useState<Record<string, string[]>>({});
  /// A chat session's conversations beside the agent's, the same way.
  const [chats, setChats] = useState<Record<string, string[]>>({});
  const [files, setFiles] = useState<Record<string, Tab[]>>({});
  const [active, setActive] = useState<Record<string, string>>({});
  /// Tab keys per worktree, in the order they were dragged into. Window state
  /// like `files` and not persisted: see `arrange`.
  const [order, setOrder] = useState<Record<string, string[]>>({});

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
    if (screen === null) return;
    const onKey = (e: KeyboardEvent) =>
      e.key === "Escape" && !e.defaultPrevented && setScreen(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [screen]);

  // The tickets shortcut. In the capture phase, so it is this window's before
  // a focused terminal takes the key and sends it to the sandbox. Not while a
  // dialog is up: that is asking a question, and the screen would change
  // under it.
  const dialogOpen = creatingProject || creatingIn !== null || confirmation !== null;
  useEffect(() => {
    if (!server || dialogOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (!isTicketsKey(e)) return;
      e.preventDefault();
      e.stopPropagation();
      setScreen((current) => (current === "tickets" ? null : "tickets"));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [server, dialogOpen]);

  // Keyed on the *names*, joined, rather than on the array: the list is a new
  // array every few seconds and re-subscribing to four sandboxes that often
  // would be worse than not subscribing at all.
  //
  // Not a dead session's: there is no sandbox to poll, and the server would
  // ask the gateway about it every two seconds for as long as the row is
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

  /// A session that has been asked for but is not in the list yet.
  ///
  /// A ref rather than state: `refresh` reads it, and making it a dependency
  /// would rebuild the poll timer every time a session is created.
  const pending = useRef<string | null>(null);

  const refresh = useCallback(async () => {
    if (!server) return null;
    try {
      const [list, known] = await Promise.all([api.sessions(server), api.projects(server)]);
      setSessions(list);
      setProjects(known);
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
  /// `create` answers as soon as the request is accepted -- the sandbox is
  /// seconds of gateway after that, on the server's own thread -- so the record
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
    const timer = setInterval(() => void refresh(), prefs.refreshMs);
    return () => clearInterval(timer);
  }, [refresh, prefs.refreshMs]);

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

  // The tickets, on a timer, for what has changed in them. Minutes rather than
  // the session list's seconds: every read is one search per filter against
  // somebody else's API, and a ticket that moved two minutes ago is news
  // enough. The first read on start is the baseline, or catches up with what
  // moved while the window was closed.
  //
  // Not gated on a server: the trackers are this machine's. A server only
  // lends its branch prefix to the suggested branch names.
  useEffect(() => {
    if (!prefs.notifyTickets) return;
    const read = () =>
      api
        .tickets(server)
        .then((tickets) => onTickets(tickets))
        // A tracker that cannot be read is the tickets screen's to say; a
        // timer has nowhere to put it.
        .catch(() => {});
    void read();
    const timer = setInterval(read, TICKET_POLL_MS);
    return () => clearInterval(timer);
  }, [server, prefs.notifyTickets]);

  const groups = useMemo(() => group(projects, live), [projects, live]);

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
  // made. Every pane would ask the gateway about it and draw `sandbox not
  // found`, which says what is missing and not why, so they are not mounted
  // and the reason is shown instead.
  const gone = session?.state === "dead";

  // Asked once per worktree as it is selected. Not polled: a shell appears
  // because someone in this window asked for one, and paying an exec a second
  // to hear that nothing changed is what the stream exists to avoid.
  useEffect(() => {
    if (!server || !session || preparing || gone || shells[session.name]) return;
    let live = true;
    api
      .shells(server, session.name)
      .then((list) => live && setShells((all) => ({ ...all, [session.name]: list })))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [server, session, preparing, gone, shells]);

  // The conversations, asked once per worktree like the shells, and only of a
  // chat session: a terminal session has no host to ask.
  const chatSession = session?.interface === "chat";
  useEffect(() => {
    if (!server || !session || !chatSession || preparing || gone || chats[session.name]) return;
    let live = true;
    api
      .chats(server, session.name)
      .then((list) => live && setChats((all) => ({ ...all, [session.name]: list.chats })))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [server, session, chatSession, preparing, gone, chats]);

  const openTabs = session
    ? arrange(
        tabsFor(session, shells[session.name] ?? [], chats[session.name] ?? [], files[session.name] ?? []),
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
      title: dead ? `Remove ${s.name}?` : `Destroy ${s.name}?`,
      body: dead ? (
        <>Its sandbox is already gone, so this only takes it off the list.</>
      ) : (
        <>Its sandbox goes with it, and anything the agent has not pushed is lost.</>
      ),
      confirm: dead ? "remove" : "destroy",
      onConfirm: () => {
        api
          .destroy(server, s.name)
          .then((left) => {
            setSessions(left);
            // Whatever was showing is gone. Left selected, the panes would go
            // on asking the server about a session it no longer has.
            if (selected === s.name) setSelected(null);
          })
          .catch((e) => setError(messageOf(e)));
      },
    });
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
  const activeTab = session ? (active[session.name] ?? keyOf(openTabs[0])) : "";

  // Nothing paired: the one screen a new install is guaranteed to see, and the
  // only empty state in the window that has to be instructions. The glyph
  // carries "there is no server", which is the part that used to be a heading;
  // what is left in words is the two commands, because no picture of a terminal
  // is going to tell anybody to type `hurad pair`.
  //
  // The servers screen is reachable from here and from nowhere else at this
  // point -- there is no header yet, because a header of destinations that all
  // need a server would be five disabled buttons.
  if (servers !== null && servers.length === 0) {
    return screen === "servers" ? (
      <ServersScreen
        servers={[]}
        onClose={() => setScreen(null)}
        onPaired={paired}
        onForgot={forgot}
      />
    ) : (
      <div className="app">
        <Empty size="page" icon={NoServer} note="no server paired">
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
          <button className="go" onClick={() => setScreen("servers")}>
            <Servers />
            paste a pairing string
          </button>
        </Empty>
      </div>
    );
  }

  return (
    <div className="app">
      <header>
        {/* The wordmark is the way to the about screen, and carries the
            version so "which build is this" is never more than a glance. */}
        <button
          className={`mark${screen === "about" ? " on" : ""}`}
          title="about hura"
          onClick={() => setScreen(screen === "about" ? null : "about")}
        >
          hura
          {version && <span className="mark-version">{version}</span>}
        </button>
        <UpdateBadge onOpen={() => setScreen("about")} />
        {/* The chooser only when there is a choice, and nothing at all when
            there is not: with one paired server there is nothing to
            disambiguate, and the servers screen names it.

            An address and a port used to sit here, and it was the wrong fact
            in the wrong place: `127.0.0.1:17671` is what you check once when
            pairing goes wrong, not something to read every time you look at
            the top of the window -- and it is already on the row for each
            server in that screen, next to the button that forgets it.

            The tally that stood beside it is gone for the same reason. "4
            worktrees in 2 projects" is the tree's own content restated as
            arithmetic: the tree is two feet to the left with the worktrees
            actually in it, and the count was reliably wrong anyway -- every
            session started from a terminal has no project, so a full list of
            them read "5 worktrees in 0 projects". */}
        {servers && servers.length > 1 && (
          <Select
            className="server-pick"
            aria-label="server"
            value={server ?? ""}
            onChange={setServer}
            options={servers.map((s) => ({ value: s.name, label: s.name, hint: s.address }))}
          />
        )}
        {/* The rate-limit windows are the *account's*, not this session's --
            two sessions on one account report the same numbers -- so they sit
            in the header rather than beside a worktree. The reading is
            whichever session last reported one; there is no source for it other
            than an agent's own status line. */}
        {(selected ? polls[selected]?.usage : undefined)?.windows.map((w) => (
          <span
            key={w.label}
            className={`limit${w.used_percentage >= 80 ? " high" : ""}`}
            title={
              w.resets_at ? `resets ${new Date(w.resets_at * 1000).toLocaleString()}` : undefined
            }
          >
            {w.label} {Math.round(w.used_percentage)}%
          </span>
        ))}
        {error && <span className="error">{error}</span>}

        {/* The window's five buttons, and the words are gone from all of them.
            The old row read `inbox`, `new project`, `integrations`, `servers`,
            `settings` -- about 210 pixels of label for five things that are
            pressed once each per sitting, in a 40-pixel strip that is also
            where a rate-limit reading and an error message have to fit.

            The note that used to sit here said an icon-only toolbar is a quiz,
            and it was right about the failure mode and wrong about the
            remedy. Three things answer it, and all three had to be built
            rather than asserted: every button carries a `title`, so the label
            is one hover away for as long as it takes to learn five glyphs; the
            screen each one opens is headed by *the same glyph* beside its name,
            so pressing one teaches what it was; and the one that is showing is
            lit, so the strip says where you are rather than only what it can
            do. A tooltip alone would have been the quiz with an answer key.

            The divider is not decoration: `new project` is an action and the
            four after it are destinations. A row that mixed them silently
            would be a row where one button does not come back. */}
        <nav className="destinations" aria-label="destinations">
          <button
            className="dest"
            disabled={!server}
            title="new project"
            aria-label="new project"
            onClick={() => setCreatingProject(true)}
          >
            <NewProject />
          </button>
          <span className="dest-split" />
          <Destination
            icon={Tracker}
            label="tickets"
            keys={TICKETS_KEYS}
            on={screen === "tickets"}
            disabled={!server}
            onOpen={() => setScreen("tickets")}
            onClose={() => setScreen(null)}
          />
          <Destination
            icon={Integrations}
            label="integrations"
            on={screen === "integrations"}
            disabled={!server}
            onOpen={() => setScreen("integrations")}
            onClose={() => setScreen(null)}
          />
          <Destination
            icon={Servers}
            label="servers"
            on={screen === "servers"}
            onOpen={() => setScreen("servers")}
            onClose={() => setScreen(null)}
          />
          <Destination
            icon={Settings}
            label="settings"
            on={screen === "settings"}
            disabled={!server}
            onOpen={() => setScreen("settings")}
            onClose={() => setScreen(null)}
          />
        </nav>
      </header>

      <main>
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
        <div className="workspace" hidden={screen !== null}>
          {/* The widths are inline styles rather than CSS variables, and that
              is the point of doing it this way: they are *values*, one per
              sidebar, and the handle beside each writes the same number the
              settings screen does. A `--tree-width` on the root would be a
              third place for the same fact to live. */}
          <Tree
            width={prefs.treeWidth}
            groups={groups}
            polls={polls}
            selected={selected}
            onSelect={setSelected}
            onNewWorktree={setCreatingIn}
            onForget={(p) => {
              if (!server) return;
              api
                .forgetProject(server, p.name)
                .then(setProjects)
                .catch((e) => setError(messageOf(e)));
            }}
            onDestroy={destroy}
            tickets={{
              disabled: !server,
              keys: TICKETS_KEYS,
              onOpen: () => setScreen("tickets"),
            }}
          />

          <Split
            label="projects sidebar width"
            value={prefs.treeWidth}
            min={LIMITS.treeWidth.min}
            max={LIMITS.treeWidth.max}
            reset={DEFAULTS.treeWidth}
            grows="right"
            onChange={(treeWidth) => setPrefs({ treeWidth })}
          />

          {preparing ? (
            // The create's own progress, said once in the middle: the steps
            // are seconds of gateway and then however long the clone takes,
            // and a spinner with the step beside it is all there is to know.
            <Empty size="page" icon={Working} note={preparingNote(session)} />
          ) : gone && session ? (
            // Why there is nothing here, which is what used to be missing: the
            // panes each said `sandbox not found`, and the reason was on the
            // server's terminal. The mark is grey because it only says what is
            // absent; the reason is red because it is an error.
            <Empty size="page" icon={NoSandbox} note="no sandbox behind this worktree">
              {session.failure && <p className="error">{session.failure}</p>}
              <button className="go" onClick={() => destroy(session)}>
                <Forget />
                remove
              </button>
            </Empty>
          ) : session && server ? (
            <>
              <Tabs
                server={server}
                name={session.name}
                tabs={openTabs}
                active={activeTab}
                failure={session.state === "failed" ? session.failure : null}
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
                      const opened = list[list.length - 1];
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
                onCloseChat={(conv) => {
                  ask({
                    title: `End ${conv}?`,
                    body: <>The conversation stops, and its transcript is forgotten.</>,
                    confirm: "end it",
                    onConfirm: () => {
                      api
                        .closeChat(server, session.name, conv)
                        .then(({ chats: list }) => {
                          setChats((all) => ({ ...all, [session.name]: list }));
                          setActive((a) =>
                            a[session.name] === `chat:${conv}` ? { ...a, [session.name]: home(session) } : a,
                          );
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
                  setActive((a) =>
                    a[session.name] === key ? { ...a, [session.name]: home(session) } : a,
                  );
                }}
                onCloseShell={(tmux) => {
                  api
                    .killShell(server, session.name, tmux)
                    .then((list) => {
                      setShells((all) => ({ ...all, [session.name]: list }));
                      // Whatever was in front may have just been killed; the
                      // agent is the one tab that is always there.
                      setActive((a) =>
                        a[session.name] === `terminal:${tmux}`
                          ? { ...a, [session.name]: home(session) }
                          : a,
                      );
                    })
                    .catch((e) => setError(messageOf(e)));
                }}
              />
              {/* Between the editor and the dock, so a drag on it takes width
                  from the middle and gives it to the right -- which is why
                  this one `grows="left"`. */}
              <Split
                label="dock width"
                value={prefs.dockWidth}
                min={LIMITS.dockWidth.min}
                max={LIMITS.dockWidth.max}
                reset={DEFAULTS.dockWidth}
                grows="left"
                onChange={(dockWidth) => setPrefs({ dockWidth })}
              />
              <Dock
                width={prefs.dockWidth}
                server={server}
                session={session}
                usage={(selected ? polls[selected]?.usage : undefined) ?? null}
                listening={(selected ? polls[selected]?.ports : undefined) ?? []}
                refreshMs={prefs.refreshMs}
                onOpenFile={(path) => openTab(session.name, { kind: "file", path })}
                onOpenDiff={(path, against: Against) =>
                  openTab(session.name, { kind: "filediff", path, against })
                }
              />
            </>
          ) : projects.length === 0 ? (
            // A glyph that is the button that fixes it, which is as close as an
            // empty state gets to explaining itself: the mark here and the mark
            // on `new project` in the header are the same picture.
            <Empty size="page" icon={NewProject} note="no projects yet">
              <button className="go" disabled={!server} onClick={() => setCreatingProject(true)}>
                <NewProject />
                new project
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
                  ? "start a worktree with + beside a project"
                  : "pick a worktree, or start one with + beside a project"
              }
            />
          )}
        </div>

        {screen === "tickets" && server && (
          <TicketsScreen
            server={server}
            notify={prefs.notifyTickets}
            prefs={prefs}
            onPrefs={setPrefs}
            projects={projects}
            currentProject={sessions.find((s) => s.name === selected)?.project ?? null}
            onClose={() => setScreen(null)}
            onStart={(project, task) => {
              // Straight into the create form, pre-filled: the list's whole
              // point is that starting work on a ticket is one step. The screen
              // closes under the dialog rather than behind it -- the form
              // answers back to the workspace, which is where the session will
              // appear.
              setScreen(null);
              setFromTask(task);
              setCreatingIn(project);
            }}
          />
        )}

        {screen === "integrations" && server && (
          <IntegrationsScreen server={server} onClose={() => setScreen(null)} />
        )}

        {screen === "servers" && (
          <ServersScreen
            servers={servers ?? []}
            onClose={() => setScreen(null)}
            onPaired={paired}
            onForgot={forgot}
          />
        )}

        {screen === "about" && (
          <AboutScreen
            server={server}
            agent={(selected ? polls[selected]?.usage : null) ?? null}
            onClose={() => setScreen(null)}
          />
        )}

        {screen === "settings" && server && (
          <SettingsScreen
            server={server}
            prefs={prefs}
            onPrefs={setPrefs}
            onClose={() => setScreen(null)}
          />
        )}
      </main>

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

      {creatingIn && server && (
        <NewWorktreeDialog
          server={server}
          project={creatingIn}
          from={fromTask}
          onClose={() => {
            setCreatingIn(null);
            setFromTask(null);
          }}
          onCreated={(name) => {
            setCreatingIn(null);
            setFromTask(null);
            // Selected before it exists, on purpose: the record is written as
            // soon as the server's thread reaches it, and the burst below is
            // what puts it in the sidebar rather than the next poll.
            setSelected(name);
            void refreshUntil(name);
          }}
        />
      )}
    </div>
  );
}

/// What a session that is not ready yet is doing.
function preparingNote(session: Session | null) {
  if (session?.state === "seeding") return `cloning into ${session.name} and starting the agent`;
  return `creating the sandbox${session ? ` for ${session.name}` : ""}`;
}

/// One of the header's destinations.
///
/// A toggle rather than a link, and that is the behaviour worth naming: pressing
/// the lit one goes back to the workspace. With no labels on these buttons, the
/// press that teaches you what a glyph meant is also the press you want to undo
/// immediately, and making the same button do it means never having to find the
/// way back.
///
/// `aria-current` and not `aria-pressed`: this says which of five places is
/// showing, not that a control is held down.
function Destination({
  icon: Mark,
  label,
  keys,
  on,
  disabled,
  onOpen,
  onClose,
}: {
  icon: React.ComponentType<{ className?: string }>;
  /// The word that is no longer on screen. It is still the tooltip and still
  /// the accessible name -- a button whose only content is an SVG has no name
  /// at all to a screen reader, which is the one way an icon-only strip can be
  /// genuinely worse rather than merely terser.
  label: string;
  /// The shortcut, when it has one, for the tooltip.
  keys?: { label: string; aria: string };
  on: boolean;
  disabled?: boolean;
  onOpen: () => void;
  onClose: () => void;
}) {
  return (
    <button
      className={`dest${on ? " on" : ""}`}
      disabled={disabled}
      title={keys ? `${label} (${keys.label})` : label}
      aria-label={label}
      aria-keyshortcuts={keys?.aria}
      aria-current={on ? "page" : undefined}
      onClick={on ? onClose : onOpen}
    >
      <Mark />
    </button>
  );
}
