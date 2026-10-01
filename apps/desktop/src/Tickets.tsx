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
// the ticket, so publishing writes back to it.
//
// **A ticket does not know which repository it is about.** A Jira issue names a
// project and an Azure DevOps work item names an area path; neither is a clone
// URL. So a card carries a project chooser: the tracker says what to do and you
// say where.
//
// **A board, one column per filter**, side by side and each scrolling on its
// own, so a fourth filter costs width rather than a page of scrolling; a
// ticket two filters match is in both. The setup is the screen's other half,
// behind the `trackers` toggle in its header -- it used to sit above the
// tickets and push them off the screen.
// What is worth interrupting for -- a ticket changing -- is `ticketNotify.ts`,
// fed from here and from the window's own timer.

import { useCallback, useEffect, useRef, useState } from "react";

import { api, messageOf } from "./api";
import { Empty, Waiting } from "./Empty";
import type { ConfiguredTracker } from "./gen/ConfiguredTracker";
import type { Inbox as Tickets } from "./gen/Inbox";
import type { Project } from "./gen/Project";
import type { Task } from "./gen/Task";
import type { Tracker } from "./gen/Tracker";
import type { TrackerFilter } from "./gen/TrackerFilter";
import type { TrackerKind } from "./gen/TrackerKind";
import {
  Comments,
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
import { Screen } from "./Screen";
import { onTickets } from "./ticketNotify";
import { Select } from "./Select";

export function TicketsScreen({
  server,
  projects,
  currentProject,
  notify,
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
  // Which half of the screen is showing, once somebody has chosen; until
  // then it follows from whether there are any trackers. See `view` below.
  const [chosen, setView] = useState<"board" | "trackers" | null>(null);

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

  // The board unless there is nothing to put on it: with no trackers, the
  // only useful thing this screen can show is how to add one.
  const view = chosen ?? (trackers !== null && trackers.length === 0 ? "trackers" : "board");
  // The tracker's name on a column only when there is more than one tracker;
  // with one, it would say the same word on every column.
  const many = new Set(columns.map((c) => c.tracker)).size > 1;

  const actions = (
    <>
      {view === "board" && (
        <button
          className="quiet-icon"
          title="read the tickets again"
          disabled={tickets === null && !error}
          onClick={() => void readTickets()}
        >
          <Refresh aria-label="refresh" />
        </button>
      )}
      <button
        className={view === "trackers" ? "quiet on" : "quiet"}
        title={view === "trackers" ? "back to the tickets" : "set up trackers, tokens and filters"}
        onClick={() => setView(view === "trackers" ? "board" : "trackers")}
      >
        <Setup /> trackers
      </button>
    </>
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
      {columns.length > 0 && (
        <div className="board scrollbar-sleek">
          {columns.map(({ tracker, filter }) => {
            const cards = tickets!.tasks.filter(
              (t) => t.tracker === tracker && t.filter === filter,
            );
            return (
              <section key={`${tracker}:${filter}`} className="board-column">
                <header>
                  <span className="board-filter">{filter}</span>
                  {many && <span className="board-tracker">{tracker}</span>}
                  <span className="board-count">{cards.length}</span>
                </header>
                <div className="board-cards scrollbar-sleek">
                  {cards.length === 0 && <p className="hint">nothing matches</p>}
                  {cards.map((task) => (
                    <Card
                      key={`${task.tracker}:${task.filter}:${task.id}`}
                      task={task}
                      projects={projects}
                      currentProject={currentProject}
                      onStart={onStart}
                    />
                  ))}
                </div>
              </section>
            );
          })}
        </div>
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
}: {
  task: Task;
  projects: Project[];
  currentProject: string | null;
  onStart: (project: Project, task: Task) => void;
}) {
  const [where, setWhere] = useState(currentProject ?? projects[0]?.name ?? "");
  const project = projects.find((p) => p.name === where);
  const updated = ago(task.updated);
  const comments = task.comments ?? 0;

  return (
    <article className="ticket-card">
      <div className="ticket-top">
        {/* The key, linked: reading the ticket is still a browser's job. The
            glyph after it is the window's one mark for "this leaves the
            window". */}
        <a className="ticket-key" href={task.url} target="_blank" rel="noreferrer" title={task.url}>
          {task.key}
          <Elsewhere />
        </a>
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

/// How long ago a tracker's timestamp was, in the fewest words: `5m ago`,
/// `3h ago`, `2d ago`. Null for anything it cannot read, which is then left
/// off the card rather than shown wrong.
function ago(stamp: string | null): string | null {
  if (!stamp) return null;
  // Jira writes `+0000` where the format wants `+00:00`, and WebKit is strict
  // about it.
  const when = Date.parse(stamp.replace(/([+-]\d\d)(\d\d)$/, "$1:$2"));
  if (Number.isNaN(when)) return null;
  const secs = Math.max(0, (Date.now() - when) / 1000);
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  if (secs < 86400 * 60) return `${Math.floor(secs / 86400)}d ago`;
  return `${Math.floor(secs / (86400 * 30))}mo ago`;
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
