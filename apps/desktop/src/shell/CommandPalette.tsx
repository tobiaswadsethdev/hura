// The command palette, T3 Code's: one box to go anywhere and do anything,
// opened with Command+K (Ctrl+K elsewhere). What is typed narrows a list in
// groups (worktrees, projects, tickets and pull requests, things to do), the
// arrows move through it, Enter does the one lit, and Esc puts it away.
//
// The items are `App`'s to build, since it holds what they act on; this only
// finds and draws them.

import { type ComponentType, useEffect, useMemo, useRef, useState } from "react";

import { Find } from "../icons";
import { cn } from "../lib/cn";

export type PaletteItem = {
  id: string;
  group: string;
  label: string;
  /// Said after the label, quieter: a branch, a project, a ticket's key.
  hint?: string;
  /// More words it answers to, besides the label and the hint.
  keywords?: string;
  icon?: ComponentType<{ className?: string }>;
  /// A shortcut to show at the end.
  keys?: string;
  run: () => void;
};

/// How well `q` fits: the label starting with it, a word in it starting with
/// it, it anywhere, then its letters in order. Lower is better; null is no.
function score(item: PaletteItem, q: string): number | null {
  if (!q) return 0;
  const label = item.label.toLowerCase();
  const rest = `${item.hint ?? ""} ${item.keywords ?? ""}`.toLowerCase();
  if (label.startsWith(q)) return 0;
  if (label.split(/[\s/._-]+/).some((w) => w.startsWith(q))) return 1;
  if (label.includes(q)) return 2;
  if (rest.includes(q)) return 3;
  let i = 0;
  for (const c of label) if (c === q[i]) i++;
  return i === q.length ? 4 : null;
}

export function CommandPalette({ items, onClose }: { items: PaletteItem[]; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => input.current?.focus(), []);

  // Best first within each group, the groups in the order they were given.
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    const groups = [...new Set(items.map((i) => i.group))];
    return groups.flatMap((g) =>
      items
        .filter((i) => i.group === g)
        .map((i) => ({ i, s: score(i, q) }))
        .filter((x): x is { i: PaletteItem; s: number } => x.s !== null)
        .sort((a, b) => a.s - b.s)
        .slice(0, q ? 8 : 6)
        .map((x) => x.i),
    );
  }, [items, query]);
  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const run = (item: PaletteItem | undefined) => {
    if (!item) return;
    onClose();
    item.run();
  };

  return (
    <div
      data-tw
      className="fixed inset-0 z-[1500] flex items-start justify-center bg-black/40 pt-[14vh]"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-label="Command palette"
        className="flex max-h-[60vh] w-[min(600px,calc(100vw-48px))] flex-col overflow-hidden rounded-2xl bg-popover text-sm shadow-[0_24px_72px_-20px_rgb(0_0_0/0.6)] ring-1 ring-border backdrop-blur-xl"
      >
        <label className="flex h-12 shrink-0 items-center gap-2.5 border-b border-border px-4">
          <Find className="size-4 shrink-0 text-muted-foreground" />
          <input
            ref={input}
            className="h-full min-w-0 flex-1 bg-transparent text-[15px] text-foreground outline-none placeholder:text-muted-foreground"
            placeholder="Go to a worktree, a ticket, or do something…"
            aria-label="Search"
            spellCheck={false}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                const n = shown.length;
                if (n) setActive((a) => (a + (e.key === "ArrowDown" ? 1 : -1) + n) % n);
              } else if (e.key === "Enter") {
                e.preventDefault();
                run(shown[active]);
              } else if (e.key === "Escape") {
                e.preventDefault();
                onClose();
              }
            }}
          />
        </label>
        <div ref={list} className="min-h-0 flex-1 overflow-y-auto p-1.5" role="listbox" aria-label="Results">
          {shown.length === 0 && <p className="px-3 py-6 text-center text-muted-foreground">Nothing fits that.</p>}
          {shown.map((item, index) => (
            <div key={item.id}>
              {(index === 0 || shown[index - 1].group !== item.group) && (
                <div className="px-2.5 pt-2.5 pb-1 text-xs font-medium text-muted-foreground">{item.group}</div>
              )}
              <button
                data-index={index}
                role="option"
                aria-selected={index === active}
                className={cn(
                  "flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-left",
                  index === active ? "bg-accent text-foreground" : "text-foreground/90",
                )}
                onMouseMove={() => index !== active && setActive(index)}
                onClick={() => run(item)}
              >
                {item.icon && <item.icon className="size-4 shrink-0 text-muted-foreground" />}
                <span className="min-w-0 shrink truncate">{item.label}</span>
                {item.hint && <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{item.hint}</span>}
                {!item.hint && <span className="flex-1" />}
                {item.keys && (
                  <kbd className="shrink-0 rounded bg-foreground/7 px-1.5 font-sans text-xs font-medium text-muted-foreground">
                    {item.keys}
                  </kbd>
                )}
              </button>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
