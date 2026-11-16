// Light, dark, or whichever the system is in: the window's own setting, as the
// iOS app has it (Account -> Settings there, Settings -> This window here).
//
// Two halves, and both are needed. The stylesheet is chosen by an attribute on
// <html> -- see the two palettes at the top of `style.css`. The *window* is
// told too, because on macOS the sidebar and toolbar are the system's own
// material showing through a transparent window, and that material, the
// traffic lights and every native menu follow the window's appearance rather
// than anything this page says. A light page over a dark material is a light
// window with a dark sidebar.
//
// "System" is the window following the OS, which is `setTheme(null)`; the page
// then reads the result back from `prefers-color-scheme`, which the webview
// takes from the window. So going back to System from a forced choice is two
// steps -- the window lets go, the media query changes, and the listener below
// picks the palette -- and needs nothing more than the listener being there.

import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type Appearance = "system" | "light" | "dark";
export type Scheme = "light" | "dark";

export const APPEARANCES: { value: Appearance; label: string }[] = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

const dark = window.matchMedia("(prefers-color-scheme: dark)");
let chosen: Appearance = "system";
const listeners = new Set<(scheme: Scheme) => void>();

/// The palette the window is in now: the choice, or the system's under
/// "System".
export function scheme(): Scheme {
  if (chosen !== "system") return chosen;
  return dark.matches ? "dark" : "light";
}

function paint(): void {
  const now = scheme();
  if (document.documentElement.dataset.theme === now) return;
  document.documentElement.dataset.theme = now;
  for (const listener of listeners) listener(now);
}

/// Take a choice. Called once before the first render, so the window does not
/// open in one palette and change to the other, and again whenever it changes.
export function applyAppearance(appearance: Appearance): void {
  chosen = appearance;
  paint();
  if (isTauri()) {
    // Nothing to do if it fails but carry on: the page is already right, and
    // only the material around it is not.
    getCurrentWindow()
      .setTheme(appearance === "system" ? null : appearance)
      .catch(() => {});
  }
}

dark.addEventListener("change", paint);

/// Told when the palette changes, for the panes that cannot read the
/// stylesheet's -- see `palette.ts`.
export function onScheme(listener: (scheme: Scheme) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
