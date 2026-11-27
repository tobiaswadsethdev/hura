// Settings as T3 Code has them (its `AppRouteModal` and `SettingsSidebarNav`):
// a large card floating over the window, the window blurred behind it, its
// pages listed down the left in groups with a search over them, and a header
// with where you are and the way out. Esc, the close button or a press on the
// window behind it closes it.
//
// Every screen that was a setting lives here now: the window's own settings,
// the server's defaults for a new worktree, the integrations, the servers, the
// trackers, the account and About. Those that were whole screens before are
// drawn without their header inside it (see `Screen.tsx`).

import {
  type ComponentType,
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

import {
  About,
  Account,
  Close,
  Find,
  Integrations,
  Servers,
  SettingsAppearance,
  SettingsDiffs,
  SettingsGeneral,
  SettingsKeys,
  SettingsProjects,
  SettingsSessions,
  Tracker,
  UsageGlyph,
} from "../icons";
import { cn } from "../lib/cn";
import { Button } from "../ui/button";

export type SettingsPageId =
  | "general"
  | "appearance"
  | "diffs"
  | "keybindings"
  | "sessions"
  | "projects"
  | "usage"
  | "integrations"
  | "servers"
  | "trackers"
  | "account"
  | "about";

export const SETTINGS_LABELS: Record<SettingsPageId, string> = {
  general: "General",
  appearance: "Appearance",
  diffs: "Diffs",
  keybindings: "Keybindings",
  sessions: "New Worktrees",
  projects: "Projects",
  usage: "Usage",
  integrations: "Integrations",
  servers: "Servers",
  trackers: "Trackers",
  account: "Account",
  about: "About",
};

const ICONS: Record<SettingsPageId, ComponentType<{ className?: string }>> = {
  general: SettingsGeneral,
  appearance: SettingsAppearance,
  diffs: SettingsDiffs,
  keybindings: SettingsKeys,
  sessions: SettingsSessions,
  projects: SettingsProjects,
  usage: UsageGlyph,
  integrations: Integrations,
  servers: Servers,
  trackers: Tracker,
  account: Account,
  about: About,
};

/// T3 Code's groups: the window's own first and unheaded, then what is the
/// server's, then what connects it to the rest, and About last.
const NAV: { heading: string; hidden?: boolean; pages: SettingsPageId[] }[] = [
  { heading: "Preferences", hidden: true, pages: ["general", "appearance", "diffs", "keybindings"] },
  { heading: "Server", pages: ["sessions", "projects", "integrations"] },
  { heading: "Connections", pages: ["servers", "trackers", "account"] },
  { heading: "More", hidden: true, pages: ["about"] },
];

/// What the search finds besides a page's name: the settings on it, by the
/// words somebody would look for them with.
const FINDS: { page: SettingsPageId; title: string; terms: string }[] = [
  { page: "general", title: "When an agent needs me", terms: "notify notifications alert waiting" },
  { page: "general", title: "When my tickets or pull requests change", terms: "notify notifications jira github" },
  { page: "general", title: "Send review comments to their agents", terms: "pull request review comments" },
  { page: "appearance", title: "Mode", terms: "theme light dark system appearance" },
  { page: "appearance", title: "VS Code theme", terms: "import colors palette custom" },
  { page: "appearance", title: "Keep terminals dark", terms: "terminal dark light" },
  { page: "appearance", title: "Interface font", terms: "font face typography" },
  { page: "appearance", title: "Code font", terms: "monospace font terminal editor" },
  { page: "appearance", title: "Code size", terms: "font size terminal editor" },
  { page: "appearance", title: "Interface size", terms: "zoom scale" },
  { page: "diffs", title: "Layout", terms: "diff side by side stacked inline" },
  { page: "diffs", title: "Hide whitespace-only changes", terms: "diff whitespace" },
  { page: "keybindings", title: "Shortcuts", terms: "keys hotkeys bindings keyboard" },
  { page: "projects", title: "Project defaults", terms: "policy toolchains base branch agent sandbox worktree" },
  { page: "sessions", title: "Branch prefix", terms: "git branch name" },
  { page: "sessions", title: "Agent", terms: "chat terminal claude" },
  { page: "sessions", title: "Base branch", terms: "git main" },
  { page: "sessions", title: "Policy", terms: "sandbox network rules" },
  { page: "sessions", title: "Credentials", terms: "providers secrets" },
  { page: "sessions", title: "Named", terms: "names title slug haiku" },
  { page: "sessions", title: "API key", terms: "anthropic naming" },
  { page: "sessions", title: "Run at once", terms: "queue limit" },
  { page: "sessions", title: "Auto-update", terms: "hurad update server" },
  { page: "integrations", title: "MCP servers, skills and secrets", terms: "mcp skills secrets tools" },
  { page: "servers", title: "Pair a server", terms: "hurad connect pair address" },
  { page: "trackers", title: "Trackers", terms: "jira github azure devops tickets" },
  { page: "account", title: "hura cloud", terms: "sign in login account" },
  { page: "about", title: "Version and updates", terms: "version update release" },
];

/// Where a page puts what it does that is not one setting, such as saving:
/// the header, left of the close button, as T3 Code's "Restore defaults" is.
const HeaderSlot = createContext<HTMLElement | null>(null);

export function SettingsActions({ children }: { children: ReactNode }) {
  const slot = useContext(HeaderSlot);
  return slot ? createPortal(children, slot) : null;
}

/// Whether what renders is inside the settings: a screen there drops its own
/// header and is drawn as a page (`Screen.tsx`).
export const InSettings = createContext(false);

export function SettingsModal({
  page,
  onPage,
  onClose,
  children,
}: {
  page: SettingsPageId;
  onPage: (page: SettingsPageId) => void;
  onClose: () => void;
  /// The page.
  children: ReactNode;
}) {
  // In with T3 Code's 150 ms scale and fade, and out the same way before the
  // window lets go of it.
  const [shown, setShown] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const popup = useRef<HTMLDivElement>(null);
  const [slot, setSlot] = useState<HTMLElement | null>(null);
  const [opener] = useState(() => (document.activeElement instanceof HTMLElement ? document.activeElement : null));
  const byKeyboard = useRef(false);
  const pressedBehind = useRef(false);
  const body = useRef<HTMLDivElement>(null);
  // A setting the search went to: scrolled to and lit once its page is drawn,
  // as T3 Code's search does.
  const [found, setFound] = useState<{ title: string; at: number } | null>(null);
  // Watched for rather than looked up once: a page that asks the server
  // (New Worktrees) draws its rows when the answer comes.
  useEffect(() => {
    const root = body.current;
    if (!found || !root) return;
    const look = () => {
      const row = [...root.querySelectorAll<HTMLElement>('[data-slot="settings-row"], section')].find(
        (el) => el.querySelector("h2, h3")?.textContent?.trim() === found.title,
      );
      if (!row) return false;
      row.scrollIntoView({ block: "center" });
      row.dataset.found = "";
      window.setTimeout(() => delete row.dataset.found, 1600);
      return true;
    };
    if (look()) return;
    const watch = new MutationObserver(() => look() && stop());
    watch.observe(root, { childList: true, subtree: true });
    const giveUp = window.setTimeout(() => stop(), 3000);
    const stop = () => {
      watch.disconnect();
      window.clearTimeout(giveUp);
    };
    return stop;
  }, [found, page]);

  useEffect(() => {
    const id = requestAnimationFrame(() => setShown(true));
    // The surface, not its first control: no stray focus ring.
    popup.current?.focus({ preventScroll: true });
    return () => {
      cancelAnimationFrame(id);
      // Back to what opened it after a close from the keyboard; after one with
      // the pointer it would only paint a focus ring.
      if (byKeyboard.current && opener?.isConnected) opener.focus({ preventScroll: true });
    };
  }, [opener]);

  const close = () => {
    if (leaving) return;
    setLeaving(true);
    window.setTimeout(onClose, 150);
  };

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      // An Escape a menu or a field inside has answered is not this one's.
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      byKeyboard.current = true;
      close();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  });

  const open = shown && !leaving;
  return (
    <InSettings.Provider value={true}>
      <div data-tw>
        <div
          aria-hidden
          className={cn(
            "fixed inset-0 z-[80] bg-[color-mix(in_srgb,var(--bg)_64%,transparent)] backdrop-blur-[4px] transition-opacity duration-200 light:bg-[color-mix(in_srgb,var(--bg)_60%,transparent)]",
            !open && "opacity-0",
          )}
        />
        <div
          className="fixed inset-0 z-[80] flex items-center justify-center sm:p-6 lg:p-10"
          onPointerDown={(e) => {
            pressedBehind.current = e.target === e.currentTarget;
          }}
          onClick={(e) => {
            // A text selection that ends out here is not a dismissal.
            if (pressedBehind.current && e.target === e.currentTarget) close();
            pressedBehind.current = false;
          }}
        >
          <div
            ref={popup}
            role="dialog"
            aria-modal="true"
            aria-label={page === "usage" ? "Usage" : "Settings"}
            tabIndex={-1}
            className={cn(
              "relative flex size-full min-h-0 min-w-0 flex-col overflow-hidden bg-background text-foreground outline-none transition-[opacity,scale] duration-150 ease-out sm:max-h-[60rem] sm:max-w-[80rem] sm:rounded-2xl sm:border sm:shadow-2xl/40",
              !open && "scale-[0.985] opacity-0",
            )}
          >
            <div className="relative flex min-h-0 flex-1">
              {page !== "usage" && <SettingsNav
                page={page}
                onPage={onPage}
                onFind={(to, title) => {
                  onPage(to);
                  setFound({ title, at: Date.now() });
                }}
              />}
              <div className="flex min-h-0 min-w-0 flex-1 flex-col">
                <header className="flex h-12 shrink-0 items-center gap-3 border-b ps-5 pe-2">
                  <ol className="m-0 flex min-w-0 list-none items-center gap-2 p-0 text-sm sm:gap-3" aria-label="Where you are">
                    {/* Usage is a card of its own, as T3 Code's is, not a page of
                        the settings. */}
                    {page !== "usage" && (
                      <>
                        <li className="flex shrink-0 items-center font-medium text-muted-foreground">Settings</li>
                        <li aria-hidden className="flex shrink-0 items-center text-icon-muted">
                          /
                        </li>
                      </>
                    )}
                    <li aria-current="page" className="flex min-w-0 items-center truncate font-medium text-foreground">
                      {SETTINGS_LABELS[page]}
                    </li>
                  </ol>
                  <div className="ms-auto flex shrink-0 items-center gap-1">
                    <div ref={setSlot} className="flex items-center gap-1" />
                    <Button size="icon-sm" variant="ghost" title="Close (Esc)" aria-label="Close" onClick={close}>
                      <Close />
                    </Button>
                  </div>
                </header>
                <HeaderSlot.Provider value={slot}>
                  {/* Keyed by the page, so one page's scroll and half-typed
                      search do not carry into the next. */}
                  <div key={page} ref={body} className="flex min-h-0 flex-1 flex-col">
                    {children}
                  </div>
                </HeaderSlot.Provider>
              </div>
            </div>
          </div>
        </div>
      </div>
    </InSettings.Provider>
  );
}

function SettingsNav({
  page,
  onPage,
  onFind,
}: {
  page: SettingsPageId;
  onPage: (page: SettingsPageId) => void;
  /// A setting found by the search: its page, and the row to go to on it.
  onFind: (page: SettingsPageId, title: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const search = useRef<HTMLInputElement>(null);
  const q = query.trim().toLowerCase();

  // Slash, as T3 Code's, puts you in the search from anywhere in the card
  // that is not already taking typing.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== "/" || e.ctrlKey || e.metaKey || e.altKey) return;
      const at = document.activeElement;
      if (at instanceof HTMLElement && at.closest("input, textarea, [contenteditable]")) return;
      e.preventDefault();
      search.current?.focus();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, []);

  const results = useMemo(() => {
    if (!q) return [];
    const pages = (Object.keys(SETTINGS_LABELS) as SettingsPageId[])
      .filter((p) => SETTINGS_LABELS[p].toLowerCase().includes(q))
      .map((p) => ({ id: `page:${p}`, page: p, title: SETTINGS_LABELS[p], row: false }));
    const rows = FINDS.filter((f) => `${f.title} ${f.terms}`.toLowerCase().includes(q)).map((f) => ({
      id: `row:${f.page}:${f.title}`,
      page: f.page,
      title: f.title,
      row: true,
    }));
    return [...pages, ...rows].slice(0, 12);
  }, [q]);

  const go = (to: SettingsPageId) => {
    setQuery("");
    onPage(to);
  };
  const pick = (r: (typeof results)[number]) => {
    setQuery("");
    if (r.row) onFind(r.page, r.title);
    else onPage(r.page);
  };

  return (
    <nav
      className="flex w-60 shrink-0 flex-col border-e bg-sidebar text-sidebar-foreground max-md:w-14"
      aria-label="Settings"
    >
      <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto p-2">
        <div className="flex flex-col gap-2">
          <label className="flex h-8 items-center gap-2 rounded-md px-2 py-1.5 text-sm font-medium text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground max-md:hidden">
            <Find className="size-4 shrink-0 text-sidebar-muted-foreground/80" />
            <input
              ref={search}
              className="h-auto min-w-0 flex-1 bg-transparent p-0 text-sm leading-normal font-medium text-sidebar-foreground outline-none placeholder:text-sidebar-muted-foreground"
              placeholder="Search"
              aria-label="Search settings"
              spellCheck={false}
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setActive(0);
              }}
              onKeyDown={(e) => {
                if (e.key === "Escape" && query) {
                  // Clears the search rather than closing the card.
                  e.preventDefault();
                  setQuery("");
                  return;
                }
                if (!results.length) return;
                if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                  e.preventDefault();
                  const n = results.length;
                  setActive((a) => (a + (e.key === "ArrowDown" ? 1 : -1) + n) % n);
                } else if (e.key === "Enter") {
                  e.preventDefault();
                  pick(results[active]);
                }
              }}
            />
            {query ? (
              <Button
                size="icon-micro"
                variant="ghost-muted"
                aria-label="Clear the search"
                onClick={() => {
                  setQuery("");
                  search.current?.focus();
                }}
              >
                <Close className="size-3" />
              </Button>
            ) : (
              <kbd className="rounded bg-foreground/7 px-1.5 font-sans text-xs font-medium text-muted-foreground">/</kbd>
            )}
          </label>

          {q ? (
            results.length === 0 ? (
              <p role="status" className="px-2 py-6 text-center text-xs text-sidebar-muted-foreground">
                No settings found
              </p>
            ) : (
              <ul className="flex flex-col gap-1" role="listbox" aria-label="Settings search results">
                {results.map((r, index) => {
                  const Icon = ICONS[r.page];
                  return (
                    <li key={r.id}>
                      <button
                        role="option"
                        aria-selected={index === active}
                        data-active={index === active}
                        className={cn(MENU_BUTTON, "h-auto min-h-10 items-start py-2")}
                        onMouseMove={() => setActive(index)}
                        onClick={() => pick(r)}
                      >
                        <Icon className="size-4 h-5" />
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-sm font-medium text-sidebar-foreground">{r.title}</span>
                          <span className="block truncate text-2xs text-sidebar-muted-foreground/75">
                            {SETTINGS_LABELS[r.page]}
                          </span>
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )
          ) : (
            <div className="flex flex-col gap-4">
              {NAV.map((group, index) => (
                <div key={group.heading} className="flex flex-col gap-1">
                  <h3
                    id={`settings-nav-group-${index}`}
                    className={cn(
                      "px-2 text-xs font-medium text-sidebar-muted-foreground/80",
                      group.hidden ? "sr-only" : "max-md:sr-only",
                    )}
                  >
                    {group.heading}
                  </h3>
                  <ul role="group" aria-labelledby={`settings-nav-group-${index}`} className="flex flex-col gap-1">
                    {group.pages.map((to) => {
                      const Icon = ICONS[to];
                      const on = to === page;
                      return (
                        <li key={to}>
                          <button
                            data-active={on}
                            aria-current={on ? "page" : undefined}
                            title={SETTINGS_LABELS[to]}
                            className={MENU_BUTTON}
                            onClick={() => go(to)}
                          >
                            <Icon />
                            <span className="truncate max-md:sr-only">{SETTINGS_LABELS[to]}</span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </nav>
  );
}

/// T3 Code's `SidebarMenuButton`, its default size and variant.
const MENU_BUTTON =
  "flex h-8 w-full cursor-pointer items-center gap-2 overflow-hidden rounded-[var(--control-radius)] px-2.5 py-1.5 text-left text-sm font-medium text-sidebar-muted-foreground/80 outline-hidden ring-ring hover:bg-sidebar-row-hover hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-inset active:bg-sidebar-row-active active:text-sidebar-foreground data-[active=true]:bg-sidebar-row-selected data-[active=true]:text-sidebar-foreground [&>svg]:shrink-0 [&>svg:not([class*='size-'])]:size-4 [&>svg]:text-[color-mix(in_srgb,var(--dim)_85%,var(--chrome))] hover:[&>svg]:text-sidebar-foreground data-[active=true]:[&>svg]:text-sidebar-foreground";
