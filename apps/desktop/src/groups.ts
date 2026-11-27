// Groups of related worktrees within a project: a feature split across
// several, or a batch of tickets worked together. Named, foldable, and only a
// way of arranging the sidebar, so they are this window's, kept as the triage
// is (`triage.ts`): per server in `localStorage`, a project's groups in the
// order they were made, and each worktree's group by its name.
//
// A worktree whose group has gone, or that has gone itself, is simply not in
// one: nothing has to be tidied for the sidebar to be right.

import { useCallback, useEffect, useState } from "react";

const KEY = "hura.groups";

export type WorktreeGroup = { id: string; name: string };

type OnServer = {
  /// By project name.
  groups: Record<string, WorktreeGroup[]>;
  /// Each worktree's group, by session name.
  of: Record<string, string>;
};

type Store = Record<string, OnServer>;

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
    // The groups last as long as the window.
  }
  window.dispatchEvent(new Event(KEY));
}

const empty = (): OnServer => ({ groups: {}, of: {} });

export function useGroups(server: string | null) {
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
  const mine = (server && store[server]) || empty();

  /// Change this server's, from what is on disk now.
  const change = useCallback(
    (f: (s: OnServer) => void) => {
      if (!server) return;
      const all = read();
      const s: OnServer = { groups: { ...(all[server]?.groups ?? {}) }, of: { ...(all[server]?.of ?? {}) } };
      f(s);
      write({ ...all, [server]: s });
    },
    [server],
  );

  const groupsOf = (project: string): WorktreeGroup[] => mine.groups[project] ?? [];
  /// The group a worktree is in, when it is in one of its project's.
  const groupOf = (session: string, project: string | null | undefined): string | null => {
    const id = mine.of[session];
    return id && project && groupsOf(project).some((g) => g.id === id) ? id : null;
  };

  const create = useCallback(
    (project: string, name: string, sessions: string[] = []): string => {
      const id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
      change((s) => {
        s.groups[project] = [...(s.groups[project] ?? []), { id, name }];
        for (const n of sessions) s.of[n] = id;
      });
      return id;
    },
    [change],
  );
  const rename = useCallback(
    (project: string, id: string, name: string) =>
      change((s) => {
        s.groups[project] = (s.groups[project] ?? []).map((g) => (g.id === id ? { ...g, name } : g));
      }),
    [change],
  );
  /// The group goes; its worktrees stay, back among the project's own.
  const remove = useCallback(
    (project: string, id: string) =>
      change((s) => {
        s.groups[project] = (s.groups[project] ?? []).filter((g) => g.id !== id);
        for (const [n, g] of Object.entries(s.of)) if (g === id) delete s.of[n];
      }),
    [change],
  );
  const assign = useCallback(
    (session: string, id: string | null) =>
      change((s) => {
        if (id) s.of[session] = id;
        else delete s.of[session];
      }),
    [change],
  );

  return { groupsOf, groupOf, create, rename, remove, assign };
}

export type Groups = ReturnType<typeof useGroups>;
