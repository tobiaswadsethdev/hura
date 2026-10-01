// What you can do to the thing under the pointer, without a row of buttons on
// every row.
//
// The window used to answer "where does this action go?" with an icon beside
// the thing it acts on, revealed on hover. That is fine for one action and
// clutter for three, and it puts the dangerous ones -- destroy, discard, block
// -- one stray click from the row you were only trying to select. A menu is
// the other answer: nothing on screen until asked for, every action named in
// words, and the dangerous ones in red at the bottom behind a deliberate
// gesture.
//
// One menu for the whole window, held by a provider, so opening a second
// closes the first and there is one set of rules for closing: a click
// elsewhere, Escape, a scroll, the window losing focus. A hook hands out the
// props a row spreads onto itself, which is the whole of what a caller writes.
//
// The keyboard is not an afterthought. The Menu key and Shift+F10 open it from
// a focused row, as they do everywhere else; the arrows move, Enter runs, and
// Escape puts the focus back where it was.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

/// One line of a menu.
export type MenuItem =
  | {
      label: string;
      icon?: React.ComponentType<{ "aria-hidden"?: boolean }>;
      /// A short qualifier, right-aligned and dim: which binary, which list.
      hint?: string;
      /// Red, and only for the things that cannot be taken back.
      danger?: boolean;
      disabled?: boolean;
      run: () => void;
    }
  | "separator";

type Open = { x: number; y: number; items: MenuItem[]; from: HTMLElement | null; keyboard: boolean };

const Ctx = createContext<(open: Open) => void>(() => {});

export function MenuProvider({ children }: { children: React.ReactNode }) {
  const [open, setOpen] = useState<Open | null>(null);

  // No webview menu anywhere else. Its Reload, Back and Inspect Element are a
  // browser's, not this window's, and a right-click that offers them on a row
  // with no menu of its own reads as a page rather than an application.
  //
  // Two exceptions. Text fields keep theirs, because it is where paste lives
  // and nothing here replaces it. And a development build keeps it behind
  // Shift, because Inspect Element is how this window gets debugged.
  useEffect(() => {
    const native = (e: MouseEvent) => {
      if (e.defaultPrevented) return;
      if (import.meta.env.DEV && e.shiftKey) return;
      const target = e.target as HTMLElement | null;
      if (target?.closest("input, textarea, [contenteditable=''], [contenteditable='true']")) return;
      e.preventDefault();
    };
    window.addEventListener("contextmenu", native);
    return () => window.removeEventListener("contextmenu", native);
  }, []);

  return (
    <Ctx.Provider value={setOpen}>
      {children}
      {open && <Menu {...open} onClose={() => setOpen(null)} />}
    </Ctx.Provider>
  );
}

/// The props that make an element open a menu: right-click, or the Menu key
/// and Shift+F10 while it has focus.
///
/// `items` is a function so the menu is built from the state at the moment it
/// opens, not the state at the last render. An empty list opens nothing, and
/// the browser's own menu is left alone.
export function useContextMenu() {
  const show = useContext(Ctx);
  return useCallback(
    (items: () => MenuItem[]) => ({
      onContextMenu: (e: React.MouseEvent<HTMLElement>) => {
        const list = items();
        if (list.length === 0) return;
        e.preventDefault();
        e.stopPropagation();
        show({ x: e.clientX, y: e.clientY, items: list, from: e.currentTarget, keyboard: false });
      },
      onKeyDown: (e: React.KeyboardEvent<HTMLElement>) => {
        if (e.key !== "ContextMenu" && !(e.key === "F10" && e.shiftKey)) return;
        const list = items();
        if (list.length === 0) return;
        e.preventDefault();
        e.stopPropagation();
        const r = e.currentTarget.getBoundingClientRect();
        show({ x: r.left + 12, y: r.bottom, items: list, from: e.currentTarget, keyboard: true });
      },
    }),
    [show],
  );
}

/// Put text on the clipboard. Here because "copy" is the one item nearly every
/// menu has, and a failure is not worth more than silence: the menu has
/// already closed, and there is nowhere honest to say so.
export function copy(text: string) {
  void navigator.clipboard?.writeText(text).catch(() => {});
}

function Menu({ x, y, items, from, keyboard, onClose }: Open & { onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState({ x, y });

  // Kept inside the window: flipped up or left when the pointer is close
  // enough to an edge that the menu would run off it.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const pad = 6;
    setAt({
      x: x + width + pad > window.innerWidth ? Math.max(pad, x - width) : x,
      y: y + height + pad > window.innerHeight ? Math.max(pad, y - height) : y,
    });
  }, [x, y]);

  const buttons = () =>
    [...(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];

  const close = useCallback(
    (restore: boolean) => {
      onClose();
      if (restore) from?.focus();
    },
    [onClose, from],
  );

  useEffect(() => {
    // From the keyboard the first item has the focus, so Enter does the
    // obvious thing; from the pointer the menu itself does, so the arrows work
    // without an item looking pre-chosen.
    if (keyboard) buttons()[0]?.focus();
    else ref.current?.focus();

    const away = (e: Event) => {
      if (!ref.current?.contains(e.target as Node)) close(false);
    };
    const gone = () => close(false);
    window.addEventListener("mousedown", away, true);
    window.addEventListener("contextmenu", away, true);
    window.addEventListener("scroll", gone, true);
    window.addEventListener("resize", gone);
    window.addEventListener("blur", gone);
    return () => {
      window.removeEventListener("mousedown", away, true);
      window.removeEventListener("contextmenu", away, true);
      window.removeEventListener("scroll", gone, true);
      window.removeEventListener("resize", gone);
      window.removeEventListener("blur", gone);
    };
    // Once per opening: the provider remounts this for every new menu.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const all = buttons();
    const i = all.indexOf(document.activeElement as HTMLButtonElement);
    const move = (to: number) => {
      e.preventDefault();
      all[(to + all.length) % all.length]?.focus();
    };
    if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      close(true);
    } else if (e.key === "ArrowDown") move(i + 1);
    else if (e.key === "ArrowUp") move(i < 0 ? -1 : i - 1);
    else if (e.key === "Home") move(0);
    else if (e.key === "End") move(-1);
  };

  return createPortal(
    <div
      ref={ref}
      className="context-menu"
      role="menu"
      tabIndex={-1}
      style={{ left: at.x, top: at.y }}
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((item, i) =>
        item === "separator" ? (
          <div key={i} className="context-sep" role="separator" />
        ) : (
          <button
            key={i}
            role="menuitem"
            className={item.danger ? "danger" : ""}
            disabled={item.disabled}
            onClick={() => {
              close(true);
              item.run();
            }}
          >
            <span className="context-icon">{item.icon && <item.icon aria-hidden />}</span>
            <span className="context-label">{item.label}</span>
            {item.hint && <span className="context-hint">{item.hint}</span>}
          </button>
        ),
      )}
    </div>,
    document.body,
  );
}
