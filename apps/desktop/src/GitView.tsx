// The working copy, as git sees it: what has changed, and what to do about it.
//
// A sidebar rather than a tab, because it is a place you work *from* -- you
// look at what changed, open one, come back, stage it. A tab you have to
// reselect after every diff would make that four clicks instead of two.
//
// **The agent is editing while this is on screen.** Every action re-reads the
// status from the server rather than adjusting the list it already had: staging
// a file the agent has since rewritten stages the rewrite, and a list that
// assumed otherwise would be quietly lying about what is about to be committed.
//
// Laid out the way the editor beside it lays out the same thing: the message
// box on top, where it stays put however long the list underneath grows, and
// each list a tree of the files tree's own folders and icons. A flat list of
// full paths read as a column of `apps/desktop/src/` with the names cut off
// the end; a tree says the directory once and the name in full.

import { useCallback, useEffect, useMemo, useState } from "react";

import { api, messageOf, type GitAnswer } from "./api";
import { useConfirm } from "./Confirm";
import { copy, useContextMenu } from "./ContextMenu";
import type { Against } from "./gen/Against";
import type { Change } from "./gen/Change";
import type { ChangedFile } from "./gen/ChangedFile";
import type { GitOp } from "./gen/GitOp";
import { Empty, Waiting } from "./Empty";
import {
  Branch,
  Busy,
  Chevron,
  Clean,
  Commit,
  Copy,
  Diff,
  Fetch,
  FileIcon,
  Folder,
  Minus,
  Plus,
  Publish,
  Pull,
  Push,
  Refresh,
  Revert,
  StageAll,
  UnstageAll,
} from "./icons";

/// One letter per change, which is what every git client uses and what fits
/// beside a filename.
const MARK: Record<Change, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  untracked: "U",
  conflicted: "!",
};

/// The remote operations. Each is a picture of what it does to which side --
/// see `icons.tsx` -- and `push` turns into `publish` for a branch the remote
/// has never heard of, which is a different operation and wears a different
/// glyph rather than a changing tooltip on the same one.
type RemoteOp = "fetch" | "pull" | "push";

export function GitView({
  server,
  name,
  onOpenDiff,
}: {
  server: string;
  name: string;
  onOpenDiff: (path: string, against: Against) => void;
}) {
  const [answer, setAnswer] = useState<GitAnswer | null>(null);
  const [error, setError] = useState<string | null>(null);
  /// What is under way, so the one control that started it can say so. Every
  /// control is disabled while anything is: two writes racing each other on
  /// the index is how a stage lands after the commit it was meant for.
  const [running, setRunning] = useState<string | null>(null);
  const busy = running !== null;
  /// Discarding a file is the one thing in this pane that destroys work, so it
  /// is asked for in a dialog of the window's own rather than the webview's.
  const { ask, dialog: confirmation } = useConfirm();
  const [message, setMessage] = useState("");

  const load = useCallback(() => {
    api
      .gitStatus(server, name)
      .then(setAnswer)
      .catch((e) => setError(messageOf(e)));
  }, [server, name]);

  useEffect(load, [load]);

  /// One or more operations in a row, keeping the answer to the last. More
  /// than one is staging a whole section, which is a stage per top-level entry
  /// rather than one of the whole tree: `git add -A -- ''` is an error, and
  /// asking the server for `.` would be a change it does not have yet.
  const act = async (what: string, actions: GitOp[]) => {
    setRunning(what);
    setError(null);
    try {
      let last: GitAnswer | null = null;
      for (const action of actions) last = await api.git(server, name, action);
      if (last) setAnswer(last);
      if (actions.some((a) => a.do === "commit")) setMessage("");
    } catch (e) {
      setError(messageOf(e));
      load();
    } finally {
      setRunning(null);
    }
  };

  if (error && !answer) return <p className="error">{error}</p>;
  if (!answer) return <Waiting />;

  const { status } = answer;
  const nothing = status.staged.length === 0 && status.unstaged.length === 0;
  const canCommit = !busy && message.trim() !== "" && status.staged.length > 0;
  const commit = () => canCommit && void act("commit", [{ do: "commit", message }]);

  const remote = (op: RemoteOp, Icon: typeof Fetch, title: string) => (
    <button
      className="quiet-icon"
      disabled={busy}
      title={title}
      aria-label={title}
      onClick={() => void act(op, [{ do: op }])}
    >
      {running === op ? <Busy className="turning" /> : <Icon />}
    </button>
  );

  return (
    <div className="git">
      {confirmation}

      {/* The branch the way the sidebar writes it, glyph and all, with the
          remote half on the line underneath in the ports pane's `→` form. */}
      <header className="git-head">
        <Branch className="git-branch-glyph" />
        <span className="branch" title={status.branch}>
          {status.branch}
        </span>
        <span className="git-ops">
          {remote("fetch", Fetch, "fetch the remote's refs without touching the working copy")}
          {remote("pull", Pull, "pull")}
          {status.upstream
            ? remote("push", Push, "push")
            : remote("push", Publish, "publish this branch, which is not on the remote yet")}
          <button className="quiet-icon" disabled={busy} onClick={load} title="re-read git" aria-label="re-read git">
            <Refresh />
          </button>
        </span>
      </header>
      <div className="git-sync">
        {status.upstream ? (
          <>
            <span className="upstream" title={status.upstream}>
              → {status.upstream}
            </span>
            {status.ahead > 0 && <span className="ahead">{status.ahead} ahead</span>}
            {status.behind > 0 && <span className="behind">{status.behind} behind</span>}
            {status.ahead === 0 && status.behind === 0 && <span>in sync</span>}
          </>
        ) : (
          // Never pushed is not the same as in sync, and the publish glyph
          // above is the button that says so.
          <span className="none">not on the remote yet</span>
        )}
      </div>

      {error && <p className="error">{error}</p>}
      {answer.said.trim() && <pre className="said">{answer.said.trim()}</pre>}

      {nothing ? (
        // A tick rather than `nothing changed`, and it is the only absence in
        // the window drawn in the colour that means "it passed": every other
        // empty pane is neutral because empty is neither good nor bad, and a
        // clean working copy is the one case where it is an answer.
        <Empty icon={Clean} tone="ok" />
      ) : (
        <>
          <div className="commit">
            <textarea
              rows={2}
              value={message}
              placeholder="commit message"
              title="ctrl+enter commits"
              onChange={(e) => setMessage(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                  e.preventDefault();
                  commit();
                }
              }}
            />
            <button
              className="go"
              // Nothing staged is the one case git refuses outright, and a
              // button that produces an error you could have been shown is a
              // bad button.
              disabled={!canCommit}
              title={status.staged.length === 0 ? "stage something first" : undefined}
              onClick={commit}
            >
              {running === "commit" ? <Busy className="turning" /> : <Commit />}
              commit
              {status.staged.length > 0 && <span className="n">{status.staged.length}</span>}
            </button>
          </div>

          <Section
            title="staged"
            entries={status.staged}
            busy={busy}
            onOpen={(p) => onOpenDiff(p, "staged")}
            verb="unstage"
            run={(paths) => void act("unstage", paths.map((path) => ({ do: "unstage", path })))}
          />
          <Section
            title="changes"
            entries={status.unstaged}
            busy={busy}
            onOpen={(p) => onOpenDiff(p, "worktree")}
            verb="stage"
            run={(paths) => void act("stage", paths.map((path) => ({ do: "stage", path })))}
            discard={(path) =>
              ask({
                title: "Throw away your changes?",
                body: (
                  <>
                    <code>{path}</code> goes back to what it was, and the agent may be part-way
                    through writing it.
                  </>
                ),
                confirm: "discard",
                onConfirm: () => void act("discard", [{ do: "discard", path }]),
              })
            }
          />
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// The lists, as trees.
// ---------------------------------------------------------------------------

type Node = {
  /// What the row says: one segment, or several joined with `/` when a
  /// directory holds nothing but one other directory.
  name: string;
  /// The whole path, which is what git is handed for a folder as for a file.
  path: string;
  children: Node[];
  file?: ChangedFile;
};

/// The paths, folded into directories.
///
/// A chain of directories with one child each is one row, `apps/desktop/src`,
/// as the editor draws it: three rows of indentation to reach the file is
/// three rows of nothing, and in a repository with its code three levels down
/// it would be every list.
function treeOf(entries: ChangedFile[]): Node[] {
  const root: Node = { name: "", path: "", children: [] };
  for (const file of entries) {
    const parts = file.path.split("/");
    let at = root;
    parts.forEach((part, i) => {
      const path = parts.slice(0, i + 1).join("/");
      if (i === parts.length - 1) {
        at.children.push({ name: part, path, children: [], file });
        return;
      }
      let dir = at.children.find((c) => !c.file && c.name === part);
      if (!dir) {
        dir = { name: part, path, children: [] };
        at.children.push(dir);
      }
      at = dir;
    });
  }
  const fold = (n: Node): Node => {
    let node = n;
    while (!node.file && node.children.length === 1 && !node.children[0].file) {
      const only = node.children[0];
      node = { ...only, name: `${node.name}/${only.name}` };
    }
    const children = node.children
      .map(fold)
      // Folders first and then by name, the files tree's order.
      .sort((a, b) => Number(!!a.file) - Number(!!b.file) || a.name.localeCompare(b.name));
    return { ...node, children };
  };
  return fold(root).children;
}

function Section({
  title,
  entries,
  busy,
  onOpen,
  verb,
  run,
  discard,
}: {
  title: string;
  entries: ChangedFile[];
  busy: boolean;
  onOpen: (path: string) => void;
  verb: "stage" | "unstage";
  /// Stage or unstage these paths, files or folders.
  run: (paths: string[]) => void;
  discard?: (path: string) => void;
}) {
  const nodes = useMemo(() => treeOf(entries), [entries]);
  const [shut, setShut] = useState(false);
  /// Folders closed by hand. Everything starts open: this list is short and
  /// exists to be read, unlike the files tree, which is the whole repository.
  const [closed, setClosed] = useState<Set<string>>(new Set());

  if (entries.length === 0) return null;
  const All = verb === "stage" ? StageAll : UnstageAll;

  const toggle = (path: string) =>
    setClosed((s) => {
      const next = new Set(s);
      next.has(path) ? next.delete(path) : next.add(path);
      return next;
    });

  return (
    <section className="changes">
      <h3>
        <button className="section-toggle" onClick={() => setShut((s) => !s)} aria-expanded={!shut}>
          <Chevron open={!shut} />
          {title} <span className="count">{entries.length}</span>
        </button>
        <button
          className="quiet-icon"
          disabled={busy}
          title={`${verb} all`}
          aria-label={`${verb} all`}
          onClick={() => run(nodes.map((n) => n.path))}
        >
          <All />
        </button>
      </h3>
      {!shut && (
        <div className="git-tree">
          {nodes.map((n) => (
            <Row
              key={n.path}
              node={n}
              depth={0}
              closed={closed}
              toggle={toggle}
              busy={busy}
              onOpen={onOpen}
              verb={verb}
              run={run}
              discard={discard}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function Row({
  node,
  depth,
  closed,
  toggle,
  busy,
  onOpen,
  verb,
  run,
  discard,
}: {
  node: Node;
  depth: number;
  closed: Set<string>;
  toggle: (path: string) => void;
  busy: boolean;
  onOpen: (path: string) => void;
  verb: "stage" | "unstage";
  run: (paths: string[]) => void;
  discard?: (path: string) => void;
}) {
  const menu = useContextMenu();
  const { file } = node;
  const open = !closed.has(node.path);
  const Act = verb === "stage" ? Plus : Minus;
  const label = verb === "stage" ? "Stage" : "Unstage";

  return (
    <>
      <div
        className={`change${file ? ` ${file.change}` : " dir"}`}
        // Everything about the row, in words. The one action that stays on it
        // is the one you do forty times an hour; discarding throws work away
        // and lives here, behind a deliberate gesture -- and only for a file,
        // because a folder's worth of discarding is one click too easy.
        {...menu(() =>
          file
            ? [
                { label: "Open diff", icon: Diff, run: () => onOpen(file.path) },
                { label, icon: Act, disabled: busy, run: () => run([file.path]) },
                "separator",
                { label: "Copy path", icon: Copy, hint: file.path, run: () => copy(file.path) },
                ...(discard
                  ? [
                      "separator" as const,
                      {
                        label: "Discard changes…",
                        icon: Revert,
                        danger: true,
                        disabled: busy,
                        run: () => discard(file.path),
                      },
                    ]
                  : []),
              ]
            : [
                { label: open ? "Collapse" : "Expand", run: () => toggle(node.path) },
                { label: `${label} folder`, icon: Act, disabled: busy, run: () => run([node.path]) },
                "separator",
                { label: "Copy path", icon: Copy, hint: node.path, run: () => copy(node.path) },
              ],
        )}
      >
        <button
          className="open"
          style={{ paddingLeft: depth * 12 + 6 }}
          onClick={() => (file ? onOpen(file.path) : toggle(node.path))}
          title={node.path}
        >
          <span className="twist">{!file && <Chevron open={open} />}</span>
          {file ? <FileIcon name={file.path} /> : <Folder name={node.name.split("/").pop()!} open={open} />}
          <span className="label">{node.name}</span>
        </button>
        <button
          className="act"
          disabled={busy}
          title={file ? verb : `${verb} folder`}
          aria-label={file ? verb : `${verb} folder`}
          onClick={() => run([node.path])}
        >
          <Act />
        </button>
        {file && (
          <span className={`mark ${file.change}`} title={file.change}>
            {MARK[file.change]}
          </span>
        )}
      </div>
      {!file &&
        open &&
        node.children.map((c) => (
          <Row
            key={c.path}
            node={c}
            depth={depth + 1}
            closed={closed}
            toggle={toggle}
            busy={busy}
            onOpen={onOpen}
            verb={verb}
            run={run}
            discard={discard}
          />
        ))}
    </>
  );
}
