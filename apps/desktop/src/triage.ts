// Keeping the sidebar to what needs you, T3 Code's way: a worktree can be
// pinned to the top, settled into a shelf at the bottom once its work is
// done, or snoozed until a time, when it comes back by itself.
//
// Kept per server in this window's `localStorage`, by session name. A session
// that is waiting on you is never kept out of sight by any of it: a settled or
// snoozed worktree that asks something comes back to its project.

import { useCallback, useEffect, useState } from "react";

const KEY = "hura.triage";

export type Triage = {
  pinned?: boolean;
  /// When it was settled.
  settled?: number;
  /// When it comes back.
  snoozedUntil?: number;
};

type Store = Record<string, Record<string, Triage>>;

function read(): Store {
  try {
    const raw: unknown = JSON.parse(window.localStorage.getItem(KEY) ?? "{}");
    return typeof raw === "object" && raw !== null ? (raw as Store) : {};
  } catch {
    return {};
  }
}

/// The triage of a server's worktrees, and changing one. Settling unpins and
/// wakes it; snoozing unsettles it; pinning wakes and unsettles it.
export function useTriage(server: string | null) {
  const [store, setStore] = useState<Store>(read);
  useEffect(() => {
    try {
      window.localStorage.setItem(KEY, JSON.stringify(store));
    } catch {
      // The window works; it just will not remember.
    }
  }, [store]);
  const of = (server && store[server]) || {};
  const change = useCallback(
    (name: string, to: Triage) => {
      if (!server) return;
      setStore((all) => {
        const mine = { ...(all[server] ?? {}) };
        const next: Triage = { ...(mine[name] ?? {}), ...to };
        if (to.settled) {
          delete next.pinned;
          delete next.snoozedUntil;
        }
        if (to.snoozedUntil) delete next.settled;
        if (to.pinned) {
          delete next.settled;
          delete next.snoozedUntil;
        }
        for (const k of Object.keys(next) as (keyof Triage)[]) if (!next[k]) delete next[k];
        if (Object.keys(next).length) mine[name] = next;
        else delete mine[name];
        return { ...all, [server]: mine };
      });
    },
    [server],
  );
  return [of, change] as const;
}

/// The times a worktree can be snoozed until, from now: an hour, this evening
/// (six o'clock, while it is still before five), tomorrow morning and next
/// Monday morning (nine).
export function snoozeChoices(now = new Date()): { label: string; until: number }[] {
  const at = (days: number, hour: number) => {
    const d = new Date(now);
    d.setDate(d.getDate() + days);
    d.setHours(hour, 0, 0, 0);
    return d.getTime();
  };
  const monday = (8 - now.getDay()) % 7 || 7;
  return [
    { label: "For an Hour", until: now.getTime() + 3600_000 },
    ...(now.getHours() < 17 ? [{ label: "Until This Evening", until: at(0, 18) }] : []),
    { label: "Until Tomorrow Morning", until: at(1, 9) },
    { label: "Until Next Week", until: at(monday, 9) },
  ];
}
