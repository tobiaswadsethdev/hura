// A project's scripts, T3 Code's project actions: the commands a project is
// worked with (its dev server, its tests, its linter), a button away in the
// header over the conversation, each run in a shell of its own in the
// worktree's sandbox, in the drawer under the conversation.
//
// Kept per server and project in this window's `localStorage`, as the triage
// is (see `triage.ts`): they are how this window works with a project, and a
// project's own config is the repository's to say. Every control in the
// window showing a project's scripts sees a change to them at once.

import { useCallback, useEffect, useState } from "react";

const KEY = "hura.scripts";

/// What a script's button wears, T3 Code's set.
export const SCRIPT_ICONS = ["play", "test", "build", "lint", "debug", "configure"] as const;
export type ScriptIcon = (typeof SCRIPT_ICONS)[number];

export type Script = { id: string; name: string; command: string; icon: ScriptIcon };

type Store = Record<string, Record<string, Script[]>>;

function read(): Store {
  try {
    const raw: unknown = JSON.parse(window.localStorage.getItem(KEY) ?? "{}");
    return typeof raw === "object" && raw !== null ? (raw as Store) : {};
  } catch {
    return {};
  }
}

function write(store: Store) {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(store));
  } catch {
    // The scripts last as long as the window.
  }
  window.dispatchEvent(new Event(KEY));
}

export const newScriptId = () => `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;

/// A project's scripts on a server, in the order they were put in (the first
/// is the header's button), and replacing them.
export function useScripts(server: string | null, project: string | null) {
  const [store, setStore] = useState<Store>(read);
  useEffect(() => {
    const changed = () => setStore(read());
    window.addEventListener(KEY, changed);
    window.addEventListener("storage", changed);
    return () => {
      window.removeEventListener(KEY, changed);
      window.removeEventListener("storage", changed);
    };
  }, []);
  const list = (server && project && store[server]?.[project]) || [];
  const save = useCallback(
    (next: Script[]) => {
      if (!server || !project) return;
      const all = read();
      const mine = { ...(all[server] ?? {}) };
      const kept = next.filter((s) => s.name.trim() && s.command.trim());
      if (kept.length) mine[project] = kept;
      else delete mine[project];
      write({ ...all, [server]: mine });
    },
    [server, project],
  );
  return [list, save] as const;
}
