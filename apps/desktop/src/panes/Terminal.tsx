// The agent's screen, live.
//
// xterm.js over the terminal channel: bytes out of the sandbox's tmux into the
// emulator, keystrokes back. Nothing here interprets the stream -- the escape
// sequences are the agent's, and the one thing this must not do is try to
// understand them.
//
// Copy and paste are a terminal's, not a web page's. tmux has the mouse, so a
// drag selects in tmux and copies on release -- which reaches here as OSC 52
// and goes to the clipboard -- and the wheel scrolls tmux's history. Holding
// Shift (Option on macOS) selects in the pane instead, for the screen as drawn.
// Ctrl+Shift+C and Ctrl+Shift+V copy and paste where Ctrl+C has to stay an
// interrupt; on macOS ⌘C and ⌘V already do, through the Edit menu.
//
// `xterm.open` goes through `withUsableFontMetrics` because WebKitGTK reports a
// font's vertical metrics wrongly and xterm believes them. See charSize.ts:
// without it the character cell is zero -- a pane that stays empty however much
// arrives in the buffer behind it -- and every row it does draw is sheared off
// two pixels short of the top.

import { useEffect, useRef } from "react";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal as Xterm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import { withUsableFontMetrics } from "../charSize";
import { openExternal } from "../open";
import { palette } from "../palette";
import { close, decodeBytes, encodeBytes, nextChannelId, open, terminal } from "../stream";

const MAC = navigator.userAgent.includes("Mac");
const WINDOWS = /windows/i.test(navigator.userAgent);

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
  background: ["--sunken", "#0a0a0a"],
  foreground: ["--text", "#fafafa"],
  // No accent to borrow: the window's hues mean "working", "needs you", "good"
  // and "bad", and a cursor is none of those. Near-white on near-black, which
  // is how everything else in the window takes emphasis.
  cursor: ["--text", "#fafafa"],
  // Nothing for the scrollbar, because this pane does not draw one -- see
  // style.css. Three colours for a slider that is `display: none` would be
  // three more things to keep in step with a palette for no pixels at all.
} satisfies Record<string, [string, string]>;

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

  const pane = element.getBoundingClientRect();
  if (!(pane.width > 0) || !(pane.height > 0)) return null;

  // The same floors and the same minimums as `FitAddon`: a terminal of zero
  // columns is not a smaller terminal, it is one that cannot be written to.
  return {
    cols: Math.max(2, Math.floor(pane.width / cell.width)),
    rows: Math.max(1, Math.floor(pane.height / cell.height)),
  };
}

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

  useEffect(() => {
    const element = host.current;
    if (!element) return;

    const xterm = new Xterm({
      convertEol: false,
      cursorBlink: true,
      // Ends in `monospace` so there is always something the platform can
      // resolve: xterm sizes its grid by measuring one character in this font,
      // and a family it cannot resolve measures zero.
      fontFamily: 'ui-monospace, "Cascadia Mono", Menlo, Consolas, monospace',
      fontSize: 13,
      // Taken from style.css, because the emulator paints its own background
      // and would otherwise sit as a rectangle of some other colour inside the
      // pane. See `palette`.
      theme: palette(THEME),
      // The sandbox's tmux keeps the scrollback that matters; this is just what
      // the pane can scroll back through without asking for it again.
      scrollback: 5000,
      // Shift already does this everywhere else; see the note at the top.
      macOptionClickForcesSelection: true,
    });
    withUsableFontMetrics(element, () => xterm.open(element));

    // Ctrl-click (⌘-click) on an address, as terminals do it: a plain click is
    // tmux's, and opening a browser on every click that lands on a URL is not
    // what anyone clicking in a terminal meant.
    xterm.loadAddon(
      new WebLinksAddon((event, uri) => {
        if (MAC ? event.metaKey : event.ctrlKey) openExternal(uri);
      }),
    );

    const copy = () => {
      const text = xterm.getSelection();
      if (text) void writeText(text).catch(() => {});
      xterm.clearSelection();
    };
    const paste = () => {
      readText()
        .then((text) => text && xterm.paste(text))
        .catch(() => {});
    };
    xterm.attachCustomKeyEventHandler((e) => {
      const action = clipboardKey(e, xterm.hasSelection());
      if (!action) return true;
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

    const id = nextChannelId();
    let live = true;

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
    }).catch((e) => xterm.writeln(`\r\n\x1b[31m${String(e)}\x1b[0m`));

    const typed = xterm.onData((data) => {
      terminal.input(id, encodeBytes(new TextEncoder().encode(data))).catch(() => {});
    });

    // The pane resizes with the window and with the split; both go through the
    // observer rather than a window listener, which would miss the split.
    const observer = new ResizeObserver(refit);
    observer.observe(element);
    const resized = xterm.onResize(sendSize);

    return () => {
      live = false;
      observer.disconnect();
      typed.dispose();
      resized.dispose();
      clipboard.dispose();
      void close(id);
      xterm.dispose();
    };
  }, [server, name, tmux]);

  return <div className="terminal" ref={host} />;
}
