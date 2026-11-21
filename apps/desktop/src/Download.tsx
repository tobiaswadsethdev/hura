// Taking a file out of a sandbox: a screenshot the agent took, a report it
// wrote. Into the Downloads folder, shown there in the file manager -- the way
// a browser does it, so there is no dialog to answer for every file.
//
// The agent puts these wherever its tools chose, `/tmp` as often as the
// working copy, and says so in a tool call or in its reply. So besides the
// file tree, a path the agent named is enough: `pathsIn` finds the absolute
// ones in what it said, and the chat offers each in a menu. A folder comes
// out as a zip of it, made by the server.
//
// One line at the foot of the window says how it went, since a download of a
// few megabytes from a sandbox on another machine takes long enough to wonder
// whether anything happened, and a failure from a menu has nowhere else to go.

import { createContext, useEffect, useState } from "react";

import { api, messageOf } from "./api";
import { Download as DownloadIcon } from "./icons";
import type { MenuItem } from "./ContextMenu";

type Status = { id: number; file: string; state: "saving" | "saved" | "failed"; said: string };

let status: Status | null = null;
let next = 0;
const listeners = new Set<(s: Status | null) => void>();
let clear: ReturnType<typeof setTimeout> | undefined;

function set(s: Status | null) {
  status = s;
  clearTimeout(clear);
  // A failure stays until clicked away; saying it once for four seconds is
  // how it gets missed.
  if (s?.state === "saved") clear = setTimeout(() => set(null), 4000);
  listeners.forEach((l) => l(s));
}

export function baseName(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || path;
}

/// Fetch `path` out of `name`'s sandbox, saying how it went at the foot of
/// the window.
export async function download(server: string, name: string, path: string): Promise<void> {
  const id = next++;
  const file = baseName(path);
  set({ id, file, state: "saving", said: `Downloading ${file}…` });
  try {
    const saved = await api.download(server, name, path);
    if (status?.id === id) set({ id, file, state: "saved", said: `Saved ${baseName(saved)} to Downloads` });
  } catch (e) {
    if (status?.id === id) set({ id, file, state: "failed", said: `Could not download ${file}: ${messageOf(e)}` });
  }
}

export function Downloads() {
  const [s, setS] = useState(status);
  useEffect(() => {
    listeners.add(setS);
    return () => void listeners.delete(setS);
  }, []);
  if (!s) return null;
  return (
    <button
      className={`download-status ${s.state}`}
      role="status"
      title="Dismiss"
      onClick={() => set(null)}
    >
      <DownloadIcon aria-hidden />
      <span>{s.said}</span>
    </button>
  );
}

/// Downloads a path out of the sandbox a conversation is about. Absent where
/// there is none, and then nothing offers to.
export const Downloader = createContext<((path: string) => void) | null>(null);

/// The menu items that download each path, or none.
export function downloadItems(paths: string[], fetch: ((path: string) => void) | null): MenuItem[] {
  if (!fetch) return [];
  return paths.map((p) => ({ label: `Download ${baseName(p)}`, icon: DownloadIcon, hint: p, run: () => fetch(p) }));
}

/// The absolute paths to files in some text, in the order they first appear.
///
/// A file is told by an extension: `/usr/bin` named in passing in a command's
/// output is not something anyone wants saved. Some false positives are the
/// price -- `/api/v1.2` -- and cost one menu item.
export function pathsIn(text: string): string[] {
  const found = new Set<string>();
  for (const m of text.matchAll(/(?:^|[\s"'`(=:[])((?:\/[\w.@+~-]+)+\.[A-Za-z0-9]{1,8})(?=\.?(?:$|[\s"'`),;:\]]))/g)) {
    if (!m[1].includes("/..")) found.add(m[1]);
  }
  return [...found].slice(0, 8);
}

/// Whether some code is nothing but an absolute path, a file's or a folder's.
/// Looser than `pathsIn`, since it is asked only of what the agent set apart
/// as code, which is where it writes the folder it put the screenshots in.
export function isPath(text: string): boolean {
  return /^(\/[\w.@+~-]+)+\/?$/.test(text) && !text.includes("/..");
}
