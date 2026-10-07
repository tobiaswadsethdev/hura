// A conversation with the agent: what it said and did, and what you say back.
//
// Over a chat channel, which is the host in the sandbox at one end and this at
// the other, with `hurad` in between. The channel opens with the transcript so
// far and then carries everything that happens: entries as they are kept, the
// reply as it is written, the state whenever it changes. Commands go back on
// the same channel.
//
// The transcript is the host's, not this pane's. It survives the window
// closing, a second window opening and the server restarting, which is why a
// channel that drops is opened again from the last entry it saw rather than
// the pane keeping anything of its own.

import { useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState } from "react";

import { Empty } from "../Empty";
import type { ChatCommand } from "../gen/ChatCommand";
import type { ChatEntry } from "../gen/ChatEntry";
import type { ChatFrame } from "../gen/ChatFrame";
import type { ChatState } from "../gen/ChatState";
import { Chat as ChatGlyph, Send, Stop, Working } from "../icons";
import { Markdown } from "../Markdown";
import type { PartialMessage, ToolProgress } from "../sdk";
import { Select } from "../Select";
import { chat, close, nextChannelId, open } from "../stream";
import { AskCard } from "../chat/Ask";
import { Items } from "../chat/Items";
import { arrived, build } from "../chat/transcript";

/// The reply being written, from the stream events: the main thread's
/// current message, block by block.
type Live = { id: string; blocks: { type: string; text: string; name?: string }[] } | null;

type View = {
  entries: ChatEntry[];
  /// The last entry this pane has, which is what a reopened channel asks from.
  seq: number;
  state: ChatState | null;
  live: Live;
  /// How long each running tool has been running, by tool use id.
  progress: Map<string, number>;
  /// Why there is nothing yet, while the session is still being prepared.
  notice: string | null;
  /// Why the channel is down, while it is.
  down: string | null;
  error: string | null;
};

const EMPTY: View = {
  entries: [],
  seq: 0,
  state: null,
  live: null,
  progress: new Map(),
  notice: null,
  down: null,
  error: null,
};

type Action = { type: "frame"; frame: ChatFrame } | { type: "down"; reason: string };

function reduce(v: View, a: Action): View {
  if (a.type === "down") return { ...v, down: a.reason, live: null };
  const f = a.frame;
  switch (f.t) {
    case "replay": {
      // A reopened channel sends what it missed. Anything already here is
      // left as it is, so a replay that overlaps cannot draw an entry twice.
      const fresh = f.entries.filter((e) => e.seq > v.seq);
      return { ...v, entries: [...v.entries, ...fresh], seq: Math.max(v.seq, f.seq), notice: null, down: null };
    }
    case "entry": {
      if (f.entry.seq <= v.seq) return v;
      const ev = f.entry.ev;
      const done = ev.kind === "sdk" && (ev.msg as { type?: string }).type === "result";
      return {
        ...v,
        entries: [...v.entries, f.entry],
        seq: f.entry.seq,
        live: done ? null : v.live,
        progress: done ? new Map() : v.progress,
      };
    }
    case "partial":
      return { ...v, live: stream(v.live, f.ev as PartialMessage) };
    case "progress": {
      const p = f.ev as ToolProgress;
      const progress = new Map(v.progress);
      progress.set(p.tool_use_id, p.elapsed_time_seconds);
      return { ...v, progress };
    }
    case "state": {
      const idle = f.state.activity === "idle";
      return {
        ...v,
        state: f.state,
        live: idle ? null : v.live,
        progress: idle ? new Map() : v.progress,
        notice: null,
        down: null,
      };
    }
    case "notice":
      return { ...v, notice: f.text };
    case "error":
      return { ...v, error: f.message };
  }
}

/// One stream event folded into the reply being written.
function stream(live: Live, msg: PartialMessage): Live {
  const ev = msg.event;
  switch (ev.type) {
    case "message_start":
      return { id: ev.message.id, blocks: [] };
    case "content_block_start": {
      if (!live) return live;
      const blocks = [...live.blocks];
      blocks[ev.index] = { type: ev.content_block.type, text: "", name: ev.content_block.name };
      return { ...live, blocks };
    }
    case "content_block_delta": {
      const block = live?.blocks[ev.index];
      if (!live || !block) return live;
      const d = ev.delta;
      const more = d.type === "text_delta" && "text" in d ? d.text : d.type === "thinking_delta" && "thinking" in d ? d.thinking : "";
      if (!more) return live;
      const blocks = [...live.blocks];
      blocks[ev.index] = { ...block, text: block.text + more };
      return { ...live, blocks };
    }
  }
  return live;
}

/// The reply as it is being written: the blocks of the current message that
/// have not yet arrived whole as entries.
function Writing({ live, entries }: { live: Live; entries: ChatEntry[] }) {
  if (!live) return null;
  const done = arrived(entries, live.id);
  const blocks = live.blocks.slice(done).filter(Boolean);
  if (!blocks.length) return null;
  return (
    <>
      {blocks.map((b, i) =>
        b.type === "text" ? (
          <Markdown key={i} text={b.text} />
        ) : b.type === "thinking" ? (
          <p key={i} className="chat-note dim">
            thinking…
          </p>
        ) : b.type === "tool_use" ? (
          <p key={i} className="chat-note dim">
            {b.name}…
          </p>
        ) : null,
      )}
    </>
  );
}

/// Claude Code's permission modes, in the words the window uses for them.
const MODES = [
  { value: "auto", label: "auto", hint: "a classifier decides what needs asking" },
  { value: "default", label: "ask", hint: "ask before edits and commands" },
  { value: "acceptEdits", label: "accept edits", hint: "edit without asking, ask for the rest" },
  { value: "plan", label: "plan", hint: "read and plan, change nothing" },
];

function Composer({
  state,
  onCommand,
}: {
  state: ChatState | null;
  onCommand: (command: ChatCommand) => void;
}) {
  const [text, setText] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);
  const working = state?.activity === "running" || state?.activity === "waiting";

  // Grows with what is typed, up to a third of the pane, and no further.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 240)}px`;
  }, [text]);

  // A slash command, offered while one is being typed.
  const typing = /^\/(\S*)$/.exec(text);
  const offered = typing
    ? (state?.commands ?? []).filter((c) => c.startsWith(typing[1])).slice(0, 8)
    : [];

  const send = () => {
    const t = text.trim();
    if (!t) return;
    onCommand({ op: "send", text: t });
    setText("");
  };

  const models = state?.models ?? [];
  return (
    <div className="chat-composer">
      {offered.length > 0 && (
        <ul className="chat-commands" role="listbox">
          {offered.map((c) => (
            <li key={c}>
              <button
                onMouseDown={(e) => {
                  e.preventDefault();
                  setText(`/${c} `);
                  box.current?.focus();
                }}
              >
                /{c}
              </button>
            </li>
          ))}
        </ul>
      )}
      <textarea
        ref={box}
        rows={1}
        value={text}
        placeholder={working ? "queue a message for when this turn is done" : "tell the agent what to do"}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing) return;
          if (e.key === "Tab" && offered.length) {
            e.preventDefault();
            setText(`/${offered[0]} `);
          } else if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            send();
          } else if (e.key === "Escape" && working) {
            e.preventDefault();
            onCommand({ op: "interrupt" });
          }
        }}
      />
      <div className="chat-controls">
        <Select
          className="chat-select"
          aria-label="permission mode"
          title="permission mode"
          value={state?.mode ?? "auto"}
          options={MODES}
          onChange={(mode) => onCommand({ op: "mode", mode })}
        />
        {models.length > 0 && (
          <Select
            className="chat-select"
            aria-label="model"
            title="model"
            value={state?.model ?? models[0].value}
            options={[
              // What Claude Code says it is running, which is a full model id
              // rather than one of the names it offers to switch to, so it is
              // its own entry at the top rather than a guess at which alias it
              // came from.
              ...(state?.model && !models.some((m) => m.value === state.model)
                ? [{ value: state.model, label: state.model, hint: "running now" }]
                : []),
              ...models.map((m) => ({ value: m.value, label: m.display_name, hint: m.description })),
            ]}
            onChange={(model) => onCommand({ op: "model", model })}
          />
        )}
        <span className="chat-facts">
          {state?.status === "compacting" && <span>compacting</span>}
          {state?.context_percentage != null && <span title="context used">ctx {Math.round(state.context_percentage)}%</span>}
          {state?.cost_usd != null && state.cost_usd > 0 && <span title="spent in this conversation">${state.cost_usd.toFixed(2)}</span>}
        </span>
        {working ? (
          <button className="chat-stop" title="stop this turn (Esc)" onClick={() => onCommand({ op: "interrupt" })}>
            <Stop aria-label="stop" />
          </button>
        ) : null}
        <button className="go chat-send" title="send (Enter)" disabled={!text.trim()} onClick={send}>
          <Send aria-label="send" />
        </button>
      </div>
    </div>
  );
}

export function ChatPane({ server, name, conv }: { server: string; name: string; conv: string }) {
  const [view, dispatch] = useReducer(reduce, EMPTY);
  const channel = useRef(0);
  const seq = useRef(0);
  seq.current = view.seq;

  useEffect(() => {
    let live = true;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let wait = 2_000;

    const connect = () => {
      const id = nextChannelId();
      channel.current = id;
      void open(server, id, { kind: "chat", session: name, conv, since: seq.current }, (frame) => {
        if (!live) return;
        if (frame.is === "chat") {
          wait = 2_000;
          dispatch({ type: "frame", frame: frame.frame });
        } else if (frame.is === "closed") {
          // Opened again from the last entry, after a pause that grows while
          // nothing answers: a session being destroyed should not be asked
          // about twice a second until the tab goes.
          dispatch({ type: "down", reason: frame.reason ?? "disconnected" });
          retry = setTimeout(connect, wait);
          wait = Math.min(wait * 2, 30_000);
        }
      }).catch((e) => {
        if (!live) return;
        dispatch({ type: "down", reason: String(e) });
        retry = setTimeout(connect, wait);
        wait = Math.min(wait * 2, 30_000);
      });
    };
    connect();

    return () => {
      live = false;
      clearTimeout(retry);
      void close(channel.current);
    };
  }, [server, name, conv]);

  const command = (c: ChatCommand) => {
    chat.command(channel.current, c).catch(() => {});
  };

  const items = useMemo(() => build(view.entries), [view.entries]);

  // Kept at the bottom while it is at the bottom: a reply being written
  // scrolls with itself, and someone who has scrolled up to read is left
  // where they are.
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [view.entries, view.live, view.state?.pending.length]);

  const pending = view.state?.pending ?? [];
  const nothing = view.entries.length === 0 && !view.live;

  return (
    <div className="chat">
      <div
        className="chat-scroll"
        ref={scroller}
        onScroll={(e) => {
          const el = e.currentTarget;
          pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
      >
        <div className="chat-log">
          {nothing &&
            (view.notice ? (
              <Empty icon={Working} note={view.notice} />
            ) : view.state ? (
              <Empty icon={ChatGlyph} />
            ) : null)}
          <Items items={items} progress={view.progress} />
          <Writing live={view.live} entries={view.entries} />
          {/* Under whatever is being written, for the whole turn, so a reply
              that has paused between two tool calls still says it is not
              finished. The word only while there is nothing else to look at. */}
          {view.state?.activity === "running" && pending.length === 0 && (
            <p className="chat-working" role="status" aria-label="working">
              <Working size={22} />
              {!view.live && "working"}
            </p>
          )}
          {pending.map((ask) => (
            <AskCard key={ask.id} ask={ask} onAnswer={(decision) => command({ op: "answer", id: ask.id, decision })} />
          ))}
        </div>
      </div>
      {(view.down || view.error) && <p className="error chat-banner">{view.down ?? view.error}</p>}
      <Composer state={view.state} onCommand={command} />
    </div>
  );
}
