import React from "react";
import ReactDOM from "react-dom/client";
import { LucideProvider } from "lucide-react";

import App from "./App";
import { MenuProvider } from "./ContextMenu";
import { Downloads } from "./Download";
import { routeLinks } from "./open";
import { markUntrustedMetrics } from "./charSize";
import { ICON_SIZE, ICON_STROKE } from "./icons";
import { watchForUpdates } from "./Update";
import { installTooltips } from "./tips";
import { readPrefs } from "./prefs";
import { applyAppearance, applyTerminalsDark } from "./theme";
import { applyCustomTheme, applyDiffs, applyFonts, applyZoom } from "./look";
import { applyKeys } from "./keys";
// The window's face, bundled so it is the same one on every platform. See
// `--font-ui` in style.css.
import "@fontsource-variable/inter";
import "./style.css";
// After the stylesheet: see the note at the top of tailwind.css.
import "./tailwind.css";

// Before the first render: the correction is a stylesheet rule keyed on a class
// on <html>, and adding it afterwards would paint the window wrong once.
// On macOS the window has no title bar of its own: the toolbar sits where it
// was, with the traffic lights inset into it, and the window is transparent so
// the sidebar and toolbar can show the system's material through them. See
// `tauri.macos.conf.json`. Elsewhere the window keeps its native frame and
// paints every surface itself.
if (navigator.userAgent.includes("Mac")) document.documentElement.classList.add("mac");
// The palette, for the same reason: chosen before anything is drawn, or a light
// window opens dark for a frame. See `theme.ts`.
// A theme taken in from VS Code is in its own light or dark.
const initial = readPrefs();
applyAppearance(initial.theme?.scheme ?? initial.appearance);
applyTerminalsDark(initial.terminalsDark);
applyCustomTheme(initial.theme);
applyFonts({ ui: initial.uiFont, mono: initial.monoFont, codeSize: initial.codeSize });
if (initial.zoom !== 1) applyZoom(initial.zoom);
applyKeys(initial.keys);
applyDiffs({ stacked: initial.diffStacked, hideWhitespace: initial.diffHideWhitespace });

markUntrustedMetrics();
routeLinks();
watchForUpdates();
// Every `title` drawn as T3 Code's tooltip: see `tips.ts`.
installTooltips();

// The webview takes dropped files itself (`dragDropEnabled: false`), so a chat
// can read them. A file dropped anywhere that does not take one would open it
// in place of the window, so here it goes nowhere instead. A pane that does
// take files handles the drop before it bubbles this far.
for (const type of ["dragover", "drop"] as const) {
  window.addEventListener(type, (e) => {
    if (e.dataTransfer?.types.includes("Files")) e.preventDefault();
  });
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {/* The one place the icon grid is set. lucide's defaults are a 24-pixel
        icon with a 2-pixel stroke, drawn for interfaces built at a larger
        scale than this one; every icon in the window inherits the numbers in
        `icons.tsx` from here instead. `absoluteStrokeWidth` is what makes the
        stroke a pixel count rather than a ratio, so the handful of icons that
        pass their own `size` stay the same weight as the rest. */}
    <LucideProvider size={ICON_SIZE} strokeWidth={ICON_STROKE} absoluteStrokeWidth>
      <MenuProvider>
        <App />
        <Downloads />
      </MenuProvider>
    </LucideProvider>
  </React.StrictMode>,
);
