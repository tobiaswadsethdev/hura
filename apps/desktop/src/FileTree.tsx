// The working copy, as a tree.
//
// One directory per request, expanded as it is opened: a repository is tens of
// thousands of files, every listing is an exec into the sandbox, and a tree
// only ever shows the few branches someone has opened. Loading it all up front
// would cost a second of execs to draw something nobody has looked at.
//
// Read-only, because the agent owns the working copy. Two writers with no
// shared lock is how a file ends up with half of each; what you want here is to
// see what it did, and the review is how you say something about it.
//
// Each listing, and which folders are open, is remembered for when the tree is
// drawn again -- after the git pane, or another worktree -- so it comes back
// as it was and is read again behind that, rather than starting from a
// spinner at the root and closed all the way down.

import { useCallback, useEffect, useState } from "react";

import { api, messageOf } from "./api";
import { keyFor, Remembered } from "./cache";
import { copy, useContextMenu } from "./ContextMenu";
import type { Entry } from "./gen/Entry";
import { Empty, Waiting } from "./Empty";
import { Chevron, Copy, FileIcon, Folder, NoFiles } from "./icons";

/// Directories as last read, by worktree and path.
const listings = new Remembered<Entry[]>(500);
/// The folders open in each worktree's tree.
const opened = new Remembered<Set<string>>(50);

export function FileTree({
  server,
  name,
  onOpen,
}: {
  server: string;
  name: string;
  onOpen: (path: string) => void;
}) {
  const tree = keyFor(server, name);
  const [open, setOpen] = useState<Set<string>>(() => opened.get(tree) ?? new Set());
  const toggle = useCallback(
    (full: string) =>
      setOpen((s) => {
        const next = new Set(s);
        next.has(full) ? next.delete(full) : next.add(full);
        opened.set(tree, next);
        return next;
      }),
    [tree],
  );

  return (
    // No heading. There used to be a sticky `FILES` above this, and it was a
    // label repeating the word on the dock tab two lines above it -- the one
    // that was pressed to get here. A pane in a tabbed strip is already named
    // by its tab, and eleven pixels of uppercase saying so again is eleven
    // pixels of the tree.
    <div className="files">
      <Level server={server} name={name} path="" depth={0} open={open} toggle={toggle} onOpen={onOpen} />
    </div>
  );
}

/// One directory's worth, and the directories opened under it.
function Level({
  server,
  name,
  path,
  depth,
  open,
  toggle,
  onOpen,
}: {
  server: string;
  name: string;
  path: string;
  depth: number;
  open: Set<string>;
  toggle: (path: string) => void;
  onOpen: (path: string) => void;
}) {
  const key = keyFor(server, name, path);
  const [entries, setEntries] = useState<Entry[] | null>(() => listings.get(key) ?? null);
  const [error, setError] = useState<string | null>(null);
  const menu = useContextMenu();

  // Read every time the level is drawn, remembered or not -- the agent is
  // still editing, and a tree of what was there an hour ago would be a tree
  // of what used to be. What was remembered is only what shows meanwhile.
  useEffect(() => {
    let live = true;
    setEntries(listings.get(key) ?? null);
    api
      .files(server, name, path)
      .then((dir) => {
        listings.set(key, dir.entries);
        if (live) {
          setEntries(dir.entries);
          setError(null);
        }
      })
      .catch((e) => live && setError(messageOf(e)));
    return () => {
      live = false;
    };
  }, [key, server, name, path]);

  const activate = (dir: boolean, full: string) => (dir ? toggle(full) : onOpen(full));

  if (error) return <p className="error">{error}</p>;
  // Indented to the level being read, so the wait appears where the
  // directory is about to, rather than at the root of the tree.
  if (!entries)
    return (
      <div style={{ paddingLeft: depth * 12 + 8 }}>
        <Waiting size="row" />
      </div>
    );

  return (
    <>
      {entries.map((e) => {
        const full = path ? `${path}/${e.name}` : e.name;
        const isOpen = open.has(full);
        return (
          <div key={full}>
            <button
              className={`entry${e.dir ? " dir" : ""}`}
              style={{ paddingLeft: depth * 12 + 8 }}
              onClick={() => activate(e.dir, full)}
              {...menu(() => [
                {
                  label: e.dir ? (isOpen ? "Collapse" : "Expand") : "Open",
                  run: () => activate(e.dir, full),
                },
                "separator",
                { label: "Copy Path", icon: Copy, hint: full, run: () => copy(full) },
                { label: "Copy Name", icon: Copy, run: () => copy(e.name) },
              ])}
            >
              <span className="twist">{e.dir && <Chevron open={isOpen} />}</span>
              {e.dir ? <Folder name={e.name} open={isOpen} /> : <FileIcon name={e.name} />}
              <span className="label">{e.name}</span>
            </button>
            {e.dir && isOpen && (
              <Level
                server={server}
                name={name}
                path={full}
                depth={depth + 1}
                open={open}
                toggle={toggle}
                onOpen={onOpen}
              />
            )}
          </div>
        );
      })}
      {/* An open folder with nothing under it -- the tree's own glyph, which
          says "this is a directory and it is empty" where the word `empty`
          said only the second half.

          At the root it gets the pane treatment instead, because the two cases
          are different claims: an empty directory somewhere down the tree is
          one row among many and belongs on that row, and an empty *repository*
          is the whole pane having nothing in it -- where a 14-pixel glyph in
          the top-left corner looks like something failed to draw. */}
      {entries.length === 0 &&
        (depth === 0 ? (
          <Empty icon={NoFiles} />
        ) : (
          <div style={{ paddingLeft: depth * 12 + 8 }}>
            <Empty size="row" icon={NoFiles} />
          </div>
        ))}
    </>
  );
}
