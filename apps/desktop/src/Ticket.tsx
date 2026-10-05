// One ticket, read in full, beside the board it was picked from.
//
// The card says what a ticket is called and where it stands; this says what it
// is. Description, conversation, who asked for it and who has it -- the reasons
// to open a ticket at all, which used to mean leaving the window for a browser.
// A Jira ticket is also changed here: its status from the status line, which
// lists the workflow's ways out of it, its fields behind the edit button, and
// its comments where they are read -- a new one at the bottom, your own ones
// edited or deleted in place. The browser is still one click away in the
// header, for what only the tracker does: attach, link, log work.
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
import type { Transition } from "./gen/Transition";
import { useConfirm } from "./Confirm";
import { Close, Edit, Elsewhere, Forget, Refresh, Start } from "./icons";
import { Select } from "./Select";
import { MarkdownBox, TicketEditor } from "./TicketEdit";

export function TicketPanel({
  task,
  projects,
  currentProject,
  onStart,
  onClose,
  onChanged,
}: {
  task: Task;
  projects: Project[];
  currentProject: string | null;
  onStart: (project: Project, task: Task) => void;
  onClose: () => void;
  /// Something about the ticket was changed from here, so whatever the
  /// board or the filters say about it is out of date.
  onChanged?: () => void;
}) {
  const [issue, setIssue] = useState<Issue | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Only Jira's tickets are changed here so far.
  const writable = task.kind === "jira";
  const [editing, setEditing] = useState(false);
  // The ways out of the current status, read with the ticket.
  const [moves, setMoves] = useState<Transition[] | null>(null);
  // Which change is in flight -- `status`, `comment`, or a comment's id --
  // and what went wrong with the last one.
  const [working, setWorking] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  // A comment of yours being rewritten: its id and the text so far.
  const [rewriting, setRewriting] = useState<{ id: string; text: string } | null>(null);
  const { ask, dialog } = useConfirm();
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
    if (writable) {
      // No transitions is a status line that cannot be changed, not an error
      // worth a banner: a ticket you may read and not move is ordinary.
      api.transitions(task.tracker, task.key).then(
        (v) => live && setMoves(v),
        () => live && setMoves([]),
      );
    }
    return () => {
      live = false;
    };
  }, [task.tracker, task.key, writable]);

  /// One change, then the ticket read again -- here and wherever it is on a
  /// board.
  const change = async (what: string, run: () => Promise<void>): Promise<boolean> => {
    setWorking(what);
    setFailed(null);
    try {
      await run();
      read();
      onChanged?.();
      return true;
    } catch (e) {
      setFailed(messageOf(e));
      return false;
    } finally {
      setWorking(null);
    }
  };

  useEffect(() => {
    setIssue(null);
    return read();
  }, [read]);

  const send = () =>
    void change("comment", () => api.addComment(task.tracker, task.key, draft)).then(
      (ok) => ok && setDraft(""),
    );

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
          {writable && (
            <button
              className={editing ? "quiet-icon on" : "quiet-icon"}
              title={editing ? "stop editing" : "edit its fields"}
              aria-pressed={editing}
              onClick={() => setEditing((e) => !e)}
            >
              <Edit aria-label="edit" />
            </button>
          )}
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
      {failed && <p className="error">{failed}</p>}
      {!issue && !error && <Waiting />}
      {dialog}

      {issue && editing && (
        <TicketEditor
          task={task}
          onCancel={() => setEditing(false)}
          onSaved={() => {
            setEditing(false);
            read();
            onChanged?.();
          }}
        />
      )}

      {issue && !editing && (
        <>
          <dl className="ticket-fields">
            <dt>status</dt>
            <dd>
              {/* A status is a transition out of this one, so the choices are
                  the workflow's, named by where each lands. */}
              {moves && moves.length > 0 ? (
                <Select
                  className={`stage-select stage ${issue.stage}`}
                  aria-label="move it to"
                  disabled={working !== null}
                  value=""
                  onChange={(id) =>
                    void change("status", () => api.transition(task.tracker, task.key, id))
                  }
                  options={[
                    { value: "", label: issue.status, hint: "now" },
                    ...moves.map((t) => ({
                      value: t.id,
                      label: t.to,
                      hint: t.name.toLowerCase() !== t.to.toLowerCase() ? t.name : undefined,
                    })),
                  ]}
                />
              ) : (
                <span className={`stage ${issue.stage}`}>{issue.status}</span>
              )}
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
                <li key={c.id || i} className={c.author?.me ? "mine" : undefined}>
                  <div className="comment-head">
                    {c.author ? <Who person={c.author} /> : <Nobody>someone</Nobody>}
                    <span className="comment-when" title={c.created ?? undefined}>
                      {ago(c.created)}
                      {c.edited && <span title={`edited ${c.edited}`}> · edited</span>}
                    </span>
                    {/* Yours to change, and only yours: Jira lets an admin
                        edit anybody's, and this is not the place for that. */}
                    {writable && c.markdown && c.id && rewriting?.id !== c.id && (
                      <span className="comment-tools">
                        <button
                          className="quiet-icon"
                          title="edit your comment"
                          disabled={working !== null}
                          onClick={() => setRewriting({ id: c.id, text: c.markdown!.text })}
                        >
                          <Edit aria-label="edit" />
                        </button>
                        <button
                          className="quiet-icon danger"
                          title="delete your comment"
                          disabled={working !== null}
                          onClick={() =>
                            ask({
                              title: "Delete this comment?",
                              body: (
                                <>
                                  Your comment on <code>{task.key}</code> goes from Jira, for
                                  everybody. There is no undo.
                                </>
                              ),
                              confirm: "delete",
                              onConfirm: () =>
                                void change(c.id, () =>
                                  api.deleteComment(task.tracker, task.key, c.id),
                                ),
                            })
                          }
                        >
                          <Forget aria-label="delete" />
                        </button>
                      </span>
                    )}
                  </div>
                  {rewriting?.id === c.id && c.markdown ? (
                    <div className="comment-compose">
                      <MarkdownBox
                        rows={4}
                        autoFocus
                        value={{ text: rewriting.text, lost: c.markdown.lost }}
                        onChange={(text) => setRewriting({ id: c.id, text })}
                      />
                      <div className="comment-compose-actions">
                        <button
                          className="quiet"
                          disabled={working !== null}
                          onClick={() => setRewriting(null)}
                        >
                          cancel
                        </button>
                        <button
                          className="go"
                          disabled={working !== null || rewriting.text.trim().length === 0}
                          onClick={() =>
                            void change(c.id, () =>
                              api.editComment(task.tracker, task.key, c.id, rewriting.text),
                            ).then((ok) => ok && setRewriting(null))
                          }
                        >
                          save
                        </button>
                      </div>
                    </div>
                  ) : (
                    <Doc blocks={c.body} />
                  )}
                </li>
              ))}
            </ol>
            {writable && (
              <div
                className="comment-compose"
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && draft.trim() && !working) {
                    e.preventDefault();
                    send();
                  }
                }}
              >
                <MarkdownBox
                  rows={3}
                  placeholder="Add a comment — Markdown, ctrl+enter sends"
                  value={{ text: draft, lost: [] }}
                  onChange={setDraft}
                />
                <div className="comment-compose-actions">
                  <button
                    className="go"
                    disabled={working !== null || draft.trim().length === 0}
                    onClick={send}
                  >
                    comment
                  </button>
                </div>
              </div>
            )}
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
