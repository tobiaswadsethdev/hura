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
// channel that drops is opened again from the last entry it saw.
//
// What this pane has seen is remembered for when it is opened again, after a
// switch to another worktree and back: it draws that at once and asks only for
// what came after, rather than having the whole conversation replayed. A host
// that answers with less than was remembered has started its transcript again,
// and is asked for all of it.

import { useEffect, useId, useLayoutEffect, useMemo, useReducer, useRef, useState } from "react";

import { Empty } from "../Empty";
import type { ChatCommand } from "../gen/ChatCommand";
import type { ChatEntry } from "../gen/ChatEntry";
import type { ChatFrame } from "../gen/ChatFrame";
import type { ChatState } from "../gen/ChatState";
import type { SlashCommand } from "../gen/SlashCommand";
import { Attach, Chat as ChatGlyph, Close, Plus, Send, Shell, Skill, Stop, Working } from "../icons";
import { api } from "../api";
import { keyFor, Remembered } from "../cache";
import { Markdown, Pictures, type ReadPicture } from "../Markdown";
import type { PartialMessage, ToolProgress } from "../sdk";
import { Select } from "../Select";
import { chat, close, nextChannelId, open } from "../stream";
import { AskCard } from "../chat/Ask";
import { matching, naming, offeredCommands, type Typed, typed } from "../chat/commands";
import { type Attached, isPicture, read, size, UPLOAD_BYTES } from "../chat/files";
import { Items, OpenAgent, Unpin } from "../chat/Items";
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

/// What a conversation's pane had when it went, by server, worktree and
/// conversation. A few, since a transcript is mostly pictures and can run to
/// megabytes.
type Kept = { entries: ChatEntry[]; seq: number; state: ChatState | null };
const kept = new Remembered<Kept>(12);

/// A conversation that has ended: one opened later under the same name is
/// another conversation.
export function forgetChat(server: string, name: string, conv: string) {
  kept.delete(keyFor(server, name, "chat", conv));
}

/// Where a pane starts: from what it had, drawn at once, when it had anything.
function restore(key: string): View {
  const k = kept.get(key);
  return k ? { ...EMPTY, ...k, caughtUp: true } : EMPTY;
}

type Action = { type: "frame"; frame: ChatFrame } | { type: "down"; reason: string } | { type: "reset" };

function reduce(v: View, a: Action): View {
  if (a.type === "down") return { ...v, down: a.reason, live: null };
  if (a.type === "reset") return EMPTY;
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
            Thinking…
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
  { value: "auto", label: "Auto", hint: "Decides what to ask" },
  { value: "default", label: "Ask", hint: "Before edits and commands" },
  { value: "acceptEdits", label: "Accept Edits", hint: "Asks for the rest" },
  { value: "plan", label: "Plan", hint: "Changes nothing" },
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
  return e === "xhigh" ? "Extra High" : e.charAt(0).toUpperCase() + e.slice(1);
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
          <button className="chat-unattach" title="Take it out of the message" onClick={() => onRemove(f.key)}>
            <Close aria-label={`Remove ${f.name}`} />
          </button>
        </li>
      ))}
    </ul>
  );
}

/// What is typed, drawn under the textarea in its place: a slash command the
/// host offers in the accent, with what it takes after it until something is
/// typed there, or a `!` command with its bang marked. Only colour and a
/// background differ from the textarea's own text, never the metrics, so
/// every character lands where the caret thinks it is.
function Shade({ what, shade }: { what: Typed; shade: React.Ref<HTMLDivElement> }) {
  if (what.kind === "message" || (what.kind === "command" && !what.known)) return null;
  return (
    <div className="chat-shade" ref={shade} aria-hidden>
      {what.kind === "shell" ? (
        <>
          <span className="chat-bang">!</span>
          {what.command}
        </>
      ) : (
        <>
          <span className="chat-slash">/{what.name}</span>
          {what.rest}
          {what.known?.argument_hint && !what.rest.trim() && (
            <span className="chat-ghost">
              {what.rest ? "" : " "}
              {what.known.argument_hint}
            </span>
          )}
        </>
      )}
      {/* A last line that is empty still has a height. */}
      {"\u200b"}
    </div>
  );
}

/// The slash commands that fit what is typed, under the pointer or the arrow
/// keys; the active one's description beside its name.
function CommandList({
  id,
  commands,
  active,
  onHover,
  onPick,
}: {
  id: string;
  commands: SlashCommand[];
  active: number;
  onHover: (i: number) => void;
  onPick: (c: SlashCommand) => void;
}) {
  const list = useRef<HTMLUListElement>(null);
  useLayoutEffect(() => {
    list.current?.children[active]?.scrollIntoView({ block: "nearest" });
  }, [active]);
  return (
    <ul className="chat-commands" role="listbox" id={id} ref={list} aria-label="Slash commands">
      {commands.map((c, i) => (
        <li
          key={c.name}
          id={`${id}-${i}`}
          role="option"
          aria-selected={i === active}
          className={i === active ? "active" : undefined}
          onMouseMove={() => i !== active && onHover(i)}
          // Down rather than click, so the textarea keeps the focus.
          onMouseDown={(e) => {
            e.preventDefault();
            onPick(c);
          }}
        >
          <span className="chat-command-name">
            /{c.name}
            {c.argument_hint && <span className="chat-command-hint"> {c.argument_hint}</span>}
          </span>
          {(c.description || c.aliases.length > 0) && (
            <span className="chat-command-about">
              {c.description}
              {c.aliases.length > 0 && ` (also ${c.aliases.map((a) => `/${a}`).join(", ")})`}
            </span>
          )}
        </li>
      ))}
    </ul>
  );
}

/// A line under what is typed saying what sending it will do, when that is
/// not to say it to Claude: run a shell command, or run a slash command,
/// which says what it does once the list of them has gone.
function ModeNote({ what, open }: { what: Typed; open: boolean }) {
  if (what.kind === "shell") {
    return (
      <p className="chat-mode">
        <Shell />
        Runs in the working copy; Claude sees what it prints with your next message
      </p>
    );
  }
  if (what.kind === "command" && what.known?.description && !open) {
    return (
      <p className="chat-mode">
        <Skill />
        <span>{what.known.description}</span>
      </p>
    );
  }
  return null;
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
  const shade = useRef<HTMLDivElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const listId = useId();
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

  const commands = useMemo(() => offeredCommands(state), [state?.slash_commands, state?.commands]);
  const what = typed(text, commands, state?.shell ?? false);
  // The commands, offered while a name is being typed, until Esc puts them
  // away for that name.
  const query = naming(text);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const offered = useMemo(
    () => (query === null || query === dismissed ? [] : matching(query, commands)),
    [query, dismissed, commands],
  );
  const [active, setActive] = useState(0);
  useEffect(() => setActive(0), [query]);
  const picked = offered[Math.min(active, offered.length - 1)];

  const complete = (c: SlashCommand) => {
    setText(`/${c.name} `);
    box.current?.focus();
  };

  const say = (t: string) => {
    sent.current = { text: t, at: Date.now() };
    setText("");
  };
  const send = () => {
    if (what.kind === "shell") {
      // Files wait for the next message: a command does not take them.
      const command = what.command.trim();
      if (!command) return;
      onCommand({ op: "shell", command });
      say(text.trim());
      return;
    }
    const t = text.trim();
    if (!t && !files.length) return;
    onCommand({
      op: "send",
      text: t,
      files: files.length ? files.map(({ name, mime, data }) => ({ name, mime, data })) : undefined,
    });
    say(t);
    onSent();
  };
  const sendable = what.kind === "shell" ? Boolean(what.command.trim()) : Boolean(text.trim() || files.length);

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
        <CommandList id={listId} commands={offered} active={active} onHover={setActive} onPick={complete} />
      )}
      {/* One box: what you write, and under it, in the same frame, what it
          will run under. The controls are words until pointed at, because
          they are read far more often than they are changed. A click on the
          box's own padding is a click on the text. */}
      <div
        className={`chat-box${what.kind === "shell" ? " shell" : ""}`}
        onClick={(e) => e.target === e.currentTarget && box.current?.focus()}
      >
        <AttachedFiles files={files} onRemove={onRemove} />
        <div className="chat-field">
          <Shade what={what} shade={shade} />
          <textarea
            ref={box}
            rows={1}
            value={text}
            className={what.kind === "shell" || (what.kind === "command" && what.known) ? "shaded" : undefined}
            role="combobox"
            aria-expanded={offered.length > 0}
            aria-controls={offered.length > 0 ? listId : undefined}
            aria-activedescendant={offered.length > 0 ? `${listId}-${Math.min(active, offered.length - 1)}` : undefined}
            aria-autocomplete="list"
            // A command is code: corrected, `wc` became "we".
            spellCheck={!(what.kind === "shell" || query !== null)}
            autoCorrect={what.kind === "shell" || query !== null ? "off" : undefined}
            autoCapitalize={what.kind === "shell" || query !== null ? "off" : undefined}
            placeholder={
              working
                ? "Queue a message for when this turn is done"
                : commands.length || state?.shell
                  ? `Tell the agent what to do${commands.length ? " · / for commands" : ""}${state?.shell ? " · ! for the shell" : ""}`
                  : "Tell the agent what to do"
            }
            onScroll={(e) => {
              if (shade.current) shade.current.scrollTop = e.currentTarget.scrollTop;
            }}
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
              if (naming(e.target.value) !== dismissed) setDismissed(null);
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
              if (picked && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
                e.preventDefault();
                const step = e.key === "ArrowDown" ? 1 : -1;
                setActive((active + step + offered.length) % offered.length);
              } else if (picked && e.key === "Tab") {
                e.preventDefault();
                complete(picked);
              } else if (picked && e.key === "Enter" && !e.shiftKey && query !== picked.name) {
                // The name, completed. Sent only once it is whole, so a
                // command is never run by a key meant to finish typing it.
                e.preventDefault();
                complete(picked);
              } else if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              } else if (e.key === "Escape" && picked) {
                e.preventDefault();
                setDismissed(query);
              } else if (e.key === "Escape" && working) {
                e.preventDefault();
                onCommand({ op: "interrupt" });
              }
            }}
          />
        </div>
        <ModeNote what={what} open={offered.length > 0} />
        <div className="chat-controls">
          {state?.files && (
            <>
              <button
                className="chat-round chat-attach"
                title="Send files with the message"
                aria-label="Attach Files"
                onClick={() => picker.current?.click()}
              >
                <Plus size={16} />
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
            aria-label="Permission mode"
            title="Permission mode"
            value={state?.mode ?? "auto"}
            options={MODES}
            onChange={(mode) => onCommand({ op: "mode", mode })}
          />
          {models.length > 0 && (
            <Select
              className="chat-select"
              aria-label="Model"
              title={model ? `Model: ${model}` : "Model"}
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
              aria-label="Reasoning effort"
              title={effort ? `Reasoning effort: ${effortName(effort)}` : "Reasoning effort"}
              value={effort ?? ""}
              shown={effort ? undefined : "Effort"}
              options={[
                { value: "", label: "Default", hint: "The model's own" },
                ...efforts.map((e) => ({ value: e, label: effortName(e) })),
              ]}
              onChange={(next) => onCommand({ op: "effort", effort: next || null })}
            />
          )}
          <span className="chat-facts">
            {state?.status === "compacting" && <span>compacting</span>}
            {state?.context_percentage != null && (
              <span title="Context used">ctx {Math.round(state.context_percentage)}%</span>
            )}
          </span>
          {/* One round button at the end, as on the phone: Stop while a turn
              runs and nothing is written, Send otherwise -- a message
              written during a turn is queued, so it is still sent. */}
          {working && !text.trim() && !files.length ? (
            <button
              className="chat-round chat-stop"
              title="Stop This Turn (Esc)"
              aria-label="Stop"
              onClick={() => onCommand({ op: "interrupt" })}
            >
              <Stop size={10} fill="currentColor" />
            </button>
          ) : (
            <button
              className="chat-round chat-send"
              title={what.kind === "shell" ? "Run It in the Sandbox (Enter)" : "Send (Enter)"}
              aria-label={what.kind === "shell" ? "Run" : "Send"}
              disabled={!sendable}
              onClick={send}
            >
              {what.kind === "shell" ? <Shell size={15} /> : <Send size={16} strokeWidth={2.25} />}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

export function ChatPane({ server, name, conv }: { server: string; name: string; conv: string }) {
  const key = keyFor(server, name, "chat", conv);
  const [view, dispatch] = useReducer(reduce, key, restore);
  const channel = useRef(0);
  const seq = useRef(0);
  seq.current = view.seq;

  // Kept once the history is all here: a pane that went part-way through a
  // replay would otherwise come back as only the start of it.
  useEffect(() => {
    if (view.caughtUp) kept.set(key, { entries: view.entries, seq: view.seq, state: view.state });
  }, [key, view.caughtUp, view.entries, view.seq, view.state]);
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
        if (!live || id !== channel.current) return;
        if (frame.is === "chat") {
          wait = 2_000;
          // Every replay frame ends past `since` unless the host has less than
          // this pane does: its transcript started again, and what is here is
          // another conversation's. Asked again from the beginning.
          if (frame.frame.t === "replay" && frame.frame.seq < seq.current) {
            seq.current = 0;
            dispatch({ type: "reset" });
            void close(id);
            connect();
            return;
          }
          dispatch({ type: "frame", frame: frame.frame });
        } else if (frame.is === "closed") {
          // Opened again from the last entry, after a pause that grows while
          // nothing answers: a session being destroyed should not be asked
          // about twice a second until the tab goes.
          dispatch({ type: "down", reason: frame.reason ?? "Disconnected" });
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
  const unpin = useMemo(() => () => void (pinned.current = false), []);
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
  // A `!` command running says so in its own card.
  const last = items.at(-1);
  const shellRunning = last?.kind === "shell" && last.output === null;

  return (
    <div className="chat-split">
      <OpenAgent.Provider value={openAgent}>
        <Unpin.Provider value={unpin}>
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
                {view.state?.activity === "running" && pending.length === 0 && !shellRunning && (
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
        </Unpin.Provider>
      </OpenAgent.Provider>
      {followed && (
        <Pictures.Provider value={readPicture}>
          <AgentPanel agent={followed} progress={view.tasks.get(followed.id)} onClose={() => setFollowing(null)} />
        </Pictures.Provider>
      )}
    </div>
  );
}
