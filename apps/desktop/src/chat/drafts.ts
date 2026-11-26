// What was being written in each conversation and not yet sent.
//
// Kept per conversation, so switching to another worktree and back, closing
// the window, or a reload after an update leaves the half-written message where
// it was. Only the text: an attached file can be megabytes, and a picture
// pasted an hour ago is easy to paste again.
//
// In `localStorage` beside the window's preferences (see `prefs.ts`), under one
// key of its own. A draft not touched for a month is dropped the next time any
// draft is written, and there are never more than `LIMIT`, so a conversation
// that ended without its draft being sent does not keep it forever.

const KEY = "hura.drafts";
const LIMIT = 100;
const MONTH_MS = 30 * 24 * 60 * 60 * 1000;

type Draft = { text: string; at: number };

function readAll(): Record<string, Draft> {
  try {
    const raw: unknown = JSON.parse(window.localStorage.getItem(KEY) ?? "{}");
    if (typeof raw !== "object" || raw === null) return {};
    const out: Record<string, Draft> = {};
    for (const [key, value] of Object.entries(raw)) {
      const d = value as Partial<Draft> | null;
      if (d && typeof d.text === "string" && typeof d.at === "number") out[key] = { text: d.text, at: d.at };
    }
    return out;
  } catch {
    return {};
  }
}

function writeAll(all: Record<string, Draft>) {
  const now = Date.now();
  const kept = Object.entries(all)
    .filter(([, d]) => d.text && now - d.at < MONTH_MS)
    .sort((a, b) => b[1].at - a[1].at)
    .slice(0, LIMIT);
  try {
    window.localStorage.setItem(KEY, JSON.stringify(Object.fromEntries(kept)));
  } catch {
    // A full or unavailable store: the draft lives as long as the pane does.
  }
}

/// The draft kept for a conversation, or an empty string.
export function readDraft(key: string): string {
  return readAll()[key]?.text ?? "";
}

/// Keep the text as the conversation's draft; an empty one forgets it.
export function writeDraft(key: string, text: string) {
  const all = readAll();
  if (text.trim()) all[key] = { text, at: Date.now() };
  else delete all[key];
  writeAll(all);
}
