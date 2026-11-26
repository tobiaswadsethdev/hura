// The agent's screen, live.
//
// xterm.js over the terminal channel: bytes out of the sandbox's tmux into the
// emulator, keystrokes back. Nothing here interprets the stream -- the escape
// sequences are the agent's, and the one thing this must not do is try to
// understand them.
//
// The mouse is this pane's, not tmux's -- with one exception, the wheel. tmux
// asks for the mouse when it attaches, and while it had it, a drag was a tmux
// selection that vanished on release, a right-click was tmux's own menu of
// pane commands, and copying anything meant knowing to hold Shift. So the
// request is kept from xterm (see `MOUSE_MODES`): a drag selects here, a
// double-click takes a word, a right-click opens this window's menu. The
// wheel still goes to tmux, sent from here as tmux asked for it, because
// tmux's history is the only history there is.
//
// Scrolling back puts tmux in copy mode, where keys move about the history
// rather than reaching the program. A terminal snaps to the bottom when you
// type, so this does too: the first key while scrolled back leaves copy mode
// before it is sent, and a button over the screen does the same.
//
// Ctrl+Shift+C and Ctrl+Shift+V copy and paste where Ctrl+C has to stay an
// interrupt; on macOS ⌘C and ⌘V already do, through the Edit menu. What a
// program in the sandbox copies arrives as OSC 52 and goes to the clipboard.
//
// `xterm.open` goes through `withUsableFontMetrics` because WebKitGTK reports a
// font's vertical metrics wrongly and xterm believes them. See charSize.ts:
// without it the character cell is zero -- a pane that stays empty however much
// arrives in the buffer behind it -- and every row it does draw is sheared off
// two pixels short of the top.

import { useEffect, useRef, useState } from "react";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal as Xterm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import { withUsableFontMetrics } from "../charSize";
import { useContextMenu } from "../ContextMenu";
import { Copy, Latest, Paste, SelectAll } from "../icons";
import { openExternal } from "../open";
import { messageOf } from "../api";
import { palette } from "../palette";
import { close, decodeBytes, encodeBytes, nextChannelId, open, terminal } from "../stream";
import { onScheme, terminalScheme, type Scheme } from "../theme";
import { codeFont, onLook, themeAnsi } from "../look";

const MAC = navigator.userAgent.includes("Mac");
const WINDOWS = /windows/i.test(navigator.userAgent);

/// Keys that are only ever a modifier, and type nothing on their own.
const MODIFIERS = new Set(["Shift", "Control", "Alt", "Meta", "CapsLock", "Fn", "OS"]);

/// What a key does to the clipboard, if anything, before the terminal sees it.
///
/// Off macOS, where Ctrl is the terminal's: Ctrl+Shift+C and Ctrl+Shift+V as
/// every Linux terminal has them, Shift+Insert, Ctrl+C only while something is
/// selected -- otherwise it is the interrupt, and must stay one -- and Ctrl+V
/// on Windows, where Windows Terminal has taught everyone it pastes.
function clipboardKey(e: KeyboardEvent, selecting: boolean): "copy" | "paste" | null {
  if (MAC || e.altKey || e.metaKey) return null;
  const key = e.key.toLowerCase();
  if (e.ctrlKey && e.shiftKey && key === "c") return "copy";
  if (e.ctrlKey && e.shiftKey && key === "v") return "paste";
  if (!e.ctrlKey && e.shiftKey && e.key === "Insert") return "paste";
  if (e.ctrlKey && !e.shiftKey && key === "c" && selecting) return "copy";
  if (e.ctrlKey && !e.shiftKey && key === "v" && WINDOWS) return "paste";
  return null;
}

/// The text of an OSC 52, or `null` for one that is not putting text on the
/// clipboard.
///
/// The payload is `<selections>;<base64>`. A `?` there asks for the clipboard
/// to be *read* back into the stream, and is never answered: anything running
/// in the sandbox could ask.
function osc52Text(data: string): string | null {
  const semi = data.indexOf(";");
  if (semi < 0) return null;
  const payload = data.slice(semi + 1);
  if (!payload || payload === "?") return null;
  try {
    return new TextDecoder().decode(decodeBytes(payload));
  } catch {
    return null;
  }
}

/// Which custom property each of xterm's colours comes from, and what to fall
/// back to. See `palette.ts` for why this is resolved rather than written down.
const THEME = {
  background: ["--sunken", "#161617"],
  foreground: ["--term-fg", "#e4e4e6"],
  // No accent to borrow: the window's hues mean "working", "needs you", "good"
  // and "bad", and a cursor is none of those. The text colour, which is how
  // everything else in the window takes emphasis.
  cursor: ["--term-fg", "#e4e4e6"],
  cursorAccent: ["--sunken", "#161617"],
  selectionBackground: ["--term-selection", "#0a84ff66"],
  // Nothing for the scrollbar, because this pane does not draw one -- see
  // style.css. Three colours for a slider that is `display: none` would be
  // three more things to keep in step with a palette for no pixels at all.
} satisfies Record<string, [string, string]>;

/// The sixteen colours a program names by number, for each palette.
///
/// xterm's own are the Tango set, which is neither this window's nor drawn for
/// a light background. These are the system's hues, as the rest of the window
/// uses them, and as the phone's are (`TerminalScreen.swift`). In light,
/// `white` stays light, because it is as often a background as a text colour;
/// where it is text, `minimumContrastRatio` darkens it, cell by cell, against
/// whatever is behind it.
const ANSI: Record<Scheme, Record<string, string>> = {
  dark: {
    black: "#1c1c1e", red: "#ff453a", green: "#32d74b", yellow: "#ffd60a",
    blue: "#0a84ff", magenta: "#bf5af2", cyan: "#64d2ff", white: "#d1d1d6",
    brightBlack: "#8e8e93", brightRed: "#ff6961", brightGreen: "#30db5b", brightYellow: "#ffe066",
    brightBlue: "#409cff", brightMagenta: "#da8fff", brightCyan: "#70d7ff", brightWhite: "#ffffff",
  },
  light: {
    black: "#1d1d1f", red: "#d70015", green: "#248a3d", yellow: "#a05a00",
    blue: "#0040dd", magenta: "#8944ab", cyan: "#0071a4", white: "#d1d1d6",
    brightBlack: "#6e6e73", brightRed: "#ff3b30", brightGreen: "#28a745", brightYellow: "#c78a00",
    brightBlue: "#007aff", brightMagenta: "#af52de", brightCyan: "#0a8fbf", brightWhite: "#f2f2f7",
  },
};

/// How hard xterm works to keep text readable. Off in dark, where what runs
/// here was drawn to be read. In light it is what makes a program written for
/// a dark terminal legible at all -- Claude Code's pale greys, a `white` that
/// was never meant for paper -- and 4.5 is WCAG's AA, the least that reads.
const CONTRAST: Record<Scheme, number> = { dark: 1, light: 4.5 };

/// The DEC private modes that hand the mouse to the program, and their
/// encodings. tmux sets these when it attaches with `mouse on`; xterm, seeing
/// them, would send every click and drag to tmux and select nothing itself.
/// See the note at the top.
const MOUSE_MODES = new Set([9, 1000, 1001, 1002, 1003, 1005, 1006, 1015, 1016]);
const TRACKING = new Set([9, 1000, 1001, 1002, 1003]);
const SGR = 1006;

/// How many lines of wheel travel make one click sent to tmux, which scrolls
/// five lines a click. Three, as on the phone: a little faster than the hand,
/// and a trackpad's flick still lands somewhere you can read.
const LINES_PER_CLICK = 3;

/// Whether tmux is showing its history rather than the live screen.
///
/// tmux says so in one place a client can see: copy mode draws its position,
/// `[12/340]`, at the right of the top row. A program could draw the same, so
/// this is only asked after this pane has scrolled tmux back itself.
function inHistory(xterm: Xterm): boolean {
  const buffer = xterm.buffer.active;
  const top = buffer.getLine(buffer.viewportY)?.translateToString(true) ?? "";
  return /\[\d+\/\d+\]$/.test(top.trimEnd());
}

/// How many characters fit in the pane, across and down.
///
/// This is `FitAddon`'s job and it is not used for it, because of one line in
/// it: the width it fits into is the pane's *minus a scrollbar's*, always, on
/// the assumption that the scrollbar sits beside the text. This one does not --
/// xterm 6 draws it as an element positioned over the right of the screen, and
/// it fades out when the pointer is elsewhere. So the fourteen pixels were being
/// held open for something that was never going to occupy them, and the agent's
/// screen was fourteen pixels narrower than the pane for it. Which is invisible
/// until the agent draws a rule across its own full width -- and then it is a
/// line that stops short of an edge it is plainly meant to reach.
///
/// The cell is measured off what xterm has already drawn rather than asked for:
/// the screen element is exactly `cols` by `rows` cells, so dividing gives the
/// cell without reaching into `_core` for the render service, which is the
/// private API `FitAddon` carries a `TODO` about. Before the first render the
/// grid is xterm's default 80x24 and the screen has a size, so this holds from
/// the first call.
///
/// Up to one cell is still left over at the right and the bottom: a grid cannot
/// draw a fraction of a column, and rounding up would clip one instead. That
/// residue is the terminal's, not the pane's.
function gridFor(element: HTMLElement, xterm: Xterm): { cols: number; rows: number } | null {
  const screen = element.querySelector<HTMLElement>(".xterm-screen");
  if (!screen) return null;

  const cell = { width: screen.clientWidth / xterm.cols, height: screen.clientHeight / xterm.rows };
  if (!(cell.width > 0) || !(cell.height > 0)) return null;

  // The content box: the pane's padding is a margin around the screen, not
  // room for another column.
  const style = getComputedStyle(element);
  const pane = {
    width: element.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight),
    height: element.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom),
  };
  if (!(pane.width > 0) || !(pane.height > 0)) return null;

  // The same floors and the same minimums as `FitAddon`: a terminal of zero
  // columns is not a smaller terminal, it is one that cannot be written to.
  return {
    cols: Math.max(2, Math.floor(pane.width / cell.width)),
    rows: Math.max(1, Math.floor(pane.height / cell.height)),
  };
}

/// What the pane's menu and its button reach into the effect for.
type Handle = {
  xterm: Xterm;
  copy: () => void;
  paste: () => void;
  /// Out of tmux's history and back to the live screen.
  latest: () => void;
};

export function TerminalPane({
  server,
  name,
  tmux,
}: {
  server: string;
  name: string;
  /// Which tmux session in the sandbox. `null` is the agent's own, which is
  /// what this meant before there were others.
  tmux: string | null;
}) {
  const host = useRef<HTMLDivElement>(null);
  const handle = useRef<Handle | null>(null);
  const [scheme, setScheme] = useState<Scheme>(terminalScheme);
  // Whether tmux is in its history, so the way back is on screen.
  const [back, setBack] = useState(false);
  const menu = useContextMenu();

  useEffect(() => onScheme(() => setScheme(terminalScheme())), []);
  // A theme taken in, or another face: counted, so the palette below is read
  // again in the same light or dark.
  const [looks, setLooks] = useState(0);
  useEffect(() => onLook(() => setLooks((n) => n + 1)), []);

  useEffect(() => {
    const element = host.current;
    if (!element) return;

    const scheme = terminalScheme();
    const xterm = new Xterm({
      convertEol: false,
      cursorBlink: true,
      // Ends in `monospace` so there is always something the platform can
      // resolve: xterm sizes its grid by measuring one character in this font,
      // and a family it cannot resolve measures zero.
      fontFamily: codeFont().family,
      fontSize: codeFont().size,
      // Taken from style.css, because the emulator paints its own background
      // and would otherwise sit as a rectangle of some other colour inside the
      // pane. See `palette`.
      theme: { ...palette(THEME, element), ...ANSI[scheme], ...themeAnsi(scheme) },
      minimumContrastRatio: CONTRAST[scheme],
      // The sandbox's tmux keeps the scrollback that matters; this is just what
      // the pane can scroll back through without asking for it again.
      scrollback: 5000,
      // A right-click takes the word under it first, so Copy in the menu has
      // something to copy -- as a right-click in any Mac text view does.
      rightClickSelectsWord: true,
    });
    withUsableFontMetrics(element, () => xterm.open(element));

    const id = nextChannelId();
    let live = true;
    const send = (data: string) => {
      terminal.input(id, encodeBytes(new TextEncoder().encode(data))).catch(() => {});
    };

    // Ctrl-click (⌘-click) on an address, as terminals do it: a plain click
    // starts a selection, and opening a browser on every click that lands on a
    // URL is not what anyone clicking in a terminal meant.
    xterm.loadAddon(
      new WebLinksAddon((event, uri) => {
        if (MAC ? event.metaKey : event.ctrlKey) openExternal(uri);
      }),
    );

    // What tmux asked of the mouse, kept here instead of given to xterm. See
    // `MOUSE_MODES`.
    const mouse = { tracking: false, sgr: false };
    const modes = (set: boolean) => (params: (number | number[])[]) => {
      const all = params.flat();
      // Mixed in with a mode that is not the mouse's, the whole sequence goes
      // through: xterm cannot be handed half of one. tmux sends each alone.
      if (all.length === 0 || !all.every((p) => MOUSE_MODES.has(p))) return false;
      for (const p of all) {
        if (TRACKING.has(p)) mouse.tracking = set;
        if (p === SGR) mouse.sgr = set;
      }
      return true;
    };
    const setModes = xterm.parser.registerCsiHandler({ prefix: "?", final: "h" }, modes(true));
    const resetModes = xterm.parser.registerCsiHandler({ prefix: "?", final: "l" }, modes(false));

    // Whether this pane has scrolled tmux back, and when it last did: the
    // position tmux draws is only believed after that. See `inHistory`.
    let scrolled = false;
    let scrolledAt = 0;
    const showBack = (on: boolean) => {
      if (live) setBack(on);
    };
    const latest = () => {
      if (scrolled && inHistory(xterm)) send("q");
      scrolled = false;
      showBack(false);
    };
    const parsed = xterm.onWriteParsed(() => {
      if (!scrolled) return;
      const now = inHistory(xterm);
      // Not yet in it is not the same as out of it: output can arrive between
      // the wheel and tmux's answer to it.
      if (!now && performance.now() - scrolledAt > 1000) scrolled = false;
      showBack(now);
    });

    // The wheel, as tmux asked to be sent it: one click per few lines of
    // travel, at the cell under the pointer.
    let travel = 0;
    xterm.attachCustomWheelEventHandler((e) => {
      if (!mouse.tracking) return true;
      e.preventDefault();
      const screen = element.querySelector<HTMLElement>(".xterm-screen");
      if (!screen) return false;
      const rect = screen.getBoundingClientRect();
      const cell = { width: rect.width / xterm.cols, height: rect.height / xterm.rows };
      if (!(cell.height > 0)) return false;
      travel +=
        e.deltaMode === WheelEvent.DOM_DELTA_LINE
          ? e.deltaY
          : e.deltaMode === WheelEvent.DOM_DELTA_PAGE
            ? e.deltaY * xterm.rows
            : e.deltaY / cell.height;
      const col = Math.min(xterm.cols, Math.max(1, Math.floor((e.clientX - rect.left) / cell.width) + 1));
      const row = Math.min(xterm.rows, Math.max(1, Math.floor((e.clientY - rect.top) / cell.height) + 1));
      while (Math.abs(travel) >= LINES_PER_CLICK) {
        const up = travel < 0;
        const button = up ? 64 : 65;
        send(
          mouse.sgr
            ? `\x1b[<${button};${col};${row}M`
            : `\x1b[M${String.fromCharCode(32 + button, 32 + Math.min(col, 223), 32 + Math.min(row, 223))}`,
        );
        if (up) {
          scrolled = true;
          scrolledAt = performance.now();
        }
        travel += up ? LINES_PER_CLICK : -LINES_PER_CLICK;
      }
      return false;
    });

    const copy = () => {
      const text = xterm.getSelection();
      if (text) void writeText(text).catch(() => {});
      xterm.clearSelection();
    };
    const paste = () => {
      readText()
        .then((text) => {
          if (!text) return;
          latest();
          xterm.paste(text);
        })
        .catch(() => {});
    };
    handle.current = { xterm, copy, paste, latest };
    // ⌘V reaches xterm as a paste event through the Edit menu, not as a key.
    // Caught on the way down, so copy mode is left before the text is sent.
    const pasting = () => {
      if (scrolled) latest();
    };
    element.addEventListener("paste", pasting, true);

    xterm.attachCustomKeyEventHandler((e) => {
      const action = clipboardKey(e, xterm.hasSelection());
      if (!action) {
        // A key meant for the program, while tmux shows its history, would be
        // taken by copy mode instead. Out first, as a terminal snaps to the
        // bottom when you type. Not for a shortcut of the window's own -- ⌘C
        // copying out of the history is the reason to be in it -- nor for a
        // modifier on its own.
        if (e.type === "keydown" && scrolled && !e.metaKey && !MODIFIERS.has(e.key)) latest();
        return true;
      }
      // Every phase of the key is swallowed, and only the press acts. The
      // default is prevented as well: returning false keeps the key from the
      // terminal, but not from a webview that would paste a second time.
      e.preventDefault();
      if (e.type === "keydown") (action === "copy" ? copy : paste)();
      return false;
    });
    const clipboard = xterm.parser.registerOscHandler(52, (data) => {
      const text = osc52Text(data);
      if (text) void writeText(text).catch(() => {});
      return true;
    });

    const refit = () => {
      const size = gridFor(element, xterm);
      // A pane with no size yet -- a tab that is not on screen -- measures
      // nothing. Not a failure, and not worth resizing to.
      if (!size) return;
      if (size.cols !== xterm.cols || size.rows !== xterm.rows) {
        xterm.resize(size.cols, size.rows);
      }
    };

    // After the fonts and after a layout: `refit` measures the element, and
    // immediately after `open` the browser has laid out neither. Measuring
    // early leaves the default 80x24, which is then what the *server* sizes its
    // pty to -- the agent's screen comes back wrapped to eighty columns inside
    // a pane three times as wide.
    void document.fonts.ready.then(() => requestAnimationFrame(refit));

    const sendSize = () => {
      // The server has no other way to know how wide this pane is: there is no
      // terminal on its side to take a size from, so the pty it allocates is
      // sized from here.
      terminal.resize(id, xterm.cols, xterm.rows).catch(() => {});
    };

    void open(server, id, { kind: "terminal", session: name, tmux }, (frame) => {
      if (!live) return;
      switch (frame.is) {
        case "opened":
          sendSize();
          break;
        case "output":
          xterm.write(decodeBytes(frame.data));
          break;
        case "closed":
          xterm.writeln(`\r\n\x1b[33m-- ${frame.reason ?? "detached"} --\x1b[0m`);
          break;
      }
    }).catch((e) => xterm.writeln(`\r\n\x1b[31m${messageOf(e)}\x1b[0m`));

    const typed = xterm.onData(send);

    // The pane resizes with the window and with the split; both go through the
    // observer rather than a window listener, which would miss the split.
    const observer = new ResizeObserver(refit);
    observer.observe(element);
    // Another face or size is another cell size, and so another grid in the
    // same pane.
    const unlook = onLook(() => {
      const { family, size } = codeFont();
      if (xterm.options.fontFamily === family && xterm.options.fontSize === size) return;
      xterm.options.fontFamily = family;
      xterm.options.fontSize = size;
      requestAnimationFrame(refit);
    });
    const resized = xterm.onResize(sendSize);

    return () => {
      live = false;
      handle.current = null;
      element.removeEventListener("paste", pasting, true);
      observer.disconnect();
      unlook();
      typed.dispose();
      resized.dispose();
      parsed.dispose();
      setModes.dispose();
      resetModes.dispose();
      clipboard.dispose();
      void close(id);
      xterm.dispose();
    };
  }, [server, name, tmux]);

  // The palette, again, when the window's changes. After the commit, so the
  // properties read are the ones `data-theme` now selects.
  useEffect(() => {
    const xterm = handle.current?.xterm;
    const element = host.current;
    if (!xterm || !element) return;
    xterm.options.theme = { ...palette(THEME, element), ...ANSI[scheme], ...themeAnsi(scheme) };
    xterm.options.minimumContrastRatio = CONTRAST[scheme];
  }, [scheme, looks]);

  const menuProps = menu(() => {
    const h = handle.current;
    if (!h) return [];
    const done = (run: () => void) => () => {
      run();
      h.xterm.focus();
    };
    return [
      {
        label: "Copy",
        icon: Copy,
        hint: MAC ? "⌘C" : "Ctrl+Shift+C",
        disabled: !h.xterm.hasSelection(),
        run: done(h.copy),
      },
      { label: "Paste", icon: Paste, hint: MAC ? "⌘V" : "Ctrl+Shift+V", run: done(h.paste) },
      { label: "Select All", icon: SelectAll, run: done(() => h.xterm.selectAll()) },
      ...(back
        ? (["separator", { label: "Back to Latest", icon: Latest, run: done(h.latest) }] as const)
        : []),
    ];
  });

  return (
    <div className="terminal" data-theme={scheme} onContextMenu={menuProps.onContextMenu}>
      <div className="terminal-screen" ref={host} />
      {back && (
        <button
          className="terminal-latest"
          onClick={() => {
            handle.current?.latest();
            handle.current?.xterm.focus();
          }}
        >
          <Latest aria-hidden />
          Back to latest
        </button>
      )}
    </div>
  );
}
