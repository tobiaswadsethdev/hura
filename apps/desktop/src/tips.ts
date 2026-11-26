// Every `title` in the window, drawn as T3 Code's tooltip rather than the
// system's: a small card over the control, 4 pixels above it (below it where
// there is no room), that fades and grows in, with a shortcut at the end of
// the text drawn as a key.
//
// One layer for the whole window rather than a component at each control:
// the window has hundreds of `title`s, and every one of them is the right
// text already. While the pointer is on a control (or the keyboard has put
// the focus there) its `title` is held here, so the system does not draw its
// own as well, and put back when it leaves.
//
// The timing is Base UI's, which T3 Code's tooltips are: 600 ms before the
// first opens, and none for the next while one has just closed, so running
// the pointer along a row of icons reads each at once.

const OPEN_MS = 600;
const WARM_MS = 400;
const GAP = 4;
const EDGE = 6;

/// "Send (Enter)", "Search Conversations (Ctrl+Shift+F)": the words, and the
/// shortcut in the brackets at the end when it is one.
const SHORTCUT =
  /^(.*\S)\s*\(((?:Ctrl|Cmd|Command|Option|Alt|Shift|Esc|Enter|Return|Tab|Space|F\d{1,2}|[⌘⌥⇧⌃↵])[^()]*)\)$/;

export function installTooltips() {
  const tip = document.createElement("div");
  tip.className = "tip";
  tip.setAttribute("role", "tooltip");
  tip.setAttribute("aria-hidden", "true");
  document.body.append(tip);

  let target: Element | null = null;
  let text = "";
  let timer = 0;
  let closedAt = 0;

  const putBack = () => {
    if (target && text && !target.hasAttribute("title")) target.setAttribute("title", text);
  };

  const hide = () => {
    clearTimeout(timer);
    putBack();
    target = null;
    if (tip.hasAttribute("data-open")) {
      tip.removeAttribute("data-open");
      closedAt = Date.now();
    }
  };

  const draw = () => {
    tip.replaceChildren();
    const m = SHORTCUT.exec(text);
    const words = document.createElement("span");
    words.textContent = m ? m[1] : text;
    tip.append(words);
    if (m) {
      const key = document.createElement("kbd");
      key.textContent = m[2];
      tip.append(key);
    }
  };

  const show = () => {
    if (!target || !target.isConnected) return hide();
    draw();
    const r = target.getBoundingClientRect();
    const { offsetWidth: w, offsetHeight: h } = tip;
    let top = r.top - h - GAP;
    let side = "top";
    if (top < EDGE) {
      top = r.bottom + GAP;
      side = "bottom";
    }
    const left = Math.min(Math.max(EDGE, r.left + r.width / 2 - w / 2), window.innerWidth - w - EDGE);
    tip.style.left = `${Math.round(left)}px`;
    tip.style.top = `${Math.round(top)}px`;
    tip.dataset.side = side;
    tip.setAttribute("data-open", "");
  };

  const arm = (el: Element) => {
    if (el === target) return;
    hide();
    const said = el.getAttribute("title");
    if (!said || !said.trim()) return;
    target = el;
    text = said;
    el.removeAttribute("title");
    const warm = Date.now() - closedAt < WARM_MS;
    timer = window.setTimeout(show, warm ? 0 : OPEN_MS);
  };

  document.addEventListener(
    "pointerover",
    (e) => {
      const at = e.target as Element | null;
      if (!at || (target && target.contains(at))) return;
      const el = at.closest?.("[title]");
      if (el) arm(el);
      else hide();
    },
    true,
  );
  document.addEventListener(
    "pointerout",
    (e) => {
      if (!target) return;
      const to = e.relatedTarget as Node | null;
      if (!to || !target.contains(to)) hide();
    },
    true,
  );
  // From the keyboard too, as T3 Code's do, but not for a click's focus.
  document.addEventListener(
    "focusin",
    (e) => {
      const el = e.target as Element | null;
      if (el && el.matches(":focus-visible") && el.hasAttribute("title")) arm(el);
    },
    true,
  );
  document.addEventListener(
    "focusout",
    (e) => {
      if (target && e.target === target) hide();
    },
    true,
  );
  // Whatever is done next puts it away: a press, a key, a scroll.
  for (const kind of ["pointerdown", "keydown", "wheel"] as const) {
    document.addEventListener(kind, hide, true);
  }
  window.addEventListener("scroll", hide, true);
  window.addEventListener("blur", hide);
}
