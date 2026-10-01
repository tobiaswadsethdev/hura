// One ticket, read in full, beside the board it was picked from.
//
// The card says what a ticket is called and where it stands; this says what it
// is. Description, conversation, who asked for it and who has it -- the reasons
// to open a ticket at all, which used to mean leaving the window for a browser.
// The browser is still one click away in the header, for the things only the
// tracker can do: edit, transition, attach.
//
// Beside the board rather than over it, because the board is where you were:
// picking the next card should not mean closing this one first.

import { useCallback, useEffect, useRef, useState } from "react";

import { api, messageOf } from "./api";
import { Doc } from "./Doc";
import { Waiting } from "./Empty";
import type { Issue } from "./gen/Issue";
import type { IssuePerson } from "./gen/IssuePerson";
import type { Project } from "./gen/Project";
import type { Task } from "./gen/Task";
import { Close, Elsewhere, Refresh, Start } from "./icons";
import { Select } from "./Select";

export function TicketPanel({
  task,
  projects,
  currentProject,
  onStart,
  onClose,
}: {
  task: Task;
  projects: Project[];
  currentProject: string | null;
  onStart: (project: Project, task: Task) => void;
  onClose: () => void;
}) {
  const [issue, setIssue] = useState<Issue | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [where, setWhere] = useState(currentProject ?? projects[0]?.name ?? "");
  const project = projects.find((p) => p.name === where);
  const panel = useRef<HTMLElement>(null);

  // Focused on opening, so the keyboard is where the eyes are: Escape closes
  // it and the arrows scroll it, rather than either going to the card behind.
  useEffect(() => panel.current?.focus({ preventScroll: true }), []);

  const read = useCallback(() => {
    let live = true;
    setError(null);
    api.ticket(task.tracker, task.key).then(
      (v) => live && setIssue(v),
      (e) => live && setError(messageOf(e)),
    );
    return () => {
      live = false;
    };
  }, [task.tracker, task.key]);

  useEffect(() => {
    setIssue(null);
    return read();
  }, [read]);

  return (
    <aside
      ref={panel}
      // Focusable, so a click anywhere in it -- on the text being read --
      // keeps Escape here rather than handing it to the screen.
      tabIndex={-1}
      className="ticket-panel scrollbar-sleek"
      aria-label={`${task.key} ${task.title}`}
      // Escape closes this rather than the whole screen, while focus is in it.
      onKeyDown={(e) => {
        if (e.key === "Escape" && !e.defaultPrevented) {
          e.preventDefault();
          onClose();
        }
      }}
    >
      <header className="ticket-panel-head">
        <span className="ticket-panel-key">{task.key}</span>
        {(issue?.item_type || task.item_type) && (
          <span className="ticket-type">{issue?.item_type || task.item_type}</span>
        )}
        <span className="ticket-panel-tools">
          <button className="quiet-icon" title="read it again" onClick={() => void read()}>
            <Refresh aria-label="refresh" />
          </button>
          {/* The window's one mark for "this leaves the window". */}
          <a
            className="quiet-icon"
            href={task.url}
            target="_blank"
            rel="noreferrer"
            title={`open in ${task.kind === "jira" ? "Jira" : "the browser"}`}
          >
            <Elsewhere aria-label="open in the browser" />
          </a>
          <button className="quiet-icon" title="close" onClick={onClose}>
            <Close aria-label="close" />
          </button>
        </span>
      </header>

      <h2 className="ticket-panel-title">{issue?.title ?? task.title}</h2>

      {error && <p className="error">{error}</p>}
      {!issue && !error && <Waiting />}

      {issue && (
        <>
          <dl className="ticket-fields">
            <dt>status</dt>
            <dd>
              <span className={`stage ${issue.stage}`}>{issue.status}</span>
            </dd>
            <dt>assignee</dt>
            <dd>{issue.assignee ? <Who person={issue.assignee} /> : <Nobody>unassigned</Nobody>}</dd>
            <dt>reporter</dt>
            <dd>{issue.reporter ? <Who person={issue.reporter} /> : <Nobody>nobody</Nobody>}</dd>
            {issue.priority && (
              <>
                <dt>priority</dt>
                <dd>{issue.priority}</dd>
              </>
            )}
            {issue.parent && (
              <>
                <dt>parent</dt>
                <dd>
                  <span className="ticket-panel-key">{issue.parent.key}</span> {issue.parent.title}
                </dd>
              </>
            )}
            {issue.labels.length > 0 && (
              <>
                <dt>labels</dt>
                <dd className="ticket-labels">
                  {issue.labels.map((l) => (
                    <span key={l} className="tag">
                      {l}
                    </span>
                  ))}
                </dd>
              </>
            )}
            {issue.due && (
              <>
                <dt>due</dt>
                <dd>{issue.due}</dd>
              </>
            )}
            <dt>created</dt>
            <dd title={issue.created ?? undefined}>{ago(issue.created) ?? "—"}</dd>
            <dt>updated</dt>
            <dd title={issue.updated ?? undefined}>{ago(issue.updated) ?? "—"}</dd>
          </dl>

          <section className="ticket-section">
            <h3>description</h3>
            {issue.description.length > 0 ? (
              <Doc blocks={issue.description} />
            ) : (
              <Nobody>no description</Nobody>
            )}
          </section>

          <section className="ticket-section">
            <h3>
              comments <span className="count">{issue.comments_total}</span>
            </h3>
            {issue.comments_total > issue.comments.length && (
              <p className="hint">
                {issue.comments_total - issue.comments.length} older ones are only in the tracker.
              </p>
            )}
            {issue.comments.length === 0 && <Nobody>no comments yet</Nobody>}
            <ol className="ticket-comments-list">
              {issue.comments.map((c, i) => (
                <li key={i} className={c.author?.me ? "mine" : undefined}>
                  <div className="comment-head">
                    {c.author ? <Who person={c.author} /> : <Nobody>someone</Nobody>}
                    <span className="comment-when" title={c.created ?? undefined}>
                      {ago(c.created)}
                      {c.edited && <span title={`edited ${c.edited}`}> · edited</span>}
                    </span>
                  </div>
                  <Doc blocks={c.body} />
                </li>
              ))}
            </ol>
          </section>
        </>
      )}

      {/* Where it ends up either way: a ticket read is usually a ticket about
          to be started. */}
      <footer className="ticket-panel-foot">
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
          className="go"
          disabled={!project}
          title={project ? `start a worktree in ${project.name}` : "make a project first"}
          onClick={() => project && onStart(project, task)}
        >
          <Start /> start
        </button>
      </footer>
    </aside>
  );
}

/// A person, as initials and a name. Initials rather than the tracker's
/// avatar: the window loads nothing from the network, and an avatar is a
/// request per face.
function Who({ person }: { person: IssuePerson }) {
  const initials = person.name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join("");
  return (
    <span className="who">
      <span className="avatar" aria-hidden>
        {initials || "?"}
      </span>
      <span className="who-name">{person.name}</span>
      {person.me && <span className="who-me">you</span>}
    </span>
  );
}

function Nobody({ children }: { children: React.ReactNode }) {
  return <span className="nobody">{children}</span>;
}

/// How long ago a tracker's timestamp was, in the fewest words: `5m ago`,
/// `3h ago`, `2d ago`. Null for anything it cannot read, which is then left
/// off rather than shown wrong.
export function ago(stamp: string | null): string | null {
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
