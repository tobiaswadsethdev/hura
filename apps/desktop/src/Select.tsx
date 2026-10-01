// A choice from a short list, drawn by this window rather than the platform.
//
// The native `<select>` was two problems. Its closed state is WebKit's own
// control, the loudest thing on whatever it sits in -- which is why the ticket
// cards had already flattened theirs. And its *open* state is not WebKit's at
// all but the toolkit's: on Linux the popup is a GTK menu drawn in the desktop
// theme, so it came up light while inheriting this window's near-white text,
// and read as white on white. No stylesheet reaches that popup, so the answer
// is not to have one.
//
// So this is a button and a listbox in the context menu's surface: the same
// panel, the same row height, the current choice ticked. A hint column carries
// what the native control could only append to the label -- a policy's summary
// beside its name, dim, instead of one long line.
//
// The keyboard does what a select's does: Enter, Space or the arrows open it,
// the arrows and Home/End move, typing jumps to the first match, Enter picks
// and Escape puts it back as it was.

import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { Chevron, Chosen } from "./icons";

export type Option = {
  value: string;
  label: string;
  /// Dim, right-aligned: what the label alone does not say.
  hint?: string;
};

export function Select({
  value,
  options,
  onChange,
  className,
  title,
  disabled,
  "aria-label": ariaLabel,
}: {
  value: string;
  options: Option[];
  onChange: (value: string) => void;
  className?: string;
  title?: string;
  disabled?: boolean;
  "aria-label"?: string;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const current = options.find((o) => o.value === value);
  const list = useId();

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
      e.preventDefault();
      setOpen(true);
    }
  };

  return (
    <>
      <button
        ref={trigger}
        type="button"
        className={`select-trigger ${className ?? ""}`}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? list : undefined}
        aria-label={ariaLabel}
        title={title ?? current?.hint}
        disabled={disabled}
        onClick={() => setOpen((o) => !o)}
        onKeyDown={onKeyDown}
      >
        <span className="select-value">{current?.label ?? value}</span>
        <Chevron open className="select-chevron" />
      </button>
      {open && trigger.current && (
        <List
          id={list}
          anchor={trigger.current}
          options={options}
          value={value}
          onPick={(v) => {
            setOpen(false);
            trigger.current?.focus();
            if (v !== value) onChange(v);
          }}
          onClose={(refocus) => {
            setOpen(false);
            if (refocus) trigger.current?.focus();
          }}
        />
      )}
    </>
  );
}

function List({
  id,
  anchor,
  options,
  value,
  onPick,
  onClose,
}: {
  id: string;
  anchor: HTMLElement;
  options: Option[];
  value: string;
  onPick: (value: string) => void;
  onClose: (refocus: boolean) => void;
}) {
  const ref = useRef<HTMLUListElement>(null);
  const [active, setActive] = useState(() => Math.max(0, options.findIndex((o) => o.value === value)));
  // Transparent rather than hidden until it is placed: a hidden element cannot
  // take the focus, and the list has to have it for the keyboard to work.
  const [at, setAt] = useState<React.CSSProperties>({ opacity: 0 });
  const typed = useRef({ text: "", at: 0 });

  // Under the trigger and at least as wide as it, flipped above when there is
  // no room below -- a select at the bottom of a dialog opens upwards.
  useLayoutEffect(() => {
    const r = anchor.getBoundingClientRect();
    const el = ref.current;
    if (!el) return;
    const height = el.offsetHeight;
    const width = Math.max(r.width, el.offsetWidth);
    const below = window.innerHeight - r.bottom;
    const top = below >= height + 8 || below > r.top ? r.bottom + 4 : r.top - height - 4;
    const left = Math.min(r.left, window.innerWidth - width - 6);
    setAt({ top, left: Math.max(6, left), minWidth: r.width });
    el.focus({ preventScroll: true });
  }, [anchor]);

  useEffect(() => {
    const away = (e: Event) => {
      const t = e.target as Node;
      if (!ref.current?.contains(t) && !anchor.contains(t)) onClose(false);
    };
    const scrolled = (e: Event) => {
      if (!ref.current?.contains(e.target as Node)) onClose(false);
    };
    const gone = () => onClose(false);
    window.addEventListener("mousedown", away, true);
    window.addEventListener("scroll", scrolled, true);
    window.addEventListener("resize", gone);
    window.addEventListener("blur", gone);
    return () => {
      window.removeEventListener("mousedown", away, true);
      window.removeEventListener("scroll", scrolled, true);
      window.removeEventListener("resize", gone);
      window.removeEventListener("blur", gone);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    ref.current?.children[active]?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const last = options.length - 1;
    const move = (i: number) => {
      e.preventDefault();
      setActive(Math.min(last, Math.max(0, i)));
    };
    if (e.key === "ArrowDown") move(active + 1);
    else if (e.key === "ArrowUp") move(active - 1);
    else if (e.key === "Home") move(0);
    else if (e.key === "End") move(last);
    else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onPick(options[active].value);
    } else if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      onClose(true);
    } else if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      // Type-ahead, the way a select does it: letters typed within a moment
      // of each other are one prefix.
      const now = Date.now();
      typed.current = {
        text: (now - typed.current.at < 700 ? typed.current.text : "") + e.key.toLowerCase(),
        at: now,
      };
      const hit = options.findIndex((o) => o.label.toLowerCase().startsWith(typed.current.text));
      if (hit >= 0) setActive(hit);
    }
  };

  return createPortal(
    <ul
      ref={ref}
      id={id}
      className="select-list"
      role="listbox"
      tabIndex={-1}
      aria-activedescendant={`${id}-${active}`}
      style={at}
      onKeyDown={onKeyDown}
    >
      {options.map((o, i) => (
        <li
          key={o.value}
          id={`${id}-${i}`}
          role="option"
          aria-selected={o.value === value}
          className={`${i === active ? "active" : ""} ${o.value === value ? "chosen" : ""}`}
          onMouseEnter={() => setActive(i)}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => onPick(o.value)}
        >
          <span className="select-tick" aria-hidden>
            {o.value === value && <Chosen />}
          </span>
          <span className="select-label">{o.label}</span>
          {o.hint && <span className="select-hint">{o.hint}</span>}
        </li>
      ))}
    </ul>,
    document.body,
  );
}
