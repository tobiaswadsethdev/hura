// The work a conversation has going on beside it: the subagents it started
// and the commands it left running in the background, and one of them
// followed.
//
// A subagent's work is folded into the card of the call that started it,
// which is right for one that runs inside a turn and wrong for one in the
// background: its call returns at once, the turn goes on and ends, and the
// agent keeps working inside a closed card that has scrolled away. A command
// run in the background is the same, and worse: its output is a file in the
// sandbox the conversation never shows. So both get a strip of their own above
// the box, running ones first, and one opens beside the conversation, where an
// agent is followed as it works and a command's output as it is written.

import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { Fold } from "../Fold";
import { messageOf } from "../api";
import type { TailText } from "../gen/TailText";
import { Agent as AgentGlyph, Chosen, Close, Shell, Twisty, Working } from "../icons";
import { Markdown } from "../Markdown";
import { since, useNow } from "./elapsed";
import { Items } from "./Items";
import type { Agent, Command } from "./transcript";

/// What a running task last said about itself: an SDK `task_progress`.
export type TaskProgress = {
  task_id: string;
  summary?: string;
  last_tool_name?: string;
  usage?: { total_tokens: number; tool_uses: number; duration_ms: number };
};

/// A row that scrolls across, moved the ways a mouse can: its wheel turned
/// up and down (not only with Shift, as the engine has it), and dragged. A
/// drag that moved is not also a click on the chip it started on.
function useSideScroll(ref: React.RefObject<HTMLElement | null>, shown: boolean) {
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const wheel = (e: WheelEvent) => {
      if (el.scrollWidth <= el.clientWidth || e.shiftKey) return;
      if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
      const step = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? e.deltaY * 16 : e.deltaY;
      const before = el.scrollLeft;
      el.scrollLeft += step;
      if (el.scrollLeft !== before) e.preventDefault();
    };
    let from: { x: number; left: number; id: number } | null = null;
    let dragged = false;
    const down = (e: PointerEvent) => {
      dragged = false;
      if (e.pointerType !== "mouse" || e.button !== 0 || el.scrollWidth <= el.clientWidth) return;
      from = { x: e.clientX, left: el.scrollLeft, id: e.pointerId };
    };
    const move = (e: PointerEvent) => {
      if (!from || e.pointerId !== from.id) return;
      const dx = e.clientX - from.x;
      if (!dragged && Math.abs(dx) < 4) return;
      if (!dragged) {
        dragged = true;
        el.setPointerCapture(e.pointerId);
        el.classList.add("dragging");
      }
      el.scrollLeft = from.left - dx;
    };
    const up = () => {
      from = null;
      el.classList.remove("dragging");
    };
    const click = (e: MouseEvent) => {
      if (!dragged) return;
      dragged = false;
      e.stopPropagation();
      e.preventDefault();
    };
    el.addEventListener("wheel", wheel, { passive: false });
    el.addEventListener("pointerdown", down);
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
    el.addEventListener("click", click, true);
    return () => {
      el.removeEventListener("wheel", wheel);
      el.removeEventListener("pointerdown", down);
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      el.removeEventListener("click", click, true);
    };
  }, [ref, shown]);
}

function tokens(n: number): string {
  return n < 1000 ? `${n}` : `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
}

function StatusGlyph({ status }: { status: Agent["status"] }) {
  switch (status) {
    case "running":
      return <Working aria-label="running" />;
    case "completed":
      return <Chosen aria-label="done" />;
    default:
      return <Close aria-label={status} />;
  }
}

export function AgentsBar({
  agents,
  commands,
  open,
  onOpen,
}: {
  agents: Agent[];
  commands: Command[];
  open: string | null;
  onOpen: (id: string | null) => void;
}) {
  const row = useRef<HTMLDivElement>(null);
  const any = agents.length + commands.length > 0;
  useSideScroll(row, any);
  if (!any) return null;
  // Running ones first, then the most recently finished, agents and commands
  // together: what is going on beside the conversation, whichever it is.
  const work = [
    ...agents.map((a) => ({ kind: "agent" as const, id: a.id, status: a.status, at: a.ended ?? a.started, agent: a })),
    ...commands.map((c) => ({ kind: "command" as const, id: c.id, status: c.status, at: c.ended ?? c.started, command: c })),
  ].sort((x, y) => Number(y.status === "running") - Number(x.status === "running") || y.at - x.at);
  const running = work.filter((w) => w.status === "running").length;
  return (
    <div ref={row} className="chat-agents" role="list" aria-label="Background work">
      <span className="chat-agents-count" title="Subagents and background commands in this conversation">
        <AgentGlyph />
        {running ? `${running} running` : `${work.length}`}
      </span>
      {work.map((w) =>
        w.kind === "agent" ? (
          <button
            key={w.id}
            role="listitem"
            className={`chat-agent-chip ${w.status}${open === w.id ? " on" : ""}`}
            title={[w.agent.type, w.agent.background ? "in the background" : null, w.status].filter(Boolean).join(" · ")}
            onClick={() => onOpen(open === w.id ? null : w.id)}
          >
            <StatusGlyph status={w.status} />
            <span>{w.agent.description}</span>
          </button>
        ) : (
          <button
            key={w.id}
            role="listitem"
            className={`chat-agent-chip ${w.status}${open === w.id ? " on" : ""}`}
            title={[w.command.command, "a command in the background", w.status].filter(Boolean).join(" · ")}
            onClick={() => onOpen(open === w.id ? null : w.id)}
          >
            <StatusGlyph status={w.status} />
            <Shell aria-hidden className="chat-agent-kind" />
            <span>{w.command.description || w.command.command || "Command"}</span>
          </button>
        ),
      )}
    </div>
  );
}

export function AgentPanel({
  agent,
  progress,
  onClose,
}: {
  agent: Agent;
  progress: TaskProgress | undefined;
  onClose: () => void;
}) {
  const running = agent.status === "running";
  const now = useNow(running);
  const [instructions, setInstructions] = useState(false);
  const usage = progress?.usage ?? agent.usage;
  const children = agent.call?.children ?? [];
  const facts = [
    agent.type,
    agent.background ? "background" : null,
    running ? "running" : agent.status === "completed" ? "done" : agent.status,
    since((agent.ended ?? now) - agent.started),
    usage ? `${usage.tool_uses} tool use${usage.tool_uses === 1 ? "" : "s"}` : null,
    usage ? `${tokens(usage.total_tokens)} tokens` : null,
  ].filter(Boolean);

  // Kept at the bottom while it is at the bottom, as the conversation is.
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [children.length, agent.call?.result, progress?.summary]);

  return (
    <aside className="chat-agent-panel" aria-label={`Subagent: ${agent.description}`}>
      <header>
        <StatusGlyph status={agent.status} />
        <h3 title={agent.description}>{agent.description}</h3>
        <button className="chat-agent-close" title="Close" onClick={onClose}>
          <Close aria-label="Close" />
        </button>
      </header>
      <p className="chat-agent-facts">{facts.join(" · ")}</p>
      {running && progress?.summary && <p className="chat-agent-now">{progress.summary}</p>}
      <div
        className="chat-agent-scroll"
        ref={scroller}
        onScroll={(e) => {
          const el = e.currentTarget;
          pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
      >
        {agent.prompt && (
          <div className="chat-thinking">
            <button aria-expanded={instructions} onClick={() => setInstructions(!instructions)}>
              <Twisty open={instructions} />
              Instructions
            </button>
            <Fold open={instructions}>
              <div className="body">
                <Markdown text={agent.prompt} />
              </div>
            </Fold>
          </div>
        )}
        <div className="chat-log">
          <Items items={children} />
          {/* A foreground agent's report is its call's result; a background
              one's arrives as a SubagentHandback call among its own. */}
          {!agent.background && agent.call?.result && !agent.call.result.isError && (
            <Markdown text={agent.call.result.text} />
          )}
          {running && (
            <p className="chat-working" role="status">
              <Working size={22} />
              {progress?.last_tool_name ?? "working"}
            </p>
          )}
          {!running && children.length === 0 && <p className="chat-note dim">Nothing it did came through.</p>}
        </div>
      </div>
    </aside>
  );
}

/// Colour and cursor codes, which a log written for a terminal is full of and
/// a page cannot draw.
const ESCAPES = /\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07|\x1b\\)/g;

/// How often a running command's output is read again.
const TAIL_MS = 2_000;

/// A command left running in the background, followed: what it is, how long
/// it has run, and the end of what it has written, read again every few
/// seconds while it runs.
export function CommandPanel({
  command,
  readTail,
  onClose,
}: {
  command: Command;
  readTail: (path: string) => Promise<TailText>;
  onClose: () => void;
}) {
  const running = command.status === "running";
  const now = useNow(running);
  const [tail, setTail] = useState<TailText | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const path = command.output;
    if (!path) return;
    let live = true;
    const read = () =>
      readTail(path).then(
        (t) => {
          if (!live) return;
          setTail(t);
          setError(null);
        },
        (e) => live && setError(messageOf(e)),
      );
    void read();
    if (!running) {
      return () => {
        live = false;
      };
    }
    const timer = setInterval(read, TAIL_MS);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [command.output, running, readTail]);

  const facts = [
    "in the background",
    running ? "running" : command.status === "completed" ? "done" : command.status,
    since((command.ended ?? now) - command.started),
    command.taskId,
  ].filter(Boolean);

  // Kept at the bottom while it is at the bottom, as a terminal is.
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [tail?.text]);

  const title = command.description || "Background command";
  return (
    <aside className="chat-agent-panel" aria-label={`Background command: ${title}`}>
      <header>
        <StatusGlyph status={command.status} />
        <h3 title={title}>{title}</h3>
        <button className="chat-agent-close" title="Close" onClick={onClose}>
          <Close aria-label="Close" />
        </button>
      </header>
      <p className="chat-agent-facts">{facts.join(" · ")}</p>
      {command.summary && <p className="chat-agent-now">{command.summary}</p>}
      <div
        className="chat-agent-scroll chat-command-panel"
        ref={scroller}
        onScroll={(e) => {
          const el = e.currentTarget;
          pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
      >
        {command.command && <pre className="chat-command">{command.command}</pre>}
        {tail ? (
          tail.text ? (
            <pre className="chat-output chat-command-output">
              {tail.cut ? "…\n" : ""}
              {tail.text.replace(ESCAPES, "")}
            </pre>
          ) : (
            <p className="chat-note dim">Nothing written yet.</p>
          )
        ) : error ? (
          <p className="chat-note dim">{error}</p>
        ) : command.output ? (
          <p className="chat-note dim">Reading its output…</p>
        ) : (
          <p className="chat-note dim">The transcript does not say where its output goes.</p>
        )}
      </div>
    </aside>
  );
}
