// The window's shortcuts, and changing them: T3 Code's keybindings, in one
// place. Each command has a default; a binding changed in Settings is kept in
// the window's prefs and handed here (`applyKeys`), and every handler asks
// `pressed` rather than comparing keys itself, so a change takes everywhere at
// once.
//
// A binding is written as VS Code writes one, `Mod+Shift+F`: `Mod` is Command
// on a Mac and Ctrl elsewhere, so one default serves both, and `Ctrl` is Ctrl
// on both (the terminal's is, as VS Code's is). Empty is no binding.
//
// Most of the defaults have Shift because the plain Ctrl letters are
// readline's in a terminal: Ctrl+F forward a character, Ctrl+B back, Ctrl+T
// transpose.

import { useEffect, useState } from "react";

export const MAC = navigator.platform.toLowerCase().includes("mac");

export const COMMANDS = {
  palette: { label: "Command Palette", combo: "Mod+K" },
  search: { label: "Search Conversations", combo: "Mod+Shift+F" },
  tickets: { label: "Tickets and Pull Requests", combo: "Mod+Shift+T" },
  sidebar: { label: "Show or Hide the Sidebar", combo: "" },
  panel: { label: "Show or Hide the Panel", combo: "Mod+Shift+B" },
  terminal: { label: "Show or Hide the Terminal", combo: "Ctrl+`" },
  newWorktree: { label: "New Worktree", combo: "" },
  runScript: { label: "Run the Project's First Script", combo: "" },
  stash: { label: "Put the Prompt Aside", combo: "Mod+S" },
} as const;

export type Command = keyof typeof COMMANDS;

let bindings: Partial<Record<Command, string>> = {};
const listeners = new Set<() => void>();

/// The bindings changed from their defaults, from the prefs.
export function applyKeys(changed: Record<string, string>): void {
  const next: Partial<Record<Command, string>> = {};
  for (const [id, combo] of Object.entries(changed)) if (id in COMMANDS) next[id as Command] = combo;
  if (JSON.stringify(next) === JSON.stringify(bindings)) return;
  bindings = next;
  for (const listener of listeners) listener();
}

export function comboOf(command: Command): string {
  return bindings[command] ?? COMMANDS[command].combo;
}

type Parsed = { mod: boolean; ctrl: boolean; meta: boolean; shift: boolean; alt: boolean; key: string };

function parse(combo: string): Parsed | null {
  if (!combo) return null;
  const parts = combo.split("+");
  // `Ctrl++` binds the plus key: the last part is the key, even when empty.
  const key = parts.pop() || (combo.endsWith("+") ? "+" : "");
  const has = (m: string) => parts.some((p) => p.toLowerCase() === m);
  return { mod: has("mod"), ctrl: has("ctrl"), meta: has("meta") || has("cmd"), shift: has("shift"), alt: has("alt"), key: key.toLowerCase() };
}

/// Whether `e` is the binding of `command`.
export function pressed(e: KeyboardEvent, command: Command): boolean {
  const p = parse(comboOf(command));
  if (!p) return false;
  const ctrl = p.ctrl || (p.mod && !MAC);
  const meta = p.meta || (p.mod && MAC);
  return (
    e.ctrlKey === ctrl &&
    e.metaKey === meta &&
    e.shiftKey === p.shift &&
    e.altKey === p.alt &&
    e.key.toLowerCase() === p.key
  );
}

/// A key press as a binding, for the editor that records one: null for a
/// modifier on its own, which is a binding still being typed.
export function comboFrom(e: KeyboardEvent): string | null {
  if (["Control", "Meta", "Shift", "Alt", "AltGraph", "CapsLock", "Dead"].includes(e.key)) return null;
  const parts: string[] = [];
  // Command on a Mac and Ctrl elsewhere are written as `Mod`, so a binding
  // made on one works on the other.
  if (MAC ? e.metaKey : e.ctrlKey) parts.push("Mod");
  if (MAC && e.ctrlKey) parts.push("Ctrl");
  if (!MAC && e.metaKey) parts.push("Meta");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  const key = e.key === " " ? "Space" : e.key.length === 1 ? e.key.toUpperCase() : e.key;
  return [...parts, key].join("+");
}

/// A binding as this platform writes one: `⌘⇧F` on a Mac, `Ctrl+Shift+F`
/// elsewhere. Empty for none.
export function keysLabel(combo: string): string {
  const p = parse(combo);
  if (!p) return "";
  const key = p.key === "space" ? "Space" : p.key.length === 1 ? p.key.toUpperCase() : p.key[0].toUpperCase() + p.key.slice(1);
  if (MAC) {
    return `${p.ctrl ? "⌃" : ""}${p.alt ? "⌥" : ""}${p.shift ? "⇧" : ""}${p.mod || p.meta ? "⌘" : ""}${key}`;
  }
  return [p.mod || p.ctrl ? "Ctrl" : "", p.meta ? "Meta" : "", p.alt ? "Alt" : "", p.shift ? "Shift" : "", key]
    .filter(Boolean)
    .join("+");
}

/// The label and the `aria-keyshortcuts` of a command's binding, as the
/// header's and the palette's buttons show them.
export function keysOf(command: Command): { label: string; aria: string } {
  const combo = comboOf(command);
  const p = parse(combo);
  if (!p) return { label: "", aria: "" };
  const aria = [
    p.ctrl || (p.mod && !MAC) ? "Control" : "",
    p.meta || (p.mod && MAC) ? "Meta" : "",
    p.alt ? "Alt" : "",
    p.shift ? "Shift" : "",
    p.key.length === 1 ? p.key.toUpperCase() : p.key,
  ]
    .filter(Boolean)
    .join("+");
  return { label: keysLabel(combo), aria };
}

/// Re-render when a binding changes, for what shows one.
export function useKeys(): number {
  const [version, setVersion] = useState(0);
  useEffect(() => {
    const changed = () => setVersion((v) => v + 1);
    listeners.add(changed);
    return () => {
      listeners.delete(changed);
    };
  }, []);
  return version;
}
