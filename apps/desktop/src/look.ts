// The rest of how the window looks, past light and dark (`theme.ts`): T3 Code's
// appearance settings. A VS Code colour theme taken in as the palette, the
// faces for text and for code, the size code is drawn at, and the size of the
// whole window.
//
// Each is applied here rather than read where it is drawn, the way the
// palette is: the stylesheet takes custom properties set on <html>, and the
// two panes that paint their own (xterm and Monaco, see `palette.ts`) are told
// and redraw.

import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";

import type { Scheme } from "./theme";

/// A VS Code theme, as this window keeps it: already turned into the window's
/// own tokens, so a theme is read once, when it is taken in.
export type CustomTheme = {
  name: string;
  scheme: Scheme;
  /// The window's custom properties, without their `--`: `bg`, `accent`.
  colors: Record<string, string>;
  /// The terminal's sixteen, by xterm's names: `red`, `brightBlue`.
  ansi: Record<string, string>;
};

/// Each of the window's tokens, and the VS Code colours that say it, best
/// first. Only what a theme gives is taken; the rest stays the window's.
const TOKENS: Record<string, string[]> = {
  bg: ["editor.background"],
  chrome: ["sideBar.background", "editor.background"],
  surface: ["editorWidget.background", "sideBar.background"],
  "surface-2": ["input.background", "dropdown.background", "editorWidget.background"],
  sunken: ["terminal.background", "panel.background", "editor.background"],
  text: ["editor.foreground", "foreground"],
  dim: ["descriptionForeground", "foreground"],
  dimmer: ["editorLineNumber.foreground", "disabledForeground"],
  "secondary-label": ["descriptionForeground"],
  line: ["panel.border", "editorGroup.border", "sideBar.border", "widget.border", "contrastBorder"],
  "row-hover": ["list.hoverBackground"],
  "row-active": ["list.activeSelectionBackground"],
  "row-selected": ["list.inactiveSelectionBackground"],
  hover: ["toolbar.hoverBackground", "list.hoverBackground"],
  accent: ["button.background", "focusBorder", "textLink.foreground"],
  "accent-ink": ["button.foreground"],
  working: ["progressBar.background", "terminal.ansiBlue"],
  ok: ["terminal.ansiGreen", "gitDecoration.addedResourceForeground"],
  bad: ["errorForeground", "editorError.foreground", "terminal.ansiRed"],
  warn: ["editorWarning.foreground", "terminal.ansiYellow"],
  attention: ["terminal.ansiYellow", "editorWarning.foreground"],
  stop: ["terminal.ansiRed", "errorForeground"],
};

const ANSI_NAMES = ["Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White"];

const COLOUR = /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;

/// VS Code's theme files are JSON with comments and trailing commas. Both
/// taken out, minding strings, which can hold `//` (a URL in `$schema`).
export function parseJsonc(text: string): unknown {
  let out = "";
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '"') {
      let j = i + 1;
      while (j < text.length && text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      out += text.slice(i, j + 1);
      i = j;
    } else if (c === "/" && text[i + 1] === "/") {
      while (i < text.length && text[i] !== "\n") i++;
      out += "\n";
    } else if (c === "/" && text[i + 1] === "*") {
      i = text.indexOf("*/", i + 2);
      if (i < 0) break;
      i++;
    } else out += c;
  }
  return JSON.parse(out.replace(/,(\s*[}\]])/g, "$1"));
}

/// A theme file, read into the window's tokens. Throws a sentence for a file
/// that is not one.
export function readVsCodeTheme(text: string, fileName = ""): CustomTheme {
  let raw: unknown;
  try {
    raw = parseJsonc(text);
  } catch {
    throw new Error("That is not a theme file: it does not read as JSON.");
  }
  const theme = (typeof raw === "object" && raw !== null ? raw : {}) as Record<string, unknown>;
  const given = (typeof theme.colors === "object" && theme.colors !== null ? theme.colors : {}) as Record<
    string,
    unknown
  >;
  const colour = (key: string) => {
    const v = given[key];
    return typeof v === "string" && COLOUR.test(v.trim()) ? v.trim() : null;
  };
  if (!colour("editor.background")) {
    throw new Error(
      typeof theme.include === "string"
        ? `This theme takes its colours from ${theme.include}: take in that file instead.`
        : "That file has no colours in it: a VS Code theme's are under \"colors\".",
    );
  }
  const colors: Record<string, string> = {};
  for (const [token, keys] of Object.entries(TOKENS)) {
    for (const key of keys) {
      const v = colour(key);
      if (v) {
        colors[token] = v;
        break;
      }
    }
  }
  const ansi: Record<string, string> = {};
  for (const name of ANSI_NAMES) {
    const plain = colour(`terminal.ansi${name}`);
    const bright = colour(`terminal.ansiBright${name}`);
    if (plain) ansi[name.toLowerCase()] = plain;
    if (bright) ansi[`bright${name}`] = bright;
  }
  const type = typeof theme.type === "string" ? theme.type.toLowerCase() : "";
  const scheme: Scheme =
    type === "light" || type === "hclight" ? "light" : type === "dark" || type === "hc" ? "dark" : luminance(colors.bg) > 0.5 ? "light" : "dark";
  const name =
    (typeof theme.name === "string" && theme.name.trim()) ||
    fileName.replace(/\.json$/i, "").replace(/-color-theme$/i, "") ||
    "Imported theme";
  return { name, scheme, colors, ansi };
}

function luminance(hex: string | undefined): number {
  const m = hex?.match(/^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i);
  if (!m) return 0;
  const [r, g, b] = m.slice(1, 4).map((h) => parseInt(h, 16) / 255);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/// What a stored theme has to look like to be used: anything else is a store
/// some other build wrote, and the window's own palette is the safe answer.
export function sanitizeTheme(stored: unknown): CustomTheme | null {
  if (typeof stored !== "object" || stored === null) return null;
  const t = stored as Record<string, unknown>;
  const map = (v: unknown) =>
    Object.fromEntries(
      Object.entries(typeof v === "object" && v !== null ? v : {}).filter(
        (e): e is [string, string] => /^[a-z0-9-]+$/i.test(e[0]) && typeof e[1] === "string" && COLOUR.test(e[1]),
      ),
    );
  if (typeof t.name !== "string" || (t.scheme !== "dark" && t.scheme !== "light")) return null;
  return { name: t.name, scheme: t.scheme, colors: map(t.colors), ansi: map(t.ansi) };
}

let theme: CustomTheme | null = null;
let applied: string[] = [];

/// Take a theme's colours over the window's, or put the window's back with
/// `null`. The light or dark under it is `theme.ts`'s to set: see `App`.
export function applyCustomTheme(next: CustomTheme | null): void {
  theme = next;
  const root = document.documentElement.style;
  for (const token of applied) root.removeProperty(`--${token}`);
  applied = Object.keys(next?.colors ?? {});
  for (const token of applied) root.setProperty(`--${token}`, next!.colors[token]);
  tell();
}

/// The terminal's colours from the theme, when the terminal is in the theme's
/// light or dark: a dark theme's sixteen on a terminal kept dark in a light
/// window would be the wrong sixteen.
export function themeAnsi(scheme: Scheme): Record<string, string> {
  return theme && theme.scheme === scheme ? theme.ansi : {};
}

/// The faces and the code size.
export type Fonts = { ui: string; mono: string; codeSize: number };

/// What is in `style.css` and the panes when nothing is chosen.
const MONO_STACK = 'ui-monospace, "Cascadia Mono", Menlo, Consolas, monospace';
export const CODE_SIZE = { min: 10, max: 20, default: 13 };

let fonts: Fonts = { ui: "", mono: "", codeSize: CODE_SIZE.default };

/// A family as CSS wants it: quoted unless it already is, or is a list.
const family = (name: string) => (/[,"']/.test(name) ? name : `"${name}"`);

export function applyFonts(next: Fonts): void {
  if (next.ui === fonts.ui && next.mono === fonts.mono && next.codeSize === fonts.codeSize) return;
  fonts = next;
  const root = document.documentElement.style;
  // Ahead of the stylesheet's own list, which stays as the fallback, so a face
  // this machine does not have costs nothing.
  if (next.ui.trim()) root.setProperty("--font-ui", `${family(next.ui.trim())}, ${uiStack()}`);
  else root.removeProperty("--font-ui");
  if (next.mono.trim()) root.setProperty("--font-mono", `${family(next.mono.trim())}, ${MONO_STACK}`);
  else root.removeProperty("--font-mono");
  tell();
}

/// The stylesheet's list, read before anything was set over it.
let uiDefault: string | null = null;
function uiStack(): string {
  if (uiDefault === null) {
    const root = document.documentElement.style;
    const mine = root.getPropertyValue("--font-ui");
    root.removeProperty("--font-ui");
    uiDefault = getComputedStyle(document.documentElement).getPropertyValue("--font-ui").trim() || "sans-serif";
    if (mine) root.setProperty("--font-ui", mine);
  }
  return uiDefault;
}

/// What the terminal and the editors draw code in.
export function codeFont(): { family: string; size: number } {
  return {
    family: fonts.mono.trim() ? `${family(fonts.mono.trim())}, ${MONO_STACK}` : MONO_STACK,
    size: fonts.codeSize,
  };
}

/// How diffs are drawn: T3 Code's two diff settings.
let diffs = { stacked: false, hideWhitespace: true };

export function applyDiffs(next: { stacked: boolean; hideWhitespace: boolean }): void {
  if (next.stacked === diffs.stacked && next.hideWhitespace === diffs.hideWhitespace) return;
  diffs = next;
  tell();
}

/// The diff editor's options for them.
export function diffOptions(): { renderSideBySide: boolean; ignoreTrimWhitespace: boolean } {
  return { renderSideBySide: !diffs.stacked, ignoreTrimWhitespace: diffs.hideWhitespace };
}

export const ZOOM = { min: 0.8, max: 1.5, default: 1 };

/// The whole window larger or smaller, as a browser's zoom is: every size in
/// the stylesheet is in pixels, and the webview's scale is what moves them all.
export function applyZoom(scale: number): void {
  if (!isTauri()) return;
  getCurrentWebview()
    .setZoom(scale)
    .catch(() => {});
}

const listeners = new Set<() => void>();
function tell() {
  for (const listener of listeners) listener();
}

/// Told when the theme or the fonts change, for what paints its own.
export function onLook(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
