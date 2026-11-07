// The subagents a conversation has started, and one of them followed.
//
// A subagent's work is folded into the card of the call that started it,
// which is right for one that runs inside a turn and wrong for one in the
// background: its call returns at once, the turn goes on and ends, and the
// agent keeps working inside a closed card that has scrolled away. So the
// agents get a strip of their own above the box, running ones first, and one
// opens beside the conversation, where it is followed as it works.

import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { Agent as AgentGlyph, Chevron, Chosen, Close, Working } from "../icons";
import { Markdown } from "../Markdown";
import { Items } from "./Items";
import type { Agent } from "./transcript";

/// What a running task last said about itself: an SDK `task_progress`.
export type TaskProgress = {
  task_id: string;
  summary?: string;
  last_tool_name?: string;
  usage?: { total_tokens: number; tool_uses: number; duration_ms: number };
};

function since(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

function tokens(n: number): string {
  return n < 1000 ? `${n}` : `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
}

/// Now, once a second while `running`: how long a running agent has been at it.
function useNow(running: boolean): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [running]);
  return now;
}

function StatusGlyph({ agent }: { agent: Agent }) {
  switch (agent.status) {
    case "running":
      return <Working aria-label="running" />;
    case "completed":
      return <Chosen aria-label="done" />;
    default:
      return <Close aria-label={agent.status} />;
  }
}

export function AgentsBar({
  agents,
  open,
  onOpen,
}: {
  agents: Agent[];
  open: string | null;
  onOpen: (id: string | null) => void;
}) {
  if (!agents.length) return null;
  // Running ones first, then the most recently finished.
  const shown = [...agents].sort(
    (a, b) =>
      Number(b.status === "running") - Number(a.status === "running") ||
      (b.ended ?? b.started) - (a.ended ?? a.started),
  );
  const running = agents.filter((a) => a.status === "running").length;
  return (
    <div className="chat-agents" role="list" aria-label="subagents">
      <span className="chat-agents-count" title="subagents in this conversation">
        <AgentGlyph />
        {running ? `${running} running` : `${agents.length}`}
      </span>
      {shown.map((a) => (
        <button
          key={a.id}
          role="listitem"
          className={`chat-agent-chip ${a.status}${open === a.id ? " on" : ""}`}
          title={[a.type, a.background ? "in the background" : null, a.status].filter(Boolean).join(" · ")}
          onClick={() => onOpen(open === a.id ? null : a.id)}
        >
          <StatusGlyph agent={a} />
          <span>{a.description}</span>
        </button>
      ))}
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
    <aside className="chat-agent-panel" aria-label={`subagent: ${agent.description}`}>
      <header>
        <StatusGlyph agent={agent} />
        <h3 title={agent.description}>{agent.description}</h3>
        <button className="chat-agent-close" title="close" onClick={onClose}>
          <Close aria-label="close" />
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
              <Chevron open={instructions} />
              instructions
            </button>
            {instructions && (
              <div className="body">
                <Markdown text={agent.prompt} />
              </div>
            )}
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
