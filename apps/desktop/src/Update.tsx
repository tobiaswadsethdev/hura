// The window noticing that a newer release exists, and offering to take it.
//
// **Windows only, and that is not an oversight.** The release page carries a
// Windows installer and no Linux one, because a Tauri bundle links against the
// webkit2gtk of the distribution that built it -- see docs/install.md. On Linux
// the window is built from the tree, so there is nothing for an updater to
// fetch and this stays out of the way rather than offering something that would
// fail.
//
// **It asks.** The download is verified against a signature the release was
// built with, so the risk is not what arrives; it is *when*. A window watching
// four agents is a window somebody is using, and replacing it out from under
// them mid-session is the same mistake `hurad` refuses to make with its own
// binary. So: a badge in the header, a button on the about screen, and it
// waits.
//
// **It checks more than once.** At launch, whenever the about screen's button
// is pressed, and when the window comes back into focus after a long time
// away. A window left open for a week used to need a restart just to *learn*
// there was something to install; one request every few hours, and only when
// somebody is looking, is not polling github.
//
// The state is one store for the window rather than a component's own,
// because two places read it -- the header badge and the about screen -- and
// they have to agree about which version is on offer.

import { useSyncExternalStore } from "react";

import { Upgrade } from "./icons";

/// Re-checked on focus once this much has passed since the last check.
const STALE_MS = 6 * 60 * 60_000;

/// What was found, once a check has come back with something.
type Found = {
  version: string;
  /// The release's own notes, from `latest.json`. Absent for a release cut
  /// without any.
  notes: string | null;
  date: string | null;
  /// Kept so the install uses the very object the check returned. Re-checking
  /// on click would be a second request with a second answer, and the version
  /// on the button has to be the version that installs.
  install: (onProgress: (got: number, total: number | null) => void) => Promise<void>;
};

export type Phase =
  | { at: "unsupported" }
  | { at: "idle" }
  | { at: "checking" }
  | { at: "current"; checkedAt: number }
  | { at: "found"; found: Found; checkedAt: number }
  | { at: "installing"; version: string; got: number; total: number | null }
  | { at: "failed"; version: string | null; why: string; checkedAt: number };

/// Whether this build has an updater behind it at all.
///
/// `navigator.userAgent` rather than a Tauri call, because the answer decides
/// whether to *make* the call: the plugin is compiled in on Windows only, and
/// asking a no-op plugin for an update is an error to handle rather than a
/// question with an answer.
function onWindows(): boolean {
  return /windows/i.test(navigator.userAgent);
}

let phase: Phase = onWindows() ? { at: "idle" } : { at: "unsupported" };
const listeners = new Set<() => void>();

function set(next: Phase) {
  phase = next;
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useUpdate(): Phase {
  return useSyncExternalStore(subscribe, () => phase);
}

/// Ask github whether there is something newer. Safe to call at any time:
/// it does nothing while a check or an install is already under way.
export async function checkForUpdate(): Promise<void> {
  if (phase.at === "unsupported" || phase.at === "checking" || phase.at === "installing") return;
  set({ at: "checking" });
  try {
    // Imported here rather than at the top of the file so a Linux build
    // never loads the plugin's JS at all -- and so a browser opening the
    // dev server, which has no Tauri host, does not throw on import.
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    const checkedAt = Date.now();
    if (!update) {
      set({ at: "current", checkedAt });
      return;
    }
    set({
      at: "found",
      checkedAt,
      found: {
        version: update.version,
        notes: update.body?.trim() || null,
        date: update.date ?? null,
        install: (onProgress) => {
          let got = 0;
          let total: number | null = null;
          return update.downloadAndInstall((event) => {
            if (event.event === "Started") total = event.data.contentLength ?? null;
            if (event.event === "Progress") got += event.data.chunkLength;
            onProgress(got, total);
          });
        },
      },
    });
  } catch (e) {
    set({ at: "failed", version: null, why: messageOf(e), checkedAt: Date.now() });
  }
}

/// Download, verify, install, relaunch. Only from a button press.
export async function installUpdate(): Promise<void> {
  if (phase.at !== "found") return;
  const { found, checkedAt } = phase;
  set({ at: "installing", version: found.version, got: 0, total: null });
  try {
    await found.install((got, total) =>
      set({ at: "installing", version: found.version, got, total }),
    );
    // The installer replaced the files; the process still running is the old
    // one, so it has to go. `relaunch` is the process plugin rather than
    // `window.location.reload()`, which would reload the web view and leave
    // the same binary behind it.
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  } catch (e) {
    set({ at: "failed", version: found.version, why: messageOf(e), checkedAt });
  }
}

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/// The checks nobody asks for: one at launch, and one on focus once the last
/// is stale. Called once, from `main.tsx`.
export function watchForUpdates() {
  if (phase.at === "unsupported") return;
  void checkForUpdate();
  window.addEventListener("focus", () => {
    const last = "checkedAt" in phase ? phase.checkedAt : 0;
    if (Date.now() - last > STALE_MS) void checkForUpdate();
  });
}

/// The header's half: nothing until there is something to install, then the
/// version on offer, which opens the about screen where the button is.
///
/// Silent about a failed *check* -- "could not reach github" is not worth a
/// mark in the header, and the about screen says it where somebody asked. A
/// failed *install* is shown, because somebody pressed a button and is owed
/// an answer.
export function UpdateBadge({ onOpen }: { onOpen: () => void }) {
  const p = useUpdate();
  if (p.at === "found") {
    return (
      <button className="update-badge" title={`hura ${p.found.version} is available`} onClick={onOpen}>
        <Upgrade />
        {p.found.version}
      </button>
    );
  }
  if (p.at === "installing") {
    return (
      <button className="update-badge" title="installing" onClick={onOpen}>
        <Upgrade />
        {percent(p.got, p.total) ?? "…"}
      </button>
    );
  }
  if (p.at === "failed" && p.version) {
    return (
      <button className="update-badge bad" title={`could not install ${p.version}`} onClick={onOpen}>
        <Upgrade />
        {p.version}
      </button>
    );
  }
  return null;
}

export function percent(got: number, total: number | null): string | null {
  return total ? `${Math.min(100, Math.round((got / total) * 100))}%` : null;
}
