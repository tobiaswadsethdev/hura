import React from "react";
import ReactDOM from "react-dom/client";
import { LucideProvider } from "lucide-react";

import App from "./App";
import { MenuProvider } from "./ContextMenu";
import { routeLinks } from "./open";
import { markUntrustedMetrics } from "./charSize";
import { ICON_SIZE, ICON_STROKE } from "./icons";
import { watchForUpdates } from "./Update";
import "./style.css";

// Before the first render: the correction is a stylesheet rule keyed on a class
// on <html>, and adding it afterwards would paint the window wrong once.
// On macOS the window has no title bar of its own: the toolbar sits where it
// was, with the traffic lights inset into it, and the window is transparent so
// the sidebar and toolbar can show the system's material through them. See
// `tauri.macos.conf.json`. Elsewhere the window keeps its native frame and
// paints every surface itself.
if (navigator.userAgent.includes("Mac")) document.documentElement.classList.add("mac");

markUntrustedMetrics();
routeLinks();
watchForUpdates();

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
      </MenuProvider>
    </LucideProvider>
  </React.StrictMode>,
);
