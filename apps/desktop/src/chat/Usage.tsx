// How full this conversation's context is, as a ring in the composer, and
// behind it the account's limits.
//
// In the composer rather than the facts pane because that is where the
// question is asked: whether the next message will be the one that compacts is
// decided while it is being written, and the facts pane is not open then. The
// ring says it without being read -- hollow when the conversation is new,
// closing as it fills, yellow then red near the top -- and a click shows the
// figures.
//
// The context is the conversation's, from its own state; the limits are the
// account's, as this session's agent last reported them. Two conversations in
// one session have two contexts and one set of limits, and the popover says
// which is which.

import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import type { Usage } from "../gen/Usage";

/// Yellow from here, red from `BAD`: the same steps as the meters.
const WARN = 70;
const BAD = 90;

const tone = (pct: number): "" | "warn" | "bad" => (pct >= BAD ? "bad" : pct >= WARN ? "warn" : "");

export function ContextRing({
  percentage,
  size,
  usage,
}: {
  /// This conversation's, `null` before its first turn.
  percentage: number | null;
  size: number | null;
  /// The session's, for the account's windows.
  usage: Usage | null;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useId();

  const pct = percentage === null ? 0 : Math.max(0, Math.min(100, percentage));
  const r = 7;
  const c = 2 * Math.PI * r;
  const said = percentage === null ? "Context: not measured yet" : `Context ${Math.round(percentage)}% full`;

  return (
    <>
      <button
        ref={trigger}
        type="button"
        className={`chat-context ${tone(pct)}`}
        title={said}
        aria-label={said}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? panel : undefined}
        onClick={() => setOpen((o) => !o)}
      >
        <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
          <circle cx="9" cy="9" r={r} className="track" />
          {pct > 0 && (
            <circle
              cx="9"
              cy="9"
              r={r}
              className="fill"
              strokeDasharray={`${(c * pct) / 100} ${c}`}
              transform="rotate(-90 9 9)"
            />
          )}
        </svg>
      </button>
      {open && trigger.current && (
        <Popover
          id={panel}
          anchor={trigger.current}
          onClose={(refocus) => {
            setOpen(false);
            if (refocus) trigger.current?.focus();
          }}
        >
          <h5>This conversation</h5>
          {percentage === null ? (
            <p className="usage-note">Measured once the agent has answered.</p>
          ) : (
            <Meter label="Context" name="Context used" pct={percentage}>
              {size ? <span className="quiet"> of {tokens(size)}</span> : null}
            </Meter>
          )}
          <h5>This account</h5>
          {/* Empty until the agent has called the API, and for good on a
              provider that has no such windows. */}
          {usage && usage.windows.length > 0 ? (
            usage.windows.map((w) => (
              <Meter
                key={w.label}
                label={w.label}
                name={`${w.label} limit used`}
                pct={w.used_percentage}
                note={w.resets_at ? `resets ${resets(w.resets_at)}` : undefined}
              />
            ))
          ) : (
            <p className="usage-note">No limits reported yet.</p>
          )}
        </Popover>
      )}
    </>
  );
}

/// Over the ring and right-aligned to it: the composer is at the bottom of
/// the pane, so there is only room above. Closed by Escape, a click elsewhere,
/// or the window moving under it.
function Popover({
  id,
  anchor,
  onClose,
  children,
}: {
  id: string;
  anchor: HTMLElement;
  onClose: (refocus: boolean) => void;
  children: React.ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState<React.CSSProperties>({ opacity: 0 });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = anchor.getBoundingClientRect();
    const top = r.top - el.offsetHeight - 8;
    const left = Math.min(r.right - el.offsetWidth, window.innerWidth - el.offsetWidth - 6);
    setAt({ top: Math.max(6, top), left: Math.max(6, left) });
    el.focus({ preventScroll: true });
  }, [anchor]);

  useEffect(() => {
    const away = (e: Event) => {
      const t = e.target as Node;
      if (!ref.current?.contains(t) && !anchor.contains(t)) onClose(false);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose(true);
      }
    };
    const gone = () => onClose(false);
    window.addEventListener("mousedown", away, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("resize", gone);
    window.addEventListener("blur", gone);
    return () => {
      window.removeEventListener("mousedown", away, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("resize", gone);
      window.removeEventListener("blur", gone);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return createPortal(
    <div ref={ref} id={id} className="usage-popover" role="dialog" aria-label="Usage" tabIndex={-1} style={at}>
      {children}
    </div>,
    document.body,
  );
}

/// One percentage as a bar, its label before it and the figure after.
function Meter({
  label,
  name,
  pct,
  note,
  children,
}: {
  label: string;
  /// What a screen reader calls it.
  name: string;
  pct: number;
  /// Under the bar, quieter: when a window resets.
  note?: string;
  /// Said after the figure, quieter.
  children?: React.ReactNode;
}) {
  return (
    <div className="meter-row">
      <span className="meter-label">{label}</span>
      <span
        className={`meter ${tone(pct)}`}
        role="meter"
        aria-label={name}
        aria-valuenow={Math.round(pct)}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <span style={{ width: `${Math.min(100, Math.max(0, pct))}%` }} />
      </span>
      <span className="meter-value">
        {Math.round(pct)}%{children}
      </span>
      {note && <span className="meter-note">{note}</span>}
    </div>
  );
}

/// `200k`, `1M`: a window size as people say it.
function tokens(n: number): string {
  return n >= 1_000_000 ? `${+(n / 1_000_000).toFixed(1)}M` : `${Math.round(n / 1000)}k`;
}

/// When a window rolls over: the time if it is within a day, and the day as
/// well if not. Epoch seconds, as the status line sends it.
function resets(at: number): string {
  const when = new Date(at * 1000);
  const time = when.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (when.getTime() - Date.now() < 24 * 3600 * 1000) return `at ${time}`;
  return `${when.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })}, ${time}`;
}
