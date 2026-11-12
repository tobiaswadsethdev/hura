// How long something has been going on: a turn, a tool call, a subagent.
//
// Every start is an entry's `at`, from the sandbox's clock, and every now is
// this machine's. The two agree to within the second this counts in on any
// machine that keeps time, and a clock behind the sandbox's reads as nothing
// rather than as a negative.

import { createContext, useEffect, useState } from "react";

/// A span the way the transcript says one: `42s`, `3m 5s`, `1h 12m`.
export function since(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

/// Now, once a second while `running`.
export function useNow(running: boolean): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!running) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [running]);
  return now;
}

/// How long since `start`, counting. Its own component, so the second's tick
/// draws this and not the conversation around it. Hidden from a screen
/// reader, which would read it out every second.
export function Since({ start }: { start: number }) {
  const now = useNow(true);
  return (
    <span className="elapsed" aria-hidden>
      {since(now - start)}
    </span>
  );
}

/// Whether the conversation is mid-turn. A call the transcript leaves without
/// a result once the turn is over (the host stopped under it) is not still
/// running, and its clock stops with the turn rather than counting for ever.
export const Ticking = createContext(false);
