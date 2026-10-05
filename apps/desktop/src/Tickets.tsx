// Tickets: what your trackers' filters say, how each tracker is set up, and
// one button that turns a ticket into a session.
//
// This was two places -- an inbox showing the tickets and a section of the
// integrations screen setting up where they came from -- and the second was
// the one nobody found. A tracker with no token is why its list is empty, so
// the token goes where the empty list is.
//
// Read on the server with the credentials in its store, so this window shows
// rows and never holds a token. The row knows how to become a session, with
// the task, the branch and the name already right, and the session remembers
// the ticket it came from.
//
// **A ticket does not know which repository it is about.** A Jira issue names a
// project and an Azure DevOps work item names an area path; neither is a clone
// URL. So a card carries a project chooser: the tracker says what to do and you
// say where.
//
// **Two readings, one toggle.** *Filters* is one column per filter, side by
// side and each scrolling on its own, so a fourth filter costs width rather
// than a page of scrolling; a ticket two filters match is in both. *Board* is
// one Jira board as Jira draws it -- its columns, its sprint, its limits --
// picked from every board the trackers can see, one at a time. Both are the
// same cards. On a board a card moves by dragging it to a column, or from its
// menu, and either is a Jira transition -- see `moveTo`. The setup is the screen's third half, behind the `trackers`
// toggle in its header -- it used to sit above the tickets and push them off
// the screen.
// What is worth interrupting for -- a ticket changing -- is `ticketNotify.ts`,
// fed from here and from the window's own timer.

import { useCallback, useEffect, useRef, useState } from "react";

import { api, messageOf } from "./api";
import { Empty, Waiting } from "./Empty";
import type { Boards } from "./gen/Boards";
import type { BoardColumn } from "./gen/BoardColumn";
import type { BoardView } from "./gen/BoardView";
import type { Transition } from "./gen/Transition";
import type { ConfiguredTracker } from "./gen/ConfiguredTracker";
import type { Inbox as Tickets } from "./gen/Inbox";
import type { Project } from "./gen/Project";
import type { Task } from "./gen/Task";
import type { Tracker } from "./gen/Tracker";
import type { TrackerFilter } from "./gen/TrackerFilter";
import type { TrackerKind } from "./gen/TrackerKind";
import {
  Comments,
  Copy,
  Edit,
  Elsewhere,
  Forget,
  Plus,
  Refresh,
  Setup,
  Start,
  Store,
  Tracker as TrackerGlyph,
} from "./icons";
import { copy, useContextMenu, type MenuItem } from "./ContextMenu";
import { openExternal } from "./open";
import { Screen } from "./Screen";
import { ago, TicketPanel } from "./Ticket";
import type { Prefs } from "./prefs";
import { onTickets } from "./ticketNotify";
import { Select } from "./Select";

export function TicketsScreen({
  server,
  projects,
  currentProject,
  notify,
  prefs,
  onPrefs,
  onClose,
  onStart,
}: {
  server: string;
  projects: Project[];
  /// The project of whatever is selected in the tree, which is the most likely
  /// answer to "where" and so the one a row opens on.
  currentProject: string | null;
  /// Whether a change found by opening this screen is announced. Recorded
  /// either way, so the next poll does not announce it again.
  notify: boolean;
  /// Where this screen was left -- filters or a board, and which board -- so
  /// it opens there again.
  prefs: Prefs;
  onPrefs: (change: Partial<Prefs>) => void;
  onClose: () => void;
  onStart: (project: Project, task: Task) => void;
}) {
  const [trackers, setTrackers] = useState<ConfiguredTracker[] | null>(null);
  const [tickets, setTickets] = useState<Tickets | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  // Read through a ref: toggling the preference is not a reason to ask every
  // tracker again, which a dependency would make it.
  const notifyRef = useRef(notify);
  notifyRef.current = notify;
  // Whether the setup is showing, once somebody has chosen; until then it
  // follows from whether there are any trackers. See `view` below.
  const [setup, setSetup] = useState<boolean | null>(null);
  // The ticket open beside the board, by tracker and key: a card in two
  // columns is one ticket, and opening either lights both.
  const [reading, setReading] = useState<Task | null>(null);
  const isOpen = (t: Task) => reading?.tracker === t.tracker && reading.key === t.key;

  // The boards there are to pick from, read the first time the board view is
  // shown rather than on every visit to the filters.
  const [boards, setBoards] = useState<Boards | null>(null);
  const [boardsError, setBoardsError] = useState<string | null>(null);
  const [shown, setShown] = useState<BoardView | null>(null);
  const [boardError, setBoardError] = useState<string | null>(null);
  // Which read of a board is the latest: switching boards twice quickly is
  // two answers, and only the second is the board somebody is looking at.
  const boardRead = useRef(0);

  // The tickets are re-read after every change to a tracker, because every
  // change to a tracker -- a token stored, a filter edited -- is a change to
  // what they are.
  const readTickets = useCallback(() => {
    setTickets(null);
    return api
      .tickets(server)
      .then((v) => {
        onTickets(v, notifyRef.current);
        setTickets(v);
      })
      .catch((e) => setError(messageOf(e)));
  }, [server]);

  const readBoards = useCallback(() => {
    setBoardsError(null);
    return api
      .boards()
      .then(setBoards)
      .catch((e) => setBoardsError(messageOf(e)));
  }, []);

  useEffect(() => {
    let live = true;
    api
      .trackers()
      .then((v) => live && setTrackers(v))
      .catch((e) => live && setError(messageOf(e)));
    void readTickets();
    return () => {
      live = false;
    };
  }, [server, readTickets]);

  /// One change to the trackers, and the view it answers with.
  const act = async (
    what: string,
    run: () => Promise<ConfiguredTracker[]>,
  ): Promise<boolean> => {
    setBusy(what);
    setError(null);
    try {
      setTrackers(await run());
      void readTickets();
      // A tracker added or forgotten is boards gained or lost; asked again
      // the next time the board view is shown.
      setBoards(null);
      return true;
    } catch (e) {
      setError(messageOf(e));
      return false;
    } finally {
      setBusy(null);
    }
  };

  // Every filter that was read, in order, whether or not it matched anything
  // -- an empty "ready to start" is worth seeing as empty.
  const columns: { tracker: string; filter: string }[] = [...(tickets?.read ?? [])];
  for (const t of tickets?.tasks ?? []) {
    if (!columns.some((c) => c.tracker === t.tracker && c.filter === t.filter)) {
      columns.push({ tracker: t.tracker, filter: t.filter });
    }
  }

  // The tickets unless there is nothing to put on them: with no trackers,
  // the only useful thing this screen can show is how to add one.
  const showSetup = setup ?? (trackers !== null && trackers.length === 0);
  const mode = prefs.ticketsView;
  const view = showSetup ? "trackers" : mode;
  // The tracker's name on a column only when there is more than one tracker;
  // with one, it would say the same word on every column.
  const many = new Set(columns.map((c) => c.tracker)).size > 1;

  // The board on show: the one last picked while it is still on the list,
  // and the first on the list otherwise -- a board deleted in Jira, or a
  // tracker forgotten, is not a reason for an empty screen.
  const picked =
    boards?.boards.find((b) => b.tracker === prefs.board?.tracker && b.id === prefs.board?.id) ??
    boards?.boards[0] ??
    null;
  const manyBoardTrackers = new Set(boards?.boards.map((b) => b.tracker)).size > 1;

  useEffect(() => {
    if (view === "board" && boards === null) void readBoards();
  }, [view, boards, readBoards]);

  const readBoard = useCallback(
    (tracker: string, id: string, quietly = false) => {
      const read = ++boardRead.current;
      // Quietly after a change made from here: the board stays up while it is
      // asked again, rather than blinking to a spinner under the pointer.
      if (!quietly) setShown(null);
      setBoardError(null);
      api
        .board(server, tracker, id)
        .then((v) => read === boardRead.current && setShown(v))
        .catch((e) => read === boardRead.current && setBoardError(messageOf(e)));
    },
    [server],
  );

  useEffect(() => {
    if (view === "board" && picked) readBoard(picked.tracker, picked.id);
    // By identity rather than by the object: a re-read list with the same
    // board in it is not a reason to read the board again.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, picked?.tracker, picked?.id, readBoard]);

  const boardLoading = view === "board" && (boards === null || (picked !== null && shown === null)) && !boardsError && !boardError;

  // Moving a card: which one is being moved (its key), a move that needs a
  // choice of transition, and why the last one could not be made.
  const [moving, setMoving] = useState<string | null>(null);
  const [choosing, setChoosing] = useState<{
    task: Task;
    column: BoardColumn;
    ways: Transition[];
  } | null>(null);
  const [moveError, setMoveError] = useState<string | null>(null);

  /// What changed from here -- a move, an edit in the panel -- means the
  /// board or the filters are out of date. Asked again without blanking.
  const refreshQuietly = () => {
    if (view === "board" && picked) readBoard(picked.tracker, picked.id, true);
    else void api.tickets(server).then((v) => {
      onTickets(v, notifyRef.current);
      setTickets(v);
    }, () => {});
  };

  /// Take `task` along `way`, showing it in `column` straight away -- the
  /// read that follows says whether Jira agrees.
  const perform = async (task: Task, column: BoardColumn, way: Transition) => {
    await api.transition(task.tracker, task.key, way.id);
    setShown((v) =>
      v && {
        ...v,
        columns: v.columns.map((c) => ({
          ...c,
          tasks:
            c.name === column.name
              ? [{ ...task, status: way.to, filter: c.name }, ...c.tasks.filter((t) => t.key !== task.key)]
              : c.tasks.filter((t) => t.key !== task.key),
        })),
      },
    );
    refreshQuietly();
  };

  /// Move a card to a column: the transition out of its status that lands
  /// in one of the column's -- asked which when there are several, and told
  /// why not when there are none.
  const moveTo = async (task: Task, column: BoardColumn) => {
    setMoving(task.key);
    setMoveError(null);
    setChoosing(null);
    try {
      const ways = (await api.transitions(task.tracker, task.key)).filter((t) =>
        column.statuses.includes(t.to_id),
      );
      if (ways.length === 0) {
        setMoveError(
          `${task.key} cannot go from ${task.status} to ${column.name}: the workflow has no transition that lands there`,
        );
      } else if (ways.length > 1) {
        setChoosing({ task, column, ways });
      } else {
        await perform(task, column, ways[0]!);
      }
    } catch (e) {
      setMoveError(`${task.key}: ${messageOf(e)}`);
    } finally {
      setMoving(null);
    }
  };

  // Dragging. By pointer rather than HTML drag-and-drop, which the desktop
  // window hands to the operating system for dropping files on it.
  const [drag, setDrag] = useState<{
    task: Task;
    from: number;
    x: number;
    y: number;
    over: number | null;
  } | null>(null);
  const pressed = useRef<{ task: Task; from: number; x: number; y: number } | null>(null);
  // A drag ends in a click on the card it started on; that click is not a
  // request to open it.
  const dragged = useRef(false);

  useEffect(() => {
    const columnAt = (x: number, y: number) => {
      const el = document.elementFromPoint(x, y)?.closest("[data-column]");
      return el ? Number(el.getAttribute("data-column")) : null;
    };
    const move = (e: PointerEvent) => {
      const p = pressed.current;
      if (!p) return;
      // Five pixels before it is a drag: a click with a shaky hand is a click.
      if (!drag && Math.hypot(e.clientX - p.x, e.clientY - p.y) < 5) return;
      setDrag({ ...p, x: e.clientX, y: e.clientY, over: columnAt(e.clientX, e.clientY) });
    };
    const up = () => {
      const p = pressed.current;
      pressed.current = null;
      if (!drag || !p) return;
      dragged.current = true;
      setTimeout(() => (dragged.current = false), 0);
      const to = drag.over;
      setDrag(null);
      if (to !== null && to !== drag.from && shown?.columns[to]) void moveTo(p.task, shown.columns[to]!);
    };
    const cancel = (e: KeyboardEvent) => {
      if (e.key === "Escape" && drag) {
        e.preventDefault();
        pressed.current = null;
        setDrag(null);
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("keydown", cancel, true);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("keydown", cancel, true);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drag, shown]);

  const actions = (
    <>
      {view === "board" && picked && (
        <Select
          className="board-picker"
          aria-label="which board"
          value={`${picked.tracker}\n${picked.id}`}
          onChange={(v) => {
            const [tracker, id] = v.split("\n");
            onPrefs({ board: { tracker, id } });
          }}
          options={boards!.boards.map((b) => ({
            value: `${b.tracker}\n${b.id}`,
            label: b.name,
            hint: [b.project, manyBoardTrackers ? b.tracker : null].filter(Boolean).join(" · "),
          }))}
        />
      )}
      {/* Two readings of the same tickets, and a toggle between them rather
          than two destinations: the filters are the questions you wrote, a
          board is the process the team runs. */}
      <div className="segmented" role="tablist" aria-label="how the tickets are shown">
        {(["filters", "board"] as const).map((m) => (
          <button
            key={m}
            role="tab"
            aria-selected={view === m}
            className={view === m ? "on" : ""}
            title={m === "filters" ? "one column per filter" : "a Jira board, as its columns"}
            onClick={() => {
              setSetup(false);
              onPrefs({ ticketsView: m });
            }}
          >
            {m}
          </button>
        ))}
      </div>
      {view === "board" && shown && (
        <button
          className="quiet-icon"
          title="open this board in Jira"
          onClick={() => openExternal(shown.url)}
        >
          <Elsewhere aria-label="open in Jira" />
        </button>
      )}
      {view !== "trackers" && (
        <button
          className="quiet-icon"
          title={view === "board" ? "read the board again" : "read the tickets again"}
          disabled={view === "board" ? boardLoading : tickets === null && !error}
          onClick={() => {
            if (view === "filters") void readTickets();
            else if (boardsError || boards === null) void readBoards();
            else if (picked) readBoard(picked.tracker, picked.id);
          }}
        >
          <Refresh aria-label="refresh" />
        </button>
      )}
      <button
        className={view === "trackers" ? "quiet on" : "quiet"}
        title={view === "trackers" ? "back to the tickets" : "set up trackers, tokens and filters"}
        onClick={() => setSetup(view !== "trackers")}
      >
        <Setup /> trackers
      </button>
    </>
  );

  /// One column of cards, whichever reading it is a column of.
  const column = (
    key: string,
    title: string,
    cards: Task[],
    extra: {
      empty: string;
      sub?: string;
      max?: number | null;
      min?: number | null;
      /// On a board: where this column is, so a card can be dropped on it.
      at?: number;
    },
  ) => {
    const over = extra.max != null && cards.length > extra.max;
    const under = extra.min != null && cards.length < extra.min;
    const target = drag !== null && extra.at === drag.over && extra.at !== drag.from;
    const columns = view === "board" ? (shown?.columns ?? []) : [];
    return (
      <section
        key={key}
        className={`board-column${target ? " drop-target" : ""}`}
        data-column={extra.at}
      >
        <header>
          <span className="board-filter">{title}</span>
          {extra.sub && <span className="board-tracker">{extra.sub}</span>}
          <span
            className={`board-count${over || under ? " off-limit" : ""}`}
            title={
              extra.max != null || extra.min != null
                ? [
                    extra.min != null ? `at least ${extra.min}` : null,
                    extra.max != null ? `at most ${extra.max}` : null,
                  ]
                    .filter(Boolean)
                    .join(", ")
                : undefined
            }
          >
            {cards.length}
            {extra.max != null && ` / ${extra.max}`}
          </span>
        </header>
        <div className="board-cards scrollbar-sleek">
          {cards.length === 0 && <p className="hint">{extra.empty}</p>}
          {cards.map((task) => (
            <Card
              key={`${task.tracker}:${task.filter}:${task.id}`}
              task={task}
              projects={projects}
              currentProject={currentProject}
              onStart={onStart}
              on={isOpen(task)}
              onOpen={
                task.kind === "jira"
                  ? () => {
                      if (!dragged.current) setReading(task);
                    }
                  : undefined
              }
              dragging={drag?.task.key === task.key || moving === task.key}
              moves={
                extra.at === undefined
                  ? undefined
                  : columns
                      .filter((_, i) => i !== extra.at)
                      .map((c) => ({
                        label: `Move to ${c.name}`,
                        disabled: moving !== null,
                        run: () => void moveTo(task, c),
                      }))
              }
              onPointerDown={
                extra.at === undefined
                  ? undefined
                  : (e) => {
                      if (e.button !== 0 || moving !== null) return;
                      if ((e.target as HTMLElement).closest("button, a, input, .select-trigger"))
                        return;
                      pressed.current = { task, from: extra.at!, x: e.clientX, y: e.clientY };
                    }
              }
            />
          ))}
        </div>
      </section>
    );
  };

  /// The columns, and the ticket open beside them.
  const area = (body: React.ReactNode) => (
    <div className="board-area">
      <div className="board scrollbar-sleek">{body}</div>
      {reading && (
        <TicketPanel
          key={`${reading.tracker}:${reading.key}`}
          task={reading}
          projects={projects}
          currentProject={currentProject}
          onStart={onStart}
          onClose={() => setReading(null)}
          onChanged={refreshQuietly}
        />
      )}
      {drag && (
        <div className="drag-ghost" style={{ left: drag.x + 12, top: drag.y + 8 }} aria-hidden>
          <span className="ticket-key">{drag.task.key}</span> {drag.task.title}
        </div>
      )}
    </div>
  );

  if (view === "trackers") {
    return (
      <Screen icon={TrackerGlyph} title="tickets" actions={actions} onClose={onClose}>
        {error && <p className="error">{error}</p>}
        <section className="panel">
          <h3>trackers</h3>
          <p className="hint">
            Where tickets come from. Kept on this computer, token included, and read from here —
            the server never sees either.
          </p>
          {trackers === null && !error && <Waiting />}
          {trackers?.length === 0 && (
            <Empty icon={TrackerGlyph} note="no trackers yet — add one below" />
          )}
          {trackers?.map((t) => (
            <TrackerRow
              key={t.source.name}
              tracker={t}
              busy={busy}
              onToken={(value) =>
                act(`tracker:${t.source.name}`, () => api.setTrackerToken(t.source.name, value))
              }
              onForget={() =>
                void act(`tracker:${t.source.name}`, () => api.forgetTracker(t.source.name))
              }
              onFilters={(filters) =>
                act(`tracker:${t.source.name}`, () =>
                  api.updateTracker(t.source.name, { ...t.source, filters }),
                )
              }
            />
          ))}
          <NewTracker
            busy={busy !== null}
            onAdd={(tracker, token) =>
              act(`tracker:${tracker.name}`, () => api.addTracker(tracker, token))
            }
          />
        </section>
      </Screen>
    );
  }

  if (view === "board") {
    const noJira = trackers !== null && !trackers.some((t) => t.source.kind === "jira");
    // Why the columns hold what they do, or nothing: which sprint, and where
    // reading stopped.
    const notes = [
      ...(boards?.warnings ?? []).map((w) => ({ text: w, tone: "warn" })),
      ...(shown?.note ? [{ text: shown.note, tone: "hint" }] : []),
      ...(shown && shown.sprints.length > 0
        ? [{ text: shown.sprints.join(" · "), tone: "hint board-sprint" }]
        : []),
      ...(shown && shown.read < shown.total
        ? [
            {
              text: `the first ${shown.read} of ${shown.total} tickets on this board — the rest are in Jira`,
              tone: "hint",
            },
          ]
        : []),
    ];
    return (
      <Screen icon={TrackerGlyph} title="tickets" actions={actions} onClose={onClose} wide>
        {(boardsError || boardError || moveError || choosing || notes.length > 0) && (
          <div className="board-notes">
            {boardsError && <p className="error">{boardsError}</p>}
            {boardError && <p className="error">{boardError}</p>}
            {moveError && <p className="error">{moveError}</p>}
            {/* A column that two of the workflow's transitions land in: which
                one is a question about the process, so it is asked. */}
            {choosing && (
              <p className="board-choose">
                <span>
                  {choosing.task.key} can go to {choosing.column.name} more than one way:
                </span>
                {choosing.ways.map((w) => (
                  <button
                    key={w.id}
                    className="quiet"
                    onClick={() => {
                      const c = choosing;
                      setChoosing(null);
                      setMoving(c.task.key);
                      perform(c.task, c.column, w)
                        .catch((e) => setMoveError(`${c.task.key}: ${messageOf(e)}`))
                        .finally(() => setMoving(null));
                    }}
                  >
                    {w.name} → {w.to}
                  </button>
                ))}
                <button className="quiet" onClick={() => setChoosing(null)}>
                  cancel
                </button>
              </p>
            )}
            {notes.map((n) => (
              <p key={n.text} className={n.tone}>
                {n.text}
              </p>
            ))}
          </div>
        )}
        {noJira ? (
          <Empty
            size="page"
            icon={TrackerGlyph}
            note="boards come from Jira — add a Jira tracker under trackers"
          />
        ) : boards !== null && boards.boards.length === 0 && !boardsError ? (
          <Empty size="page" icon={TrackerGlyph} note="there are no boards this account can see" />
        ) : boardLoading ? (
          <Waiting />
        ) : (
          shown &&
          area(
            shown.columns.map((c, i) =>
              column(`${shown.id}:${i}`, c.name, c.tasks, {
                empty: "nothing here",
                max: c.max,
                min: c.min,
                at: i,
              }),
            ),
          )
        )}
      </Screen>
    );
  }

  return (
    <Screen icon={TrackerGlyph} title="tickets" actions={actions} onClose={onClose} wide>
      {(error || (tickets?.warnings.length ?? 0) > 0) && (
        <div className="board-notes">
          {error && <p className="error">{error}</p>}
          {/* A tracker that could not be read is said out loud rather than
              leaving its column quietly missing -- which is invisible, and
              looks like having nothing assigned. */}
          {tickets?.warnings.map((w) => (
            <p key={w} className="warn">
              {w}
            </p>
          ))}
        </div>
      )}
      {tickets === null && !error && <Waiting />}
      {tickets !== null && columns.length === 0 && (
        <Empty size="page" icon={TrackerGlyph} note="no filters answered — see trackers" />
      )}
      {columns.length > 0 &&
        area(
          columns.map(({ tracker, filter }) =>
            column(
              `${tracker}:${filter}`,
              filter,
              tickets!.tasks.filter((t) => t.tracker === tracker && t.filter === filter),
              { empty: "nothing matches", sub: many ? tracker : undefined },
            ),
          ),
        )}
    </Screen>
  );
}

/// One ticket, as a card: what it is, where it stands, what happened to it
/// last, and the one button that turns it into a session.
function Card({
  task,
  projects,
  currentProject,
  onStart,
  on,
  onOpen,
  moves,
  dragging = false,
  onPointerDown,
}: {
  task: Task;
  projects: Project[];
  currentProject: string | null;
  onStart: (project: Project, task: Task) => void;
  /// Whether this ticket is the one open beside the board.
  on: boolean;
  /// Read it in the window. Absent for a tracker that is only read in the
  /// browser so far, whose key links out instead.
  onOpen?: () => void;
  /// On a board: "Move to …" for every other column, which is how a card is
  /// moved without a mouse.
  moves?: MenuItem[];
  /// This card is the one being dragged.
  dragging?: boolean;
  onPointerDown?: (e: React.PointerEvent) => void;
}) {
  const menu = useContextMenu();
  const menuProps = menu(() => [
    ...(onOpen ? [{ label: "Open", run: onOpen }] : []),
    {
      label: "Open in the browser",
      icon: Elsewhere,
      run: () => openExternal(task.url),
    },
    ...(moves && moves.length > 0 ? (["separator", ...moves] as MenuItem[]) : []),
    "separator",
    { label: "Copy key", icon: Copy, hint: task.key, run: () => copy(task.key) },
    { label: "Copy link", icon: Copy, run: () => copy(task.url) },
  ]);
  const [where, setWhere] = useState(currentProject ?? projects[0]?.name ?? "");
  const project = projects.find((p) => p.name === where);
  const updated = ago(task.updated);
  const comments = task.comments ?? 0;

  return (
    <article
      className={`ticket-card${onOpen ? " opens" : ""}${on ? " on" : ""}${dragging ? " dragging" : ""}`}
      onPointerDown={onPointerDown}
      // The whole card opens it, except the controls on it.
      tabIndex={onOpen ? 0 : undefined}
      aria-current={on ? "true" : undefined}
      onClick={(e) => {
        if (!onOpen) return;
        if ((e.target as HTMLElement).closest("button, a, input, .select-trigger")) return;
        onOpen();
      }}
      onContextMenu={menuProps.onContextMenu}
      onKeyDown={(e) => {
        if (onOpen && e.key === "Enter" && e.target === e.currentTarget) onOpen();
        else menuProps.onKeyDown(e);
      }}
    >
      <div className="ticket-top">
        {/* A Jira ticket is read here, so its key is a name; anything else is
            still read in the browser, and its key is the link there. The
            glyph is the window's one mark for "this leaves the window". */}
        {onOpen ? (
          <span className="ticket-key">{task.key}</span>
        ) : (
          <a className="ticket-key" href={task.url} target="_blank" rel="noreferrer" title={task.url}>
            {task.key}
            <Elsewhere />
          </a>
        )}
        {task.item_type && <span className="ticket-type">{task.item_type}</span>}
        <span className="ticket-status" title="status">
          {task.status}
        </span>
      </div>
      <p className="ticket-title" title={task.title}>
        {task.title}
      </p>
      {(updated || comments > 0) && (
        <div className="ticket-meta">
          {updated && <span title={task.updated ?? undefined}>updated {updated}</span>}
          {comments > 0 && (
            <span
              className="ticket-comments"
              title={task.last_commenter ? `last comment by ${task.last_commenter}` : undefined}
            >
              <Comments /> {comments}
              {task.last_commenter && !task.last_comment_mine && (
                <span className="ticket-commenter"> · {task.last_commenter}</span>
              )}
            </span>
          )}
        </div>
      )}
      <div className="ticket-actions">
        {projects.length > 1 ? (
          <Select
            className="ticket-project"
            aria-label="project to start it in"
            value={where}
            onChange={setWhere}
            options={projects.map((p) => ({ value: p.name, label: p.name }))}
          />
        ) : (
          <span className="hint">{projects[0]?.name ?? "no project yet"}</span>
        )}
        <button
          className="quiet start"
          disabled={!project}
          title={project ? `start a worktree in ${project.name}` : "make a project first"}
          onClick={() => project && onStart(project, task)}
        >
          <Start /> start
        </button>
      </div>
    </article>
  );
}

/// What a tracker is pointed at, in one line.
///
/// Each kind is addressed differently -- a GitHub repository, an Azure DevOps
/// organisation and project, a Jira site -- and the row says which rather than
/// making three columns that are empty two times out of three.
function target(t: Tracker): string {
  switch (t.kind) {
    case "git-hub":
      return t.repo ?? "everything assigned to you";
    case "azure-dev-ops":
      return [t.org, t.project].filter(Boolean).join("/");
    case "jira":
      return [t.site, t.email].filter(Boolean).join(" as ");
  }
}

/// What a kind is called out here.
///
/// The wire value is serde's kebab-case of the Rust variant -- `git-hub`,
/// `azure-dev-ops` -- and those are not names anybody uses. The config file
/// spells them the way this does, so a row and the file agree.
const KINDS: { value: TrackerKind; label: string; name: string }[] = [
  { value: "jira", label: "jira", name: "Jira" },
  { value: "azure-dev-ops", label: "azure-devops", name: "Azure DevOps" },
  { value: "git-hub", label: "github", name: "GitHub" },
];

function kindLabel(kind: TrackerKind): string {
  return KINDS.find((k) => k.value === kind)?.label ?? kind;
}

/// One tracker, and whether it has a token.
///
/// A tracker with no token is the whole of why its sections come back empty
/// with a warning on them -- so it is said here, with the field to fix it.
function TrackerRow({
  tracker,
  busy,
  onToken,
  onForget,
  onFilters,
}: {
  tracker: ConfiguredTracker;
  busy: string | null;
  /// Store or replace the token. Kept on this computer and never shown again.
  onToken: (value: string) => Promise<boolean>;
  onForget: () => void;
  /// Answers whether the list was taken, so an edit that was refused
  /// keeps what was typed.
  onFilters: (filters: TrackerFilter[]) => Promise<boolean>;
}) {
  const t = tracker.source;
  const working = busy === `tracker:${t.name}`;
  const [token, setToken] = useState("");
  return (
    <div className="row tracker">
      <span className="row-name">{t.name}</span>
      <span className="tracker-kind">{kindLabel(t.kind)}</span>
      <span className="hint" title={target(t)}>
        {target(t)}
      </span>
      <span className={tracker.token_set ? "yes" : "no"}>
        {tracker.token_set ? "token stored" : "no token"}
      </span>
      <span className="row-actions">
        <input
          type="password"
          value={token}
          placeholder={tracker.token_set ? "replace the token" : "paste the token"}
          onChange={(e) => setToken(e.target.value)}
        />
        <button
          className="quiet-icon"
          disabled={working || token.trim().length === 0}
          title={tracker.token_set ? "replace the token" : "store the token"}
          onClick={() => {
            // Cleared either way: a password field holding a token is worth
            // nothing once it is stored, and less if it was refused.
            void onToken(token.trim()).finally(() => setToken(""));
          }}
        >
          <Store aria-label="store the token" />
        </button>
        <button
          className="quiet-icon danger"
          disabled={working}
          title="remove this tracker and its token from this computer"
          onClick={onForget}
        >
          <Forget aria-label={`forget ${t.name}`} />
        </button>
      </span>
      {!tracker.token_set && (
        <p className="problem">
          No token yet; paste one above and this tracker starts answering.
        </p>
      )}
      {/* GitHub's list is asked with parameters rather than a query, so there
          is nothing to filter with. */}
      {t.kind !== "git-hub" && (
        <Filters
          filters={tracker.filters}
          language={t.kind === "jira" ? "JQL" : "WIQL"}
          busy={working}
          onSave={onFilters}
        />
      )}
    </div>
  );
}

/// A tracker's named queries, each a section of the ticket list.
///
/// Starts from the filters the server says it runs -- the implied "assigned to
/// me" included -- so adding "ready to start" is adding a second section, not
/// quietly replacing the one that was there. Every change saves the whole list,
/// because the list is what the config file holds.
function Filters({
  filters,
  language,
  busy,
  onSave,
}: {
  filters: TrackerFilter[];
  language: string;
  busy: boolean;
  onSave: (filters: TrackerFilter[]) => Promise<boolean>;
}) {
  /// Which one is open for editing: an index, `"new"`, or none.
  const [editing, setEditing] = useState<number | "new" | null>(null);

  const save = (next: TrackerFilter[]) =>
    onSave(next).then((ok) => {
      if (ok) setEditing(null);
      return ok;
    });

  return (
    <div className="filters">
      {filters.map((f, i) =>
        editing === i ? (
          <FilterForm
            key={i}
            initial={f}
            language={language}
            busy={busy}
            onCancel={() => setEditing(null)}
            onSave={(edited) => save(filters.map((g, j) => (j === i ? edited : g)))}
          />
        ) : (
          <div key={i} className="filter">
            <span className="filter-name">{f.name}</span>
            <code className="filter-query" title={f.query}>
              {f.query}
            </code>
            <span className="row-actions">
              <button
                className="quiet-icon"
                disabled={busy}
                title="edit this filter"
                onClick={() => setEditing(i)}
              >
                <Edit aria-label={`edit ${f.name}`} />
              </button>
              <button
                className="quiet-icon danger"
                disabled={busy}
                title={
                  filters.length === 1
                    ? "remove it; the tracker goes back to what is assigned to you"
                    : "remove this filter"
                }
                onClick={() => void save(filters.filter((_, j) => j !== i))}
              >
                <Forget aria-label={`remove ${f.name}`} />
              </button>
            </span>
          </div>
        ),
      )}
      {editing === "new" ? (
        <FilterForm
          initial={{ name: "", query: "" }}
          language={language}
          busy={busy}
          onCancel={() => setEditing(null)}
          onSave={(added) => save([...filters, added])}
        />
      ) : (
        <button className="quiet add-filter" disabled={busy} onClick={() => setEditing("new")}>
          <Plus /> add filter
        </button>
      )}
    </div>
  );
}

function FilterForm({
  initial,
  language,
  busy,
  onCancel,
  onSave,
}: {
  initial: TrackerFilter;
  language: string;
  busy: boolean;
  onCancel: () => void;
  onSave: (filter: TrackerFilter) => Promise<boolean>;
}) {
  const [name, setName] = useState(initial.name);
  const [query, setQuery] = useState(initial.query);
  const ready = name.trim().length > 0 && query.trim().length > 0;
  return (
    <div className="filter editing">
      <input
        autoFocus
        className="filter-name"
        value={name}
        placeholder="ready to start"
        onChange={(e) => setName(e.target.value)}
      />
      <input
        className="filter-query"
        value={query}
        placeholder={language}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && ready) void onSave({ name: name.trim(), query: query.trim() });
          if (e.key === "Escape") onCancel();
        }}
      />
      <span className="row-actions">
        <button className="quiet" disabled={busy} onClick={onCancel}>
          cancel
        </button>
        <button
          className="go"
          disabled={busy || !ready}
          onClick={() => void onSave({ name: name.trim(), query: query.trim() })}
        >
          save
        </button>
      </span>
    </div>
  );
}

/// Add a tracker: where it points and the token, as labelled fields.
///
/// Only what each kind needs, the required ones marked. Custom queries are
/// not asked for here: a tracker starts with "assigned to me", and its row is
/// where a filter is added once it exists -- one way to do it, not two.
function NewTracker({
  busy,
  onAdd,
}: {
  busy: boolean;
  /// Answers whether it was taken. A rejected entry keeps what was typed --
  /// the reason is usually one field, and re-typing the rest would be the
  /// punishment for a typo.
  onAdd: (tracker: Tracker, token: string) => Promise<boolean>;
}) {
  const [kind, setKind] = useState<TrackerKind>("jira");
  const [name, setName] = useState("");
  const [token, setToken] = useState("");
  const [repo, setRepo] = useState("");
  const [org, setOrg] = useState("");
  const [project, setProject] = useState("");
  const [site, setSite] = useState("");
  const [email, setEmail] = useState("");

  const filled = (v: string) => v.trim().length > 0;
  const blank = (v: string) => (filled(v) ? v.trim() : null);
  // What each kind cannot work without -- the same rule the store applies, so
  // the button says so before the store has to.
  const ready =
    filled(token) &&
    (kind !== "jira" || (filled(site) && filled(email))) &&
    (kind !== "azure-dev-ops" || (filled(org) && filled(project)));

  const tracker = (): Tracker => ({
    kind,
    // Blank means "call it after its kind".
    name: name.trim(),
    repo: kind === "git-hub" ? blank(repo) : null,
    org: kind === "azure-dev-ops" ? blank(org) : null,
    project: kind === "azure-dev-ops" ? blank(project) : null,
    site: kind === "jira" ? blank(site) : null,
    email: kind === "jira" ? blank(email) : null,
    filters: [],
  });

  const reset = () => {
    setName("");
    setRepo("");
    setOrg("");
    setProject("");
    setSite("");
    setEmail("");
  };

  return (
    <div className="setting-group new-tracker">
      <h4>add a tracker</h4>
      <label>
        <span>tracker</span>
        <Select
          value={kind}
          onChange={(v) => setKind(v as TrackerKind)}
          options={KINDS.map((k) => ({ value: k.value, label: k.name }))}
        />
      </label>

      {kind === "jira" && (
        <>
          <Field label="site" required>
            <input
              value={site}
              placeholder="https://your-org.atlassian.net"
              onChange={(e) => setSite(e.target.value)}
            />
          </Field>
          <Field label="email" required>
            <input
              value={email}
              placeholder="you@example.com"
              onChange={(e) => setEmail(e.target.value)}
            />
          </Field>
          <p className="hint indent">The Atlassian account the token belongs to.</p>
        </>
      )}
      {kind === "azure-dev-ops" && (
        <>
          <Field label="organisation" required>
            <input value={org} placeholder="your-org" onChange={(e) => setOrg(e.target.value)} />
          </Field>
          <Field label="project" required>
            <input
              value={project}
              placeholder="YourProject"
              onChange={(e) => setProject(e.target.value)}
            />
          </Field>
        </>
      )}
      {kind === "git-hub" && (
        <>
          <Field label="repository">
            <input value={repo} placeholder="owner/name" onChange={(e) => setRepo(e.target.value)} />
          </Field>
          <p className="hint indent">Leave it empty for every issue assigned to you.</p>
        </>
      )}

      <Field label={TOKEN_LABEL[kind]} required>
        <input type="password" value={token} onChange={(e) => setToken(e.target.value)} />
      </Field>
      <p className="hint indent">{TOKEN_HINT[kind]}</p>

      <Field label="name">
        <input value={name} placeholder={kindLabel(kind)} onChange={(e) => setName(e.target.value)} />
      </Field>
      <p className="hint indent">What this tracker is called here. Only needed for a second one.</p>

      <div className="setting-actions">
        <span className="hint">* required</span>
        <button
          className="go"
          disabled={busy || !ready}
          onClick={() => {
            void onAdd(tracker(), token.trim()).then((added) => {
              // The token goes either way: a password field holding one is
              // worth nothing once it is stored, and less if it was refused.
              setToken("");
              if (added) reset();
            });
          }}
        >
          add tracker
        </button>
      </div>
    </div>
  );
}

/// A labelled field in the settings grid, with a mark when it is required.
function Field({
  label,
  required = false,
  children,
}: {
  label: string;
  required?: boolean;
  children: React.ReactNode;
}) {
  return (
    <label>
      <span>
        {label}
        {required && <span className="required"> *</span>}
      </span>
      {children}
    </label>
  );
}

/// What each kind calls its token, and where to get one.
const TOKEN_LABEL: Record<TrackerKind, string> = {
  jira: "API token",
  "azure-dev-ops": "access token",
  "git-hub": "token",
};

const TOKEN_HINT: Record<TrackerKind, string> = {
  jira: "Created at id.atlassian.com → Security → API tokens. Stored on this computer only.",
  "azure-dev-ops":
    "A personal access token with Work Items (read). Stored on this computer only.",
  "git-hub": "A token that can read issues. Stored on this computer only.",
};
