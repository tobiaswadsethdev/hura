// The previews this window is running, for everything that shows them.
//
// Two places do: the ports pane, which starts and stops them, and the tree,
// whose row for a session lights its globe while one of its ports is being
// previewed. One store so the two cannot disagree -- the same reason the
// updater's state is one store. The list lives on the Rust side, which owns
// the listeners; this is the last answer it gave.

import { useSyncExternalStore } from "react";

import { api, type Preview } from "./api";

let current: Preview[] = [];
const listeners = new Set<() => void>();

export function usePreviews(): Preview[] {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => current,
  );
}

/// Ask the Rust side again. After anything that starts or stops one, and on
/// a timer while a pane is watching connection counts.
export async function refreshPreviews(server: string): Promise<Preview[]> {
  const next = await api.previews(server);
  if (JSON.stringify(next) !== JSON.stringify(current)) {
    current = next;
    listeners.forEach((l) => l());
  }
  return next;
}
