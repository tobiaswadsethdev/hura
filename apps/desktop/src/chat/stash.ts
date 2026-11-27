// Prompts put aside to come back to, T3 Code's stash: Ctrl+S (Command+S on a
// Mac) in the composer parks what is written and empties the box, and the
// stash over the box gives it back.
//
// One stash for the window rather than one per conversation, as T3 Code's is:
// a prompt is often put aside to be sent somewhere else. In `localStorage`
// beside the drafts (see `drafts.ts`), the newest first, at most `LIMIT`.

import { useCallback, useEffect, useState } from "react";

const KEY = "hura.stash";
const LIMIT = 100;

export type Stashed = { id: string; text: string; at: number };

function read(): Stashed[] {
  try {
    const raw: unknown = JSON.parse(window.localStorage.getItem(KEY) ?? "[]");
    if (!Array.isArray(raw)) return [];
    return raw.filter(
      (s): s is Stashed =>
        typeof s === "object" && s !== null && typeof s.id === "string" && typeof s.text === "string" && typeof s.at === "number",
    );
  } catch {
    return [];
  }
}

function write(all: Stashed[]) {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(all.slice(0, LIMIT)));
  } catch {
    // A full or unavailable store: the stash lasts as long as the window.
  }
  // Every composer in the window shows the same stash.
  window.dispatchEvent(new Event(KEY));
}

/// The stash, and putting into it and taking out of it.
export function useStash() {
  const [all, setAll] = useState<Stashed[]>(read);
  useEffect(() => {
    const changed = () => setAll(read());
    window.addEventListener(KEY, changed);
    window.addEventListener("storage", changed);
    return () => {
      window.removeEventListener(KEY, changed);
      window.removeEventListener("storage", changed);
    };
  }, []);
  const put = useCallback((text: string) => {
    if (!text.trim()) return;
    write([{ id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`, text, at: Date.now() }, ...read()]);
  }, []);
  const take = useCallback((id: string) => write(read().filter((s) => s.id !== id)), []);
  return { all, put, take };
}
