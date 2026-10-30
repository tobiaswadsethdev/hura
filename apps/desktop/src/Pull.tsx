// One pull request, read in full and reviewed, beside the sidebar.
//
// The row says where a pull request stands; this says what it is: its
// description, every check by name, who has reviewed it and how, the
// conversation, and the files it changes with both sides of each. It also
// does the four things a reviewer does without needing the forge's page:
// comment, reply to a thread, approve, and ask for changes. Merging,
// resolving a thread and suggesting a change stay on the forge, one click
// away in the header.
//
// Three tabs rather than one long page, because a pull request with a hundred
// threads and forty files is three different things to read, and the
// overview is what it is opened for most of the time.

import { useCallback, useEffect, useRef, useState } from "react";
import * as monaco from "monaco-editor/editor/editor.api";

import { api, messageOf } from "./api";
import { Empty, Waiting } from "./Empty";
import type { FileDiff } from "./gen/FileDiff";
import type { Project } from "./gen/Project";
import type { PullCheck } from "./gen/PullCheck";
import type { PullDetail } from "./gen/PullDetail";
import type { PullFile } from "./gen/PullFile";
import type { PullFiles } from "./gen/PullFiles";
import type { PullRequest as Pull } from "./gen/PullRequest";
import type { PullThread } from "./gen/PullThread";
import type { PullVerdict } from "./gen/PullVerdict";
import type { Session } from "./gen/Session";
import {
  Back,
  Close,
  Elsewhere,
  Failing,
  Minus,
  NotText,
  Passing,
  Pending,
  PullDraft,
  PullRequest,
  Refresh,
  Review,
  Start,
  StateDot,
} from "./icons";
import { pullName } from "./inboxNotify";
import { Markdown } from "./Markdown";
import { openExternal } from "./open";
import { ensureTheme, languageOf, THEME } from "./panes/File";
import { Select } from "./Select";
import { ago, Nobody, Who } from "./Ticket";
import { MarkdownBox } from "./TicketEdit";

/// What a session started from a pull request is for: carrying on with it on
/// its own branch, or reading it and saying what is wrong.
export type PullStart = "work" | "review";

type Tab = "overview" | "conversation" | "files";

export function PullPanel({
  pull,
  projects,
  session,
  onStart,
  onSelect,
  onClose,
  onChanged,
}: {
  /// The sidebar's row, which is what is shown until the full read answers.
  pull: Pull;
  /// The projects that are clones of this pull request's repository, which
  /// are the only places a session for it can start.
  projects: Project[];
  /// The worktree already on its branch, if there is one.
  session: Session | null;
  onStart: (project: Project, pull: Pull, mode: PullStart) => void;
  onSelect: (session: string) => void;
  onClose: () => void;
  /// Something was said or decided from here, so the sidebar is out of date.
  onChanged?: () => void;
}) {
  const [detail, setDetail] = useState<PullDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("overview");
  // Which change is in flight (`comment`, `vote`, or a thread's id) and what
  // went wrong with the last one.
  const [working, setWorking] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [where, setWhere] = useState(projects[0]?.name ?? "");
  const project = projects.find((p) => p.name === where) ?? projects[0];
  const panel = useRef<HTMLElement>(null);

  // Focused on opening, so Escape closes this rather than the screen behind.
  useEffect(() => panel.current?.focus({ preventScroll: true }), []);

  const read = useCallback(() => {
    let live = true;
    setError(null);
    api.pull(pull.tracker, pull.repo, pull.id).then(
      (d) => live && setDetail(d),
      (e) => live && setError(messageOf(e)),
    );
    return () => {
      live = false;
    };
  }, [pull.tracker, pull.repo, pull.id]);

  useEffect(() => {
    setDetail(null);
    return read();
  }, [read]);

  /// One change, then the pull request read again, here and in the sidebar.
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

  const d = detail;
  const forge = pull.kind === "git-hub" ? "GitHub" : "Azure DevOps";
  const Mark = (d?.draft ?? pull.draft) ? PullDraft : PullRequest;
  const state = d?.state ?? "open";
  const fork = d?.fork ?? pull.fork;
  const comment = (body: string, thread: string | null) =>
    change(thread ?? "comment", () => api.pullComment(pull.tracker, pull.repo, pull.id, body, thread));

  return (
    <aside
      ref={panel}
      tabIndex={-1}
      className="ticket-panel pull-panel scrollbar-sleek"
      aria-label={`${pullName(pull)} ${pull.title}`}
      onKeyDown={(e) => {
        if (e.key === "Escape" && !e.defaultPrevented) {
          e.preventDefault();
          onClose();
        }
      }}
    >
      <header className="ticket-panel-head">
        <Mark className={`pull-mark ${state}${d?.draft ? " draft" : ""}`} aria-hidden />
        <span className="ticket-panel-key">{pullName(pull)}</span>
        {(d?.draft ?? pull.draft) && <span className="ticket-type">draft</span>}
        {state !== "open" && <span className={`pull-state ${state}`}>{state}</span>}
        <span className="ticket-panel-tools">
          <button className="quiet-icon" title="read it again" onClick={() => void read()}>
            <Refresh aria-label="refresh" />
          </button>
          <a
            className="quiet-icon"
            href={pull.url}
            target="_blank"
            rel="noreferrer"
            title={`open on ${forge}`}
          >
            <Elsewhere aria-label="open in the browser" />
          </a>
          <button className="quiet-icon" title="close" onClick={onClose}>
            <Close aria-label="close" />
          </button>
        </span>
      </header>

      <h2 className="ticket-panel-title">{d?.title ?? pull.title}</h2>

      <p className="pull-line">
        {d?.author ? <Who person={d.author} /> : <span>{pull.author}</span>}
        <span className="hint">wants</span>
        <code title="its branch">{d?.branch ?? pull.branch}</code>
        <span className="hint">into</span>
        <code title="the branch it merges into">{d?.base ?? pull.base}</code>
        {d?.created && <span className="hint">· opened {ago(d.created)}</span>}
        {d?.additions != null && d.deletions != null && (
          <span className="pull-size" title={`${d.changed_files ?? "?"} files`}>
            <span className="added">+{d.additions}</span>{" "}
            <span className="removed">−{d.deletions}</span>
          </span>
        )}
      </p>

      {(d?.conflicts ?? pull.conflicts) && state === "open" && (
        <p className="warn">
          It conflicts with <code>{d?.base ?? pull.base}</code>, so it cannot merge as it stands.
        </p>
      )}
      {error && <p className="error">{error}</p>}
      {failed && <p className="error">{failed}</p>}
      {d?.warnings.map((w) => (
        <p key={w} className="warn">
          {w}
        </p>
      ))}

      <nav className="pull-tabs" role="tablist" aria-label="what to read">
        {(
          [
            ["overview", "overview", null],
            ["conversation", "conversation", d ? d.conversation.length : pull.comments],
            ["files", "files", d?.changed_files ?? null],
          ] as const
        ).map(([key, label, count]) => (
          <button
            key={key}
            role="tab"
            aria-selected={tab === key}
            className={tab === key ? "on" : undefined}
            onClick={() => setTab(key)}
          >
            {label}
            {count != null && <span className="count">{count}</span>}
          </button>
        ))}
      </nav>

      {!d && !error && <Waiting />}

      {d && tab === "overview" && (
        <Overview
          detail={d}
          working={working}
          onVote={(verdict, body) =>
            change("vote", () => api.pullVote(pull.tracker, pull.repo, pull.id, verdict, body))
          }
        />
      )}
      {d && tab === "conversation" && (
        <Conversation detail={d} forge={forge} working={working} onComment={comment} />
      )}
      {tab === "files" && <Files pull={pull} />}

      <footer className="ticket-panel-foot">
        {session ? (
          <button
            className="inbox-worktree pull-go"
            title={`go to ${session.name}`}
            onClick={() => onSelect(session.name)}
          >
            <StateDot state={session.state} />
            <span>{session.name}</span>
          </button>
        ) : fork ? (
          <span className="hint">
            Opened from a fork, so its branch is not in this repository to start a worktree on.
          </span>
        ) : state !== "open" ? (
          <span className="hint">It is {state}, so there is nothing to start on.</span>
        ) : projects.length === 0 ? (
          <span className="hint">
            No project is a clone of <code>{pull.repo}</code>. Make one to start a worktree here.
          </span>
        ) : (
          <>
            {projects.length > 1 ? (
              <Select
                className="ticket-project"
                aria-label="project to start it in"
                value={project?.name ?? ""}
                onChange={setWhere}
                options={projects.map((p) => ({ value: p.name, label: p.name }))}
              />
            ) : (
              <span className="hint">{project?.name}</span>
            )}
            <button
              className="quiet"
              disabled={!project}
              title="a worktree on its branch that reads it and reports back, without pushing"
              onClick={() => project && onStart(project, pull, "review")}
            >
              <Review /> review it
            </button>
            <button
              className="go"
              disabled={!project}
              title="a worktree on its branch, whose pushes update this pull request"
              onClick={() => project && onStart(project, pull, "work")}
            >
              <Start /> work on it
            </button>
          </>
        )}
      </footer>
    </aside>
  );
}

// ---------------------------------------------------------------- overview

function Overview({
  detail: d,
  working,
  onVote,
}: {
  detail: PullDetail;
  working: string | null;
  onVote: (verdict: PullVerdict, body: string) => Promise<boolean>;
}) {
  return (
    <>
      <section className="ticket-section">
        <h3>description</h3>
        {d.body.trim() ? <Markdown text={d.body} /> : <Nobody>no description</Nobody>}
      </section>

      <section className="ticket-section">
        <h3>
          checks <span className="count">{d.checks.length}</span>
        </h3>
        {d.checks.length === 0 ? (
          <Nobody>
            {d.kind === "azure-dev-ops" ? "no policy or status says anything" : "none ran"}
          </Nobody>
        ) : (
          <ul className="pull-checks">
            {/* Failing first: that is what the list is read for. */}
            {[...d.checks]
              .sort((a, b) => CHECK_ORDER[a.state] - CHECK_ORDER[b.state])
              .map((c, i) => (
                <CheckRow key={`${c.name}:${i}`} check={c} />
              ))}
          </ul>
        )}
      </section>

      <section className="ticket-section">
        <h3>
          reviewers <span className="count">{d.reviewers.length}</span>
        </h3>
        {d.reviewers.length === 0 ? (
          <Nobody>nobody has been asked</Nobody>
        ) : (
          <ul className="pull-reviewers">
            {d.reviewers.map((r, i) => (
              <li key={`${r.who.name}:${i}`}>
                <Who person={r.who} />
                <VerdictWord verdict={r.verdict} />
                {r.required && <span className="tag">required</span>}
              </li>
            ))}
          </ul>
        )}
      </section>

      {d.offers.length > 0 && (
        <ReviewBox detail={d} working={working !== null} onVote={onVote} />
      )}
    </>
  );
}

const CHECK_ORDER: Record<PullCheck["state"], number> = {
  failing: 0,
  pending: 1,
  passing: 2,
  neutral: 3,
  skipped: 4,
};

function CheckRow({ check: c }: { check: PullCheck }) {
  const Mark =
    c.state === "passing"
      ? Passing
      : c.state === "failing"
        ? Failing
        : c.state === "pending"
          ? Pending
          : Minus;
  const tone =
    c.state === "passing" ? "ok" : c.state === "failing" ? "bad" : c.state === "pending" ? "running" : "";
  return (
    <li>
      <Mark className={`inbox-checks ${tone}`} aria-label={c.state} />
      <span className="pull-check-name" title={c.name}>
        {c.name}
      </span>
      {c.required && <span className="tag">required</span>}
      {c.detail && <span className="hint pull-check-detail">{c.detail}</span>}
      {c.url && (
        <button
          className="quiet-icon"
          title="open its log in the browser"
          onClick={() => openExternal(c.url!)}
        >
          <Elsewhere aria-label="open" />
        </button>
      )}
    </li>
  );
}

const VERDICT: Record<PullVerdict, { said: string; tone: string; give: string }> = {
  approved: { said: "approved", tone: "ok", give: "approve" },
  suggestions: { said: "approved with suggestions", tone: "ok", give: "approve with suggestions" },
  "changes-requested": { said: "asked for changes", tone: "bad", give: "request changes" },
  waiting: { said: "waiting for the author", tone: "bad", give: "wait for author" },
  rejected: { said: "rejected", tone: "bad", give: "reject" },
  commented: { said: "commented", tone: "", give: "comment" },
  dismissed: { said: "dismissed", tone: "", give: "dismiss" },
  pending: { said: "not yet", tone: "", give: "reset my vote" },
};

function VerdictWord({ verdict }: { verdict: PullVerdict }) {
  const v = VERDICT[verdict];
  return <span className={`pull-verdict ${v.tone}`}>{v.said}</span>;
}

/// Your review: a comment to go with it, and the verdicts this forge has.
///
/// GitHub's two are buttons, the way its own review box has them. Azure
/// DevOps has five steps, which are a list and a button rather than five
/// buttons in a row.
function ReviewBox({
  detail: d,
  working,
  onVote,
}: {
  detail: PullDetail;
  working: boolean;
  onVote: (verdict: PullVerdict, body: string) => Promise<boolean>;
}) {
  const [body, setBody] = useState("");
  const offers = d.offers.filter((v) => v !== "pending" || (d.my_vote && d.my_vote !== "pending"));
  const [vote, setVote] = useState<PullVerdict>(offers[0] ?? "approved");
  const give = (verdict: PullVerdict) =>
    void onVote(verdict, body).then((ok) => ok && setBody(""));
  const github = d.kind === "git-hub";

  return (
    <section className="ticket-section pull-review">
      <h3>your review</h3>
      {d.my_vote && d.my_vote !== "pending" && (
        <p className="hint">
          You have <VerdictWord verdict={d.my_vote} />. A new one replaces it.
        </p>
      )}
      <div className="comment-compose">
        <MarkdownBox
          rows={3}
          placeholder={
            github
              ? "What to say with it, in Markdown. A request for changes needs this."
              : "What to say with it, in Markdown. Posted as a comment beside the vote."
          }
          value={{ text: body, lost: [] }}
          onChange={setBody}
        />
        <div className="comment-compose-actions">
          {github ? (
            offers.map((v) => (
              <button
                key={v}
                className={v === "approved" ? "go" : "quiet"}
                disabled={working || (v === "changes-requested" && body.trim().length === 0)}
                title={
                  v === "changes-requested" && body.trim().length === 0
                    ? "say what should change first"
                    : undefined
                }
                onClick={() => give(v)}
              >
                {VERDICT[v].give}
              </button>
            ))
          ) : (
            <>
              <Select
                aria-label="your vote"
                value={vote}
                onChange={(v) => setVote(v as PullVerdict)}
                options={offers.map((v) => ({ value: v, label: VERDICT[v].give }))}
              />
              <button className="go" disabled={working} onClick={() => give(vote)}>
                vote
              </button>
            </>
          )}
        </div>
      </div>
    </section>
  );
}

// ------------------------------------------------------------ conversation

function Conversation({
  detail: d,
  forge,
  working,
  onComment,
}: {
  detail: PullDetail;
  forge: string;
  working: string | null;
  onComment: (body: string, thread: string | null) => Promise<boolean>;
}) {
  const [draft, setDraft] = useState("");
  const send = () => void onComment(draft, null).then((ok) => ok && setDraft(""));

  return (
    <section className="ticket-section">
      {d.older > 0 && <p className="hint">{d.older} older ones are only on {forge}.</p>}
      {d.conversation.length === 0 && <Nobody>nothing said yet</Nobody>}
      <ol className="ticket-comments-list pull-conversation">
        {d.conversation.map((t, i) => (
          <Entry
            key={`${t.id}:${t.at}:${i}`}
            thread={t}
            working={working}
            onReply={(body) => onComment(body, t.id)}
          />
        ))}
      </ol>
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
          placeholder="Comment on the pull request, in Markdown. Ctrl+Enter sends it."
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
    </section>
  );
}

/// One entry: a review's verdict, a comment, or a thread on a line.
///
/// A resolved thread starts shut, showing where it was and who started it:
/// it was settled, and a long review is mostly settled threads.
function Entry({
  thread: t,
  working,
  onReply,
}: {
  thread: PullThread;
  working: string | null;
  onReply: (body: string) => Promise<boolean>;
}) {
  const [open, setOpen] = useState(!t.resolved);
  const [replying, setReplying] = useState(false);
  const [draft, setDraft] = useState("");

  const comments = (
    <>
      {t.comments.map((c, i) => (
        <div key={i} className={`pull-comment${c.author?.me ? " mine" : ""}`}>
          <div className="comment-head">
            {c.author ? <Who person={c.author} /> : <Nobody>someone</Nobody>}
            <span className="comment-when" title={c.created ?? undefined}>
              {ago(c.created)}
            </span>
          </div>
          <Markdown text={c.body} />
        </div>
      ))}
    </>
  );

  if (!t.path) {
    // A review's verdict, with what was written with it, or a comment.
    return (
      <li className={t.verdict ? "pull-verdict-entry" : undefined}>
        {t.verdict && (
          <div className="comment-head">
            {t.author ? <Who person={t.author} /> : <Nobody>someone</Nobody>}
            <VerdictWord verdict={t.verdict} />
            <span className="comment-when" title={t.at ?? undefined}>
              {ago(t.at)}
            </span>
          </div>
        )}
        {t.verdict
          ? t.comments.map((c, i) => <Markdown key={i} text={c.body} />)
          : comments}
      </li>
    );
  }

  return (
    <li className={`pull-thread${t.resolved ? " resolved" : ""}`}>
      <button className="pull-thread-head" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <code className="pull-thread-at" title={t.path}>
          {t.path.replace(/^\//, "")}
          {t.line != null && `:${t.line}`}
        </code>
        {t.resolved && <span className="tag">resolved</span>}
        {t.outdated && <span className="tag">outdated</span>}
        {!open && t.comments[0]?.author && (
          <span className="hint">
            {t.comments[0].author.name}
            {t.comments.length > 1 && ` and ${t.comments.length - 1} more`}
          </span>
        )}
      </button>
      {open && comments}
      {open && t.can_reply && !replying && (
        <button className="link pull-reply" onClick={() => setReplying(true)}>
          reply
        </button>
      )}
      {open && replying && (
        <div
          className="comment-compose"
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && draft.trim() && !working) {
              e.preventDefault();
              void onReply(draft).then((ok) => ok && (setDraft(""), setReplying(false)));
            }
          }}
        >
          <MarkdownBox
            rows={2}
            autoFocus
            placeholder="Reply, in Markdown. Ctrl+Enter sends it."
            value={{ text: draft, lost: [] }}
            onChange={setDraft}
          />
          <div className="comment-compose-actions">
            <button className="quiet" onClick={() => setReplying(false)}>
              cancel
            </button>
            <button
              className="go"
              disabled={working !== null || draft.trim().length === 0}
              onClick={() =>
                void onReply(draft).then((ok) => ok && (setDraft(""), setReplying(false)))
              }
            >
              reply
            </button>
          </div>
        </div>
      )}
    </li>
  );
}

// ------------------------------------------------------------------- files

/// The files it changes, and one of them at a time, both sides.
///
/// Read when the tab is first opened rather than with the pull request: the
/// list is a request of its own, and most of the time it is not looked at.
function Files({ pull }: { pull: Pull }) {
  const [files, setFiles] = useState<PullFiles | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<number | null>(null);

  useEffect(() => {
    let live = true;
    api.pullFiles(pull.tracker, pull.repo, pull.id).then(
      (f) => live && setFiles(f),
      (e) => live && setError(messageOf(e)),
    );
    return () => {
      live = false;
    };
  }, [pull.tracker, pull.repo, pull.id]);

  if (error) return <p className="error">{error}</p>;
  if (!files) return <Waiting />;
  if (files.files.length === 0) return <Empty icon={PullRequest} note="it changes no files" />;

  if (open !== null && files.files[open]) {
    const file = files.files[open]!;
    return (
      <section className="pull-file">
        <header className="pull-file-head">
          <button className="quiet-icon" title="all the files" onClick={() => setOpen(null)}>
            <Back aria-label="back" />
          </button>
          <code className="pull-file-path" title={file.path}>
            {file.old_path ? `${file.old_path} → ` : ""}
            {file.path}
          </code>
          <span className="hint">
            {open + 1} of {files.files.length}
          </span>
          <button
            className="quiet"
            disabled={open === 0}
            onClick={() => setOpen(open - 1)}
          >
            previous
          </button>
          <button
            className="quiet"
            disabled={open === files.files.length - 1}
            onClick={() => setOpen(open + 1)}
          >
            next
          </button>
        </header>
        <FileSides key={`${file.path}:${open}`} tracker={pull.tracker} files={files} file={file} />
      </section>
    );
  }

  return (
    <section className="ticket-section">
      {files.truncated && (
        <p className="hint">The forge lists only the first {files.files.length}; the rest are on its page.</p>
      )}
      <ul className="pull-files">
        {files.files.map((f, i) => (
          <li key={`${f.path}:${i}`}>
            <button onClick={() => setOpen(i)} title={f.old_path ? `${f.old_path} → ${f.path}` : f.path}>
              <span className={`pull-file-change ${f.change}`} aria-label={f.change}>
                {f.change[0]!.toUpperCase()}
              </span>
              <span className="pull-file-path">{f.path.replace(/^\//, "")}</span>
              {f.additions != null && f.deletions != null && (
                <span className="pull-size">
                  <span className="added">+{f.additions}</span>{" "}
                  <span className="removed">−{f.deletions}</span>
                </span>
              )}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

function FileSides({ tracker, files, file }: { tracker: string; files: PullFiles; file: PullFile }) {
  const [sides, setSides] = useState<FileDiff | null>(null);
  const [error, setError] = useState<string | null>(null);
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let live = true;
    api.pullFile(tracker, files, file).then(
      (s) => live && setSides(s),
      (e) => live && setError(messageOf(e)),
    );
    return () => {
      live = false;
    };
  }, [tracker, files, file]);

  useEffect(() => {
    const element = host.current;
    if (!element || !sides || sides.binary) return;
    ensureTheme();
    const editor = monaco.editor.createDiffEditor(element, {
      theme: THEME,
      readOnly: true,
      domReadOnly: true,
      automaticLayout: true,
      // Two columns when the panel is wide enough for them, and Monaco's one
      // column of removed-then-added lines when it is not.
      renderSideBySide: true,
      useInlineViewWhenSpaceIsLimited: true,
      // A review reads the changes. Unchanged stretches fold away, and open
      // when clicked.
      hideUnchangedRegions: { enabled: true },
      // The colours say which lines went and which came; the + and − beside
      // them are drawn in an icon font, and say nothing the colours do not.
      renderIndicators: false,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      fontSize: 13,
    });
    const original = monaco.editor.createModel(sides.original, languageOf(file.path));
    const modified = monaco.editor.createModel(sides.modified, languageOf(file.path));
    editor.setModel({ original, modified });
    return () => {
      original.dispose();
      modified.dispose();
      editor.dispose();
    };
  }, [sides, file.path]);

  if (error) return <p className="error">{error}</p>;
  if (!sides) return <Waiting />;
  if (sides.binary) {
    return <Empty icon={NotText} note="binary, or too large to show here; it is on the forge's page" />;
  }
  return (
    <>
      <p className="hint pull-file-sides">
        {sides.original_label} → {sides.modified_label}
      </p>
      <div className="monaco pull-monaco" ref={host} />
    </>
  );
}
