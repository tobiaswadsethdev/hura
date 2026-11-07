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
import { Attach, Chat as ChatGlyph, Close, Send, Stop, Working } from "../icons";
import { api } from "../api";
import { Markdown, Pictures, type ReadPicture } from "../Markdown";
import type { PartialMessage, ToolProgress } from "../sdk";
import { Select } from "../Select";
import { chat, close, nextChannelId, open } from "../stream";
import { AskCard } from "../chat/Ask";
import { type Attached, isPicture, read, size, UPLOAD_BYTES } from "../chat/files";
import { Items, OpenAgent } from "../chat/Items";
import { arrived, build } from "../chat/transcript";
import { AgentPanel, AgentsBar, type TaskProgress } from "../chat/Agents";

/// The reply being written, from the stream events: the main thread's
/// current message, block by block.
type Live = { id: string; blocks: { type: string; text: string; name?: string }[] } | null;

type View = {
  entries: ChatEntry[];
  /// The last entry this pane has, which is what a reopened channel asks from.
  seq: number;
  /// The history is all here. A long one replays in several frames, oldest
  /// first, and the host sends the state after the last of them -- so the
  /// first state says so. Until then nothing is drawn: drawing each frame as
  /// it came scrolled the whole conversation past on the way to the newest.
  caughtUp: boolean;
  state: ChatState | null;
  live: Live;
  /// How long each running tool has been running, by tool use id.
  progress: Map<string, number>;
  /// What each running subagent last said it was doing, by task id.
  tasks: Map<string, TaskProgress>;
  /// Why there is nothing yet, while the session is still being prepared.
  notice: string | null;
  /// Why the channel is down, while it is.
  down: string | null;
  error: string | null;
};

const EMPTY: View = {
  entries: [],
  seq: 0,
  caughtUp: false,
  state: null,
  live: null,
  progress: new Map(),
  tasks: new Map(),
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
    case "task": {
      const t = f.ev as TaskProgress;
      const tasks = new Map(v.tasks);
      tasks.set(t.task_id, t);
      return { ...v, tasks };
    }
    case "state": {
      const idle = f.state.activity === "idle";
      return {
        ...v,
        state: f.state,
        caughtUp: true,
        live: idle ? null : v.live,
        progress: idle ? new Map() : v.progress,
        notice: null,
        down: null,
      };
    }
    case "notice":
      return { ...v, notice: f.text, caughtUp: true };
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
  { value: "auto", label: "auto", hint: "decides what to ask" },
  { value: "default", label: "ask", hint: "before edits and commands" },
  { value: "acceptEdits", label: "accept edits", hint: "asks for the rest" },
  { value: "plan", label: "plan", hint: "changes nothing" },
];

/// A model id as a person would say it: `claude-opus-5-5[1m]` is Opus 5.5,
/// with the million-token context. Anything that does not look like an id is
/// left as it is, since an alias like `opus` is already a word.
export function modelName(id: string): string {
  const m = /^claude-([a-z]+)-(\d+)(?:-(\d{1,2}))?(?:-\d{8})?(\[1m\])?$/.exec(id);
  if (!m) return id;
  const [, family, major, minor, wide] = m;
  const name = `${family[0].toUpperCase()}${family.slice(1)} ${major}${minor ? `.${minor}` : ""}`;
  return wide ? `${name} (1M)` : name;
}

/// A reasoning effort as Claude Code's `/effort` says it.
function effortName(e: string): string {
  return e === "xhigh" ? "extra high" : e;
}

/// The files waiting in the composer: a picture as itself, anything else as
/// its name, each with a way out of the message.
function AttachedFiles({ files, onRemove }: { files: Attached[]; onRemove: (key: string) => void }) {
  if (!files.length) return null;
  return (
    <ul className="chat-attached">
      {files.map((f) => (
        <li key={f.key} title={`${f.name}, ${size(f.size)}`}>
          {isPicture(f.mime) ? (
            <img src={`data:${f.mime};base64,${f.data}`} alt={f.name} />
          ) : (
            <span className="chat-file">
              <Attach />
              {f.name}
            </span>
          )}
          <button className="chat-unattach" title="take it out of the message" onClick={() => onRemove(f.key)}>
            <Close aria-label={`remove ${f.name}`} />
          </button>
        </li>
      ))}
    </ul>
  );
}

function Composer({
  state,
  onCommand,
  files,
  onAttach,
  onRemove,
  onSent,
}: {
  state: ChatState | null;
  onCommand: (command: ChatCommand) => void;
  files: Attached[];
  onAttach: (files: File[]) => void;
  onRemove: (key: string) => void;
  onSent: () => void;
}) {
  const [text, setText] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  /// What was just sent, and when. WebKit commits a pending autocorrection or
  /// inline prediction into the box after Enter has emptied it, which put the
  /// message straight back; for a moment after a send, that is not typing.
  const sent = useRef<{ text: string; at: number } | null>(null);
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
    if (!t && !files.length) return;
    onCommand({
      op: "send",
      text: t,
      files: files.length ? files.map(({ name, mime, data }) => ({ name, mime, data })) : undefined,
    });
    sent.current = { text: t, at: Date.now() };
    setText("");
    onSent();
  };

  const models = state?.models ?? [];
  const model = state?.model ?? null;
  // The efforts the model in use takes, found by its alias or the id the
  // alias stands for; the default's while nothing has been chosen.
  const efforts =
    (model ? models.find((m) => m.value === model || m.resolved === model) : models[0])?.efforts ?? [];
  const effort = state?.effort ?? null;
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
      {/* One box: what you write, and under it, in the same frame, what it
          will run under. The controls are words until pointed at, because
          they are read far more often than they are changed. A click on the
          box's own padding is a click on the text. */}
      <div className="chat-box" onClick={(e) => e.target === e.currentTarget && box.current?.focus()}>
        <AttachedFiles files={files} onRemove={onRemove} />
        <textarea
          ref={box}
          rows={1}
          value={text}
          placeholder={working ? "queue a message for when this turn is done" : "tell the agent what to do"}
          onChange={(e) => {
            const just = sent.current;
            if (just && Date.now() - just.at < 1000) {
              const kind = (e.nativeEvent as InputEvent).inputType;
              if (kind === "insertReplacementText" || e.target.value.trim() === just.text) {
                e.target.value = "";
                return;
              }
            }
            sent.current = null;
            setText(e.target.value);
          }}
          // A screenshot on the clipboard is a file to send rather than text
          // to type. Text pastes as text, even when a file came with it, as
          // an office suite's cells do.
          onPaste={(e) => {
            const pasted = [...e.clipboardData.files];
            if (!pasted.length || e.clipboardData.getData("text/plain")) return;
            e.preventDefault();
            onAttach(pasted);
          }}
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
          {state?.files && (
            <>
              <button className="chat-attach" title="send files with the message" onClick={() => picker.current?.click()}>
                <Attach aria-label="attach files" />
              </button>
              <input
                ref={picker}
                type="file"
                multiple
                hidden
                onChange={(e) => {
                  onAttach([...(e.target.files ?? [])]);
                  // So picking the same file again is still a change.
                  e.target.value = "";
                }}
              />
            </>
          )}
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
              title={model ? `model: ${model}` : "model"}
              // Claude Code reports a full id and offers aliases to switch to,
              // so the closed control names the one running and the list
              // holds the choices, each with the model it stands for.
              value={model ?? models[0].value}
              shown={model ? modelName(model) : undefined}
              options={models.map((m) => ({
                value: m.value,
                label: m.display_name,
                hint: m.description.split(" · ")[0],
              }))}
              onChange={(next) => onCommand({ op: "model", model: next })}
            />
          )}
          {efforts.length > 0 && (
            <Select
              className="chat-select"
              aria-label="reasoning effort"
              title={effort ? `reasoning effort: ${effortName(effort)}` : "reasoning effort"}
              value={effort ?? ""}
              shown={effort ? undefined : "effort"}
              options={[
                { value: "", label: "default", hint: "the model's own" },
                ...efforts.map((e) => ({ value: e, label: effortName(e) })),
              ]}
              onChange={(next) => onCommand({ op: "effort", effort: next || null })}
            />
          )}
          <span className="chat-facts">
            {state?.status === "compacting" && <span>compacting</span>}
            {state?.context_percentage != null && (
              <span title="context used">ctx {Math.round(state.context_percentage)}%</span>
            )}
            {state?.cost_usd != null && state.cost_usd > 0 && (
              <span title="spent in this conversation">${state.cost_usd.toFixed(2)}</span>
            )}
          </span>
          {working ? (
            <button className="chat-stop" title="stop this turn (Esc)" onClick={() => onCommand({ op: "interrupt" })}>
              <Stop aria-label="stop" />
            </button>
          ) : null}
          <button className="go chat-send" title="send (Enter)" disabled={!text.trim() && !files.length} onClick={send}>
            <Send aria-label="send" />
          </button>
        </div>
      </div>
    </div>
  );
}

export function ChatPane({ server, name, conv }: { server: string; name: string; conv: string }) {
  const [view, dispatch] = useReducer(reduce, EMPTY);
  const channel = useRef(0);
  const seq = useRef(0);
  seq.current = view.seq;
  // An image a reply names, read from this session's working copy.
  const readPicture = useMemo<ReadPicture>(
    () => (path) => api.file(server, name, path).then((f) => f.image ?? undefined),
    [server, name],
  );

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

  const [files, setFiles] = useState<Attached[]>([]);
  const [refused, setRefused] = useState<string | null>(null);
  const [dropping, setDropping] = useState(false);
  const takesFiles = view.state?.files ?? false;
  const attach = (picked: File[]) => {
    if (!picked.length) return;
    if (!takesFiles) {
      setRefused("This session's agent was started before files could be sent; a new session takes them.");
      return;
    }
    const total = [...files.map((f) => f.size), ...picked.map((f) => f.size)].reduce((a, b) => a + b, 0);
    if (total > UPLOAD_BYTES) {
      setRefused(`A message takes ${UPLOAD_BYTES >> 20} MB of files; these come to ${size(total)}.`);
      return;
    }
    setRefused(null);
    Promise.all(picked.map(read)).then(
      (read) => setFiles((had) => [...had, ...read]),
      (e) => setRefused(String(e)),
    );
  };
  // Only a drag carrying files is one to take: a tab being dragged past is not.
  const carriesFiles = (e: React.DragEvent) => e.dataTransfer.types.includes("Files");

  const receiptsOn = view.state?.receipts ?? false;
  const { items, queued, agents } = useMemo(
    () => (view.caughtUp ? build(view.entries) : build([])),
    [view.caughtUp, view.entries],
  );
  const [following, setFollowing] = useState<string | null>(null);
  const followed = agents.find((a) => a.id === following) ?? null;
  const openAgent = useMemo(
    () => (callId: string) => {
      const a = agents.find((x) => x.call?.id === callId);
      if (a) setFollowing(a.id);
    },
    [agents],
  );

  // Kept at the bottom while it is at the bottom: a reply being written
  // scrolls with itself, and someone who has scrolled up to read is left
  // where they are.
  //
  // On a change of size as well as of entries: a picture, a highlighted block
  // or a diff fills in after the render that placed it, and the log grows
  // without a scroll to say so -- the bottom was left behind by however tall
  // it turned out to be, and the newest message was below the fold.
  const scroller = useRef<HTMLDivElement>(null);
  const log = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [items, queued, view.live, view.state?.pending.length]);
  useEffect(() => {
    const el = scroller.current;
    if (!el || !log.current) return;
    const observer = new ResizeObserver(() => {
      if (pinned.current) el.scrollTop = el.scrollHeight;
    });
    observer.observe(log.current);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const pending = view.state?.pending ?? [];
  const receipts = !receiptsOn ? "off" : view.state?.activity === "idle" ? "idle" : "working";
  const nothing = items.length === 0 && queued.length === 0 && !view.live;

  return (
    <div className="chat-split">
      <OpenAgent.Provider value={openAgent}>
        <div
          className={`chat${dropping ? " dropping" : ""}`}
          onDragOver={(e) => {
            if (!carriesFiles(e)) return;
            e.preventDefault();
            e.dataTransfer.dropEffect = takesFiles ? "copy" : "none";
            setDropping(true);
          }}
          onDragLeave={(e) => {
            // Left the pane, rather than moved from one of its children to another.
            if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropping(false);
          }}
          onDrop={(e) => {
            if (!carriesFiles(e)) return;
            e.preventDefault();
            setDropping(false);
            attach([...e.dataTransfer.files]);
          }}
        >
          <div
            className="chat-scroll"
            ref={scroller}
            onScroll={(e) => {
              const el = e.currentTarget;
              pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
            }}
          >
            <div className="chat-log" ref={log}>
              {nothing &&
                (view.notice ? (
                  <Empty icon={Working} note={view.notice} />
                ) : !view.caughtUp && view.entries.length > 0 ? (
                  <Empty icon={Working} />
                ) : view.state ? (
                  <Empty icon={ChatGlyph} />
                ) : null)}
              <Pictures.Provider value={readPicture}>
                <Items items={items} progress={view.progress} receipts={receipts} />
                <Writing live={view.live} entries={view.entries} />
              </Pictures.Provider>
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
              {/* Sent and not yet read: after everything the turn in progress is
                  still doing, since Claude reads them when it is done. */}
              <Items items={queued} receipts={receipts} />
            </div>
          </div>
          <AgentsBar agents={agents} open={following} onOpen={setFollowing} />
          {(view.down || view.error || refused) && (
            <p className="error chat-banner">{view.down ?? refused ?? view.error}</p>
          )}
          <Composer
            state={view.state}
            onCommand={command}
            files={files}
            onAttach={attach}
            onRemove={(key) => setFiles((had) => had.filter((f) => f.key !== key))}
            onSent={() => {
              setFiles([]);
              setRefused(null);
            }}
          />
        </div>
      </OpenAgent.Provider>
      {followed && (
        <Pictures.Provider value={readPicture}>
          <AgentPanel agent={followed} progress={view.tasks.get(followed.id)} onClose={() => setFollowing(null)} />
        </Pictures.Provider>
      )}
    </div>
  );
}
