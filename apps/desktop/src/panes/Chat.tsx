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
//
// A search result opens a conversation at the message it found: the pane is
// handed the entry, marks the row it is drawn in, and scrolls there.

import { keysOf, pressed } from "../keys";
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useReducer, useRef, useState } from "react";

import { Empty } from "../Empty";
import type { ChatCommand } from "../gen/ChatCommand";
import type { ChatEntry } from "../gen/ChatEntry";
import type { ChatFrame } from "../gen/ChatFrame";
import type { ChatState } from "../gen/ChatState";
import type { SlashCommand } from "../gen/SlashCommand";
import { EditPrompt } from "../chat/Items";
import type { Usage } from "../gen/Usage";
import {
  Attach,
  Chat as ChatGlyph,
  Close,
  Edit,
  FileIcon,
  SideChat as SideGlyph,
  Folder,
  ModeAsk,
  ModeAuto,
  ModeEdits,
  Stash as StashGlyph,
  ModePlan,
  Queued,
  Recurring,
  Scheduled,
  Send,
  SendQueued,
  SendSteer,
  Shell,
  Skill,
  Working,
} from "../icons";
import { useButtonMenu } from "../ContextMenu";
import type { ChatModel } from "../gen/ChatModel";
import type { FoundPaths } from "../gen/FoundPaths";
import type { ChatAsk } from "../gen/ChatAsk";
import type { ChatDecision } from "../gen/ChatDecision";
import { QuestionDrawer, useQuestions } from "../chat/Questions";
import { type Stashed, useStash } from "../chat/stash";
import { since } from "../chat/elapsed";
import { AgentMark } from "../agents";
import { api, messageOf } from "../api";
import { epochOf, localInput } from "../NewWorktree";
import { type Cadence, describeRepeat, repeatOf, serverCanRepeat } from "../repeats";
import type { Repeat } from "../gen/Repeat";
import { startsAt } from "../Tree";
import type { QueuedMessage } from "../gen/QueuedMessage";
import { keyFor, Remembered } from "../cache";
import { download, Downloader } from "../Download";
import { Markdown, Pictures, type ReadPicture } from "../Markdown";
import type { PartialMessage } from "../sdk";
import { chat, close, nextChannelId, open } from "../stream";
import { AskCard } from "../chat/Ask";
import {
  hasMention,
  matching,
  mentionAt,
  mentions,
  naming,
  offeredCommands,
  type Typed,
  typed,
} from "../chat/commands";
import { type Attached, isPicture, read, size, UPLOAD_BYTES } from "../chat/files";
import { Found, Items, OpenAgent, Unpin } from "../chat/Items";
import { arrived, build, group, type Item, locate, settle, turnStart } from "../chat/transcript";
import { Since, Ticking } from "../chat/elapsed";
import { readDraft, writeDraft } from "../chat/drafts";
import { AgentPanel, AgentsBar, CommandPanel, type TaskProgress } from "../chat/Agents";
import { PullBar } from "../chat/PullBar";
import type { PullRequest as Pull } from "../gen/PullRequest";
import { ContextRing } from "../chat/Usage";
import { Zoomable } from "../Picture";

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
/// What to say in a conversation as soon as it is open: a side chat's
/// question, an edited message's new words. Said once, by its pane.
const firstWords = new Map<string, string>();
export function sayFirst(server: string, name: string, conv: string, text: string) {
  firstWords.set(keyFor(server, name, "chat", conv), text);
}

/// The last thing the agent said before an entry, by the uuid Claude Code
/// gave it: where a conversation forks to edit the message at `seq`. Null
/// before anything was said, when there is nothing to keep.
function assistantBefore(entries: ChatEntry[], seq: number): string | null {
  let at: string | null = null;
  for (const e of entries) {
    if (e.seq >= seq) break;
    if (e.ev.kind !== "sdk") continue;
    const m = e.ev.msg as { type?: string; uuid?: unknown; parent_tool_use_id?: unknown };
    if (m.type === "assistant" && m.parent_tool_use_id == null && typeof m.uuid === "string") at = m.uuid;
  }
  return at;
}

/// T3 Code's side chat, as a command of the composer's own: a question asked
/// beside the conversation, with its context, that does not interrupt it.
const SIDE_COMMAND: SlashCommand = {
  name: "btw",
  description: "Ask on the side, with this conversation's context, without interrupting it",
  argument_hint: "<question>",
  aliases: ["side"],
};

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
      };
    }
    case "partial":
      return { ...v, live: stream(v.live, f.ev as PartialMessage) };
    case "progress":
      // A call's clock runs from when it was made (`ToolCall.at`), for every
      // tool alike: only some of them say how long they have been going.
      return v;
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
/// Stop the turn and say it now, rather than after the turn: T3 Code's steer,
/// on the platform's command key with Enter or a click.
const MAC = navigator.userAgent.includes("Mac");
const STEER_KEYS = MAC ? "⌘↵" : "Ctrl+Enter";
const steering = (e: { metaKey: boolean; ctrlKey: boolean }) => (MAC ? e.metaKey : e.ctrlKey);

const MODES = [
  { value: "auto", label: "Auto", hint: "Decides what to ask" },
  { value: "default", label: "Ask", hint: "Before edits and commands" },
  { value: "acceptEdits", label: "Accept Edits", hint: "Asks for the rest" },
  { value: "plan", label: "Plan", hint: "Changes nothing" },
];
const MODE_ICONS: Record<string, React.ComponentType<{ size?: number }>> = {
  auto: ModeAuto,
  default: ModeAsk,
  acceptEdits: ModeEdits,
  plan: ModePlan,
};

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
            <Zoomable src={`data:${f.mime};base64,${f.data}`} alt={f.name} />
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
function Shade({ what, text, shade }: { what: Typed; text: string; shade: React.Ref<HTMLDivElement> }) {
  const plain = what.kind === "message" || (what.kind === "command" && !what.known);
  if (plain && !hasMention(text)) return null;
  // A mention, coloured as a chip is, in whatever it is typed into.
  const marked = (t: string) =>
    mentions(t).map((part, i) =>
      part.mention ? (
        <span key={i} className="chat-mention">
          {part.text}
        </span>
      ) : (
        part.text
      ),
    );
  return (
    <div className="chat-shade" ref={shade} aria-hidden>
      {plain ? (
        marked(text)
      ) : what.kind === "shell" ? (
        <>
          <span className="chat-bang">!</span>
          {what.command}
        </>
      ) : what.kind === "command" ? (
        <>
          <span className="chat-slash">/{what.name}</span>
          {marked(what.rest)}
          {what.known?.argument_hint && !what.rest.trim() && (
            <span className="chat-ghost">
              {what.rest ? "" : " "}
              {what.known.argument_hint}
            </span>
          )}
        </>
      ) : null}
      {/* A last line that is empty still has a height. */}
      {"\u200b"}
    </div>
  );
}

/// The paths in the working copy that fit the `@` being typed, in the same
/// drawer as the slash commands: the file's kind as its icon, its name, and
/// the directory it is in, quieter.
function MentionList({
  id,
  found,
  active,
  onHover,
  onPick,
}: {
  id: string;
  found: { paths: string[]; error: string | null; searching: boolean };
  active: number;
  onHover: (i: number) => void;
  onPick: (path: string) => void;
}) {
  const list = useRef<HTMLUListElement>(null);
  useLayoutEffect(() => {
    list.current?.children[active]?.scrollIntoView({ block: "nearest" });
  }, [active]);
  return (
    <ul className="chat-commands" role="listbox" id={id} ref={list} aria-label="Files">
      {found.paths.length === 0 ? (
        <li className="chat-mention-note" role="presentation">
          {found.error ?? (found.searching ? "Searching the working copy…" : "No matching files or folders")}
        </li>
      ) : (
        found.paths.map((path, i) => {
          const dir = path.endsWith("/");
          const trimmed = dir ? path.slice(0, -1) : path;
          const cut = trimmed.lastIndexOf("/");
          const name = trimmed.slice(cut + 1);
          return (
            <li
              key={path}
              id={`${id}-${i}`}
              role="option"
              aria-selected={i === active}
              className={i === active ? "active" : undefined}
              onMouseMove={() => i !== active && onHover(i)}
              // Down rather than click, so the textarea keeps the focus.
              onMouseDown={(e) => {
                e.preventDefault();
                onPick(path);
              }}
            >
              <span className="chat-mention-icon">{dir ? <Folder name={name} open={false} /> : <FileIcon name={name} />}</span>
              <span className="chat-command-name">{name}</span>
              {cut > 0 && <span className="chat-command-about">{trimmed.slice(0, cut)}</span>}
            </li>
          );
        })
      )}
    </ul>
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
  draftKey,
  history,
  state,
  usage,
  onCommand,
  files,
  onAttach,
  onRemove,
  onSent,
  onSchedule,
  findFiles,
  load,
  question,
  onAnswer,
  agent,
  onSideChat,
  branching,
  onBranch,
  onCancelBranch,
}: {
  /// Which conversation's draft the box holds: see `chat/drafts.ts`.
  draftKey: string;
  /// What you have said in this conversation, oldest first: what the up arrow
  /// in an empty box steps back through.
  history: string[];
  state: ChatState | null;
  usage: Usage | null;
  onCommand: (command: ChatCommand) => void;
  files: Attached[];
  onAttach: (files: File[]) => void;
  onRemove: (key: string) => void;
  onSent: () => void;
  /// Keep a message on the server to say at a time: what the clock button
  /// does instead of sending. Throws to leave the text where it is.
  /// With `repeat`, said again and again: see `repeats.ts`.
  onSchedule: (text: string, at: number, repeat: Repeat | null) => Promise<void>;
  /// The working copy's paths that fit an `@` mention; rejects on a server
  /// that cannot search them.
  findFiles: (query: string) => Promise<FoundPaths>;
  /// Text to put in the box, once per `nonce`: a queued message taken back to
  /// change. After what is already there, if anything is.
  load: { text: string; nonce: number; replace?: boolean } | null;
  /// The agent's question waiting on you, answered from here: see
  /// `chat/Questions.tsx`.
  question: ChatAsk | null;
  onAnswer: (decision: ChatDecision) => void;
  agent: string;
  /// Ask on the side (`/btw`); unset in a side chat itself.
  onSideChat?: (question: string) => void;
  /// An earlier message is being edited: sending forks the conversation
  /// rather than saying it here.
  branching: boolean;
  onBranch: (text: string) => void;
  onCancelBranch: () => void;
}) {
  const [text, setText] = useState(() => readDraft(draftKey));
  // Kept a moment after typing stops, and at once when the pane goes, so a
  // switch to another worktree mid-word keeps the word.
  const latest = useRef(text);
  latest.current = text;
  useEffect(() => {
    if (!load) return;
    const had = latest.current.trimEnd();
    const next = had && !load.replace ? `${had}\n${load.text}` : load.text;
    setText(next);
    requestAnimationFrame(() => {
      const el = box.current;
      if (!el) return;
      el.focus();
      el.selectionStart = el.selectionEnd = next.length;
    });
  }, [load?.nonce]);
  useEffect(() => {
    const timer = setTimeout(() => writeDraft(draftKey, text), 300);
    return () => clearTimeout(timer);
  }, [draftKey, text]);
  useEffect(() => () => writeDraft(draftKey, latest.current), [draftKey]);
  // Open while a message is being written for later: the time it is for, as
  // a `datetime-local` input holds it. Enter then schedules rather than sends.
  const [later, setLater] = useState<string | null>(null);
  // And how often after that, when the server can repeat it.
  const [again, setAgain] = useState<"once" | "daily" | "weekdays" | "weekly" | "hourly">("once");
  const box = useRef<HTMLTextAreaElement>(null);
  const shade = useRef<HTMLDivElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const listId = useId();
  /// What was just sent, and when. WebKit commits a pending autocorrection or
  /// inline prediction into the box after Enter has emptied it, which put the
  /// message straight back; for a moment after a send, that is not typing.
  const sent = useRef<{ text: string; at: number } | null>(null);
  const working = state?.activity === "running" || state?.activity === "waiting";

  const commands = useMemo(
    () => [...(onSideChat ? [SIDE_COMMAND] : []), ...offeredCommands(state)],
    [state?.slash_commands, state?.commands, onSideChat],
  );
  // T3 Code's "Ask anything", with the keys this box answers to after it.
  const hints = [commands.length ? "/ commands" : null, state?.shell ? "! shell" : null].filter(Boolean);
  const flow = useQuestions(question, onAnswer);
  // T3 Code's stash: Ctrl+S puts what is written aside, and the tab on the box
  // gives it back.
  const stash = useStash();
  const [stashOpen, setStashOpen] = useState(false);
  const unstash = (s: Stashed) => {
    setText((had) => (had.trim() ? `${had.trimEnd()}\n${s.text}` : s.text));
    stash.take(s.id);
    setStashOpen(false);
    requestAnimationFrame(() => box.current?.focus());
  };
  const asking = flow.question !== null;
  const placeholder = asking
    ? "Type your own answer, or leave this blank to use the choice picked"
    : branching
    ? "Say it again, or differently: it goes to a new conversation"
    : working
    ? `Queue a message for after this turn, or ${STEER_KEYS} to stop it and send now`
    : `Ask anything.${hints.length ? ` ${hints.join(", ")}` : ""}`;

  // Where the up arrow has got to in `history`, while what is in the box is
  // still what it put there. Typing anything else lets go of it.
  const [recalled, setRecalled] = useState<number | null>(null);
  const recall = (at: number | null) => {
    setRecalled(at);
    setText(at === null ? "" : history[at]);
    // At the end, so the next arrow down is a step forward rather than a
    // move through the recalled text's lines.
    requestAnimationFrame(() => {
      const el = box.current;
      if (el) el.selectionStart = el.selectionEnd = el.value.length;
    });
  };

  // Grows with what is typed, up to a third of the pane, and no further.
  //
  // Measured again when the box changes width as well as when the text or
  // the placeholder changes, because the same words wrap to more lines in a
  // narrower box. When only a change of text measured it, a composer drawn
  // while the middle was squeezed to a sliver took its placeholder at a word
  // to a line, stood 240 pixels tall, and stayed that tall and empty after the
  // middle had its width back. Not while hidden: a tab out of sight has no
  // width at all, and it is measured again as it comes back.
  const fit = useCallback(() => {
    const el = box.current;
    if (!el || el.clientWidth === 0) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 240)}px`;
  }, []);
  useLayoutEffect(fit, [fit, text, placeholder]);
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    let width = el.clientWidth;
    let frame = 0;
    const observer = new ResizeObserver(() => {
      // A change of height is this measuring itself.
      if (el.clientWidth === width) return;
      width = el.clientWidth;
      // In the next frame rather than in here, where setting the height the
      // observer is watching would be reported as a resize loop.
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(fit);
    });
    observer.observe(el);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, [fit]);

  const what = typed(text, commands, state?.shell ?? false);

  // The `@` mention at the caret, T3 Code's: the working copy searched for
  // what follows it, a moment after typing stops, and the paths offered in the
  // drawer over the box until one is picked or Esc puts them away.
  const [caret, setCaret] = useState(0);
  const typing = later === null && what.kind !== "shell" ? mentionAt(text, caret) : null;
  const [unmentioned, setUnmentioned] = useState<number | null>(null);
  const mention = typing && typing.start !== unmentioned ? typing : null;
  const [found, setFound] = useState<{ query: string; paths: string[]; error: string | null } | null>(null);
  useEffect(() => {
    if (!mention) return;
    const query = mention.query;
    let live = true;
    const timer = setTimeout(() => {
      findFiles(query).then(
        (f) => live && setFound({ query, paths: f.paths, error: null }),
        (e) =>
          live &&
          setFound({
            query,
            paths: [],
            error: /unsupported|unknown variant/i.test(messageOf(e))
              ? "This server cannot search files yet: update hurad"
              : messageOf(e),
          }),
      );
    }, 120);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [mention?.query, mention?.start, findFiles]);
  const shownFound = mention
    ? { paths: found?.paths ?? [], error: found?.error ?? null, searching: found?.query !== mention.query }
    : null;
  const [mentionActive, setMentionActive] = useState(0);
  useEffect(() => setMentionActive(0), [found]);
  const pickedPath = shownFound?.paths[Math.min(mentionActive, shownFound.paths.length - 1)];
  const mentionListId = useId();
  const pickMention = (path: string) => {
    if (!mention) return;
    const before = text.slice(0, mention.start);
    const after = text.slice(caret).replace(/^\S*/, "");
    const inserted = `@${path}${after.startsWith(" ") ? "" : " "}`;
    setText(before + inserted + after);
    const at = before.length + inserted.length;
    setCaret(at);
    requestAnimationFrame(() => {
      const el = box.current;
      if (el) el.selectionStart = el.selectionEnd = at;
    });
  };
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
  const send = (steer = false) => {
    if (later !== null) {
      const t = text.trim();
      const at = epochOf(later);
      if (!t || at === null) return;
      // The time of day, and for a weekly one the day, are the first time's.
      const first = new Date(at * 1000);
      const clock = `${String(first.getHours()).padStart(2, "0")}:${String(first.getMinutes()).padStart(2, "0")}`;
      const rule =
        again === "once"
          ? null
          : repeatOf(again === "weekly" ? (`weekly-${first.getDay()}` as Cadence) : again, clock);
      void onSchedule(t, at, rule).then(
        () => {
          say(t);
          setLater(null);
          setAgain("once");
        },
        () => {},
      );
      return;
    }
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
    if (what.kind === "command" && what.known === SIDE_COMMAND && onSideChat) {
      const q = what.rest.trim();
      if (!q) return;
      onSideChat(q);
      say(t);
      return;
    }
    if (branching) {
      onBranch(t);
      say(t);
      return;
    }
    // Steering is the turn stopped and this said in its place: the host takes
    // the message as the next turn once the interrupt has ended this one.
    if (steer && working) onCommand({ op: "interrupt" });
    onCommand({
      op: "send",
      text: t,
      files: files.length ? files.map(({ name, mime, data }) => ({ name, mime, data })) : undefined,
    });
    say(t);
    onSent();
  };
  // Whether the steer key is held, so the button can say which it will do.
  const [steerHeld, setSteerHeld] = useState(false);
  useEffect(() => {
    if (!working) return;
    const look = (e: KeyboardEvent) => setSteerHeld(steering(e));
    const off = () => setSteerHeld(false);
    window.addEventListener("keydown", look);
    window.addEventListener("keyup", look);
    window.addEventListener("blur", off);
    return () => {
      window.removeEventListener("keydown", look);
      window.removeEventListener("keyup", look);
      window.removeEventListener("blur", off);
      setSteerHeld(false);
    };
  }, [working]);
  // Sending an edited message starts another conversation, so this one's
  // turn has nothing to wait for or stop.
  const steers = working && !branching && later === null && what.kind !== "shell";
  const sendable =
    later !== null
      ? Boolean(text.trim()) && epochOf(later) !== null
      : what.kind === "shell"
        ? Boolean(what.command.trim())
        : Boolean(text.trim() || files.length);

  const models = state?.models ?? [];
  const model = state?.model ?? null;
  // The efforts the model in use takes, found by its alias or the id the
  // alias stands for; the default's while nothing has been chosen.
  const efforts =
    (model ? models.find((m) => m.value === model || m.resolved === model) : models[0])?.efforts ?? [];
  const effort = state?.effort ?? null;
  return (
    <div className="chat-composer">
      {asking && <QuestionDrawer flow={flow} onPick={(label) => flow.toggle(label, () => setText(""))} />}
      {branching && (
        <p className="chat-branching">
          <Edit aria-hidden />
          <span>Editing an earlier message: sending it starts a new conversation from just before it, and this one stays as it is.</span>
          <button onClick={onCancelBranch}>Cancel</button>
        </p>
      )}
      {!asking && !branching && !shownFound && offered.length === 0 && stash.all.length > 0 && (
        <button
          className={`chat-stash-tab${stashOpen ? " on" : ""}`}
          title={`Prompts Put Aside${keysOf("stash").label ? ` (${keysOf("stash").label} puts one here)` : ""}`}
          aria-expanded={stashOpen}
          onClick={() => setStashOpen(!stashOpen)}
        >
          <StashGlyph aria-hidden />
          Stash {stash.all.length}
        </button>
      )}
      {!asking && !shownFound && offered.length === 0 && stashOpen && stash.all.length > 0 && (
        <ul className="chat-commands chat-stash" aria-label="Prompts put aside">
          {stash.all.map((s) => (
            <li key={s.id} onMouseDown={(e) => e.preventDefault()} onClick={() => unstash(s)} title="Back into the box">
              <span className="chat-stash-text">{s.text}</span>
              <span className="chat-command-about chat-stash-age">{since(Date.now() - s.at)} ago</span>
              <button
                className="chat-stash-drop"
                title="Throw It Away"
                aria-label="Throw it away"
                onClick={(e) => {
                  e.stopPropagation();
                  stash.take(s.id);
                }}
              >
                <Close aria-hidden />
              </button>
            </li>
          ))}
        </ul>
      )}
      {!asking && shownFound && offered.length === 0 && (
        <MentionList
          id={mentionListId}
          found={shownFound}
          active={mentionActive}
          onHover={setMentionActive}
          onPick={pickMention}
        />
      )}
      {!asking && offered.length > 0 && (
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
        {later !== null && (
          <div className="chat-later">
            <Scheduled />
            <span>Say this at</span>
            <input
              type="datetime-local"
              value={later}
              aria-label="When to say it"
              onChange={(e) => setLater(e.target.value)}
            />
            {serverCanRepeat() && (
              <AgainControl
                value={again}
                day={(() => {
                  const at = epochOf(later);
                  return at === null ? null : new Date(at * 1000).toLocaleDateString([], { weekday: "long" });
                })()}
                onChange={setAgain}
              />
            )}
            <span className="hint">or when the turn running then ends · kept by the server</span>
            <button className="quiet-icon" title="Send Now Instead" aria-label="Cancel" onClick={() => setLater(null)}>
              <Close />
            </button>
          </div>
        )}
        <div className="chat-field">
          <Shade what={what} text={text} shade={shade} />
          <textarea
            ref={box}
            rows={1}
            value={text}
            className={
              what.kind === "shell" || (what.kind === "command" && what.known) || hasMention(text)
                ? "shaded"
                : undefined
            }
            role="combobox"
            aria-expanded={offered.length > 0}
            aria-controls={offered.length > 0 ? listId : undefined}
            aria-activedescendant={offered.length > 0 ? `${listId}-${Math.min(active, offered.length - 1)}` : undefined}
            aria-autocomplete="list"
            // A command is code: corrected, `wc` became "we".
            spellCheck={!(what.kind === "shell" || query !== null)}
            autoCorrect={what.kind === "shell" || query !== null ? "off" : undefined}
            autoCapitalize={what.kind === "shell" || query !== null ? "off" : undefined}
            placeholder={placeholder}
            onScroll={(e) => {
              if (shade.current) shade.current.scrollTop = e.currentTarget.scrollTop;
            }}
            // Where the caret is, for the `@` being typed at it.
            onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
            onChange={(e) => {
              setCaret(e.target.selectionStart);
              const just = sent.current;
              if (just && Date.now() - just.at < 1000) {
                const kind = (e.nativeEvent as InputEvent).inputType;
                if (kind === "insertReplacementText" || e.target.value.trim() === just.text) {
                  e.target.value = "";
                  return;
                }
              }
              sent.current = null;
              setRecalled(null);
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
              if (pressed(e.nativeEvent, "stash")) {
                // Put aside, not saved: the box empties for the next thing.
                e.preventDefault();
                if (text.trim()) {
                  stash.put(text);
                  setText("");
                }
                return;
              }
              if (asking) {
                // A choice by its number, while nothing of your own is typed.
                const n = /^[1-9]$/.test(e.key) && !e.metaKey && !e.ctrlKey && !e.altKey ? Number(e.key) : 0;
                const option = n ? flow.question?.options?.[n - 1] : undefined;
                if (option && !text) {
                  e.preventDefault();
                  flow.toggle(option.label ?? "", () => setText(""));
                  return;
                }
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  if (flow.ready(text)) setText(flow.next(text));
                  return;
                }
              }
              if (shownFound && offered.length === 0) {
                const count = shownFound.paths.length;
                if (count && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
                  e.preventDefault();
                  const step = e.key === "ArrowDown" ? 1 : -1;
                  setMentionActive((mentionActive + step + count) % count);
                  return;
                }
                if (pickedPath && (e.key === "Enter" || e.key === "Tab") && !e.shiftKey) {
                  e.preventDefault();
                  pickMention(pickedPath);
                  return;
                }
                if (e.key === "Escape") {
                  e.preventDefault();
                  setUnmentioned(mention?.start ?? null);
                  return;
                }
              }
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
              } else if (
                e.key === "ArrowUp" &&
                !files.length &&
                history.length > 0 &&
                (text === "" || (recalled !== null && text === history[recalled])) &&
                e.currentTarget.selectionStart === e.currentTarget.selectionEnd &&
                (text === "" || e.currentTarget.selectionStart === text.length)
              ) {
                // Back through what was said here, as a shell's history is.
                e.preventDefault();
                recall(recalled === null ? history.length - 1 : Math.max(0, recalled - 1));
              } else if (
                e.key === "ArrowDown" &&
                recalled !== null &&
                text === history[recalled] &&
                e.currentTarget.selectionStart === text.length
              ) {
                e.preventDefault();
                recall(recalled + 1 < history.length ? recalled + 1 : null);
              } else if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send(steers && steering(e));
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
        {/* T3 Code's footer: what goes with the message on the left, and on
            the right what the next turn runs under, then the one round
            button that sends it (or, while a turn runs and nothing is
            written, stops it). */}
        <div className="chat-controls">
          {state?.files && (
            <>
              <button
                className="chat-ghost-button"
                title="Attach Files"
                aria-label="Attach Files"
                onClick={() => picker.current?.click()}
              >
                <Attach size={16} />
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
          <span className="chat-facts">{state?.status === "compacting" && <span>compacting</span>}</span>
          <ModeControl mode={state?.mode ?? "auto"} onChange={(mode) => onCommand({ op: "mode", mode })} />
          {models.length > 0 && (
            <ModelControl
              agent={agent}
              models={models}
              model={model}
              efforts={efforts}
              effort={effort}
              onModel={(next) => onCommand({ op: "model", model: next })}
              onEffort={(next) => onCommand({ op: "effort", effort: next })}
            />
          )}
          {/* Text only: a file is not something the server keeps for later. */}
          {later === null && what.kind !== "shell" && !files.length && (
            <button
              className="chat-ghost-button"
              title="Send Later, at a Time"
              aria-label="Send Later"
              onClick={() => {
                setLater(localInput(Date.now() + 60 * 60 * 1000));
                box.current?.focus();
              }}
            >
              <Scheduled size={16} />
            </button>
          )}
          {(state?.started || usage) && (
            <ContextRing
              percentage={state?.context_percentage ?? null}
              size={state?.context_size ?? null}
              usage={usage}
            />
          )}
          {asking ? (
            // T3 Code's answer buttons in place of send: back a question, and
            // on to the next or send them all.
            <span className="chat-answer">
              {flow.step > 0 && (
                <button className="chat-answer-back" onClick={() => setText(flow.back(text))}>
                  Previous
                </button>
              )}
              <button
                className="chat-answer-go"
                disabled={!flow.ready(text)}
                onClick={() => setText(flow.next(text))}
              >
                {flow.last ? (flow.questions.length > 1 ? "Submit Answers" : "Submit Answer") : "Next Question"}
              </button>
            </span>
          ) : working && later === null && !text.trim() && !files.length ? (
            <button
              className="chat-round chat-stop"
              title="Stop This Turn (Esc)"
              aria-label="Stop"
              onClick={() => onCommand({ op: "interrupt" })}
            >
              <span className="chat-stop-square" />
            </button>
          ) : (
            <button
              className="chat-round chat-send"
              title={
                later !== null
                  ? "Schedule (Enter)"
                  : what.kind === "shell"
                    ? "Run It in the Sandbox (Enter)"
                    : steers && steerHeld
                      ? `Stop the Turn and Send Now (${STEER_KEYS})`
                      : steers
                        ? `Send After This Turn (Enter), or Stop It and Send Now (${STEER_KEYS})`
                        : "Send (Enter)"
              }
              aria-label={
                later !== null
                  ? "Schedule"
                  : what.kind === "shell"
                    ? "Run"
                    : steers
                      ? steerHeld
                        ? "Stop and send now"
                        : "Send after this turn"
                      : "Send"
              }
              disabled={!sendable}
              onClick={(e) => send(steers && (steerHeld || steering(e)))}
            >
              {later !== null ? (
                <Scheduled size={14} />
              ) : what.kind === "shell" ? (
                <Shell size={14} />
              ) : steers && steerHeld ? (
                <SendSteer size={14} strokeWidth={1.8} />
              ) : steers ? (
                <SendQueued size={14} strokeWidth={1.8} />
              ) : (
                <Send size={14} strokeWidth={1.8} />
              )}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

/// How often a scheduled message is said: once, or on a rule from its first
/// time. See `repeats.ts`.
function AgainControl({
  value,
  day,
  onChange,
}: {
  value: "once" | "daily" | "weekdays" | "weekly" | "hourly";
  /// The first time's weekday, which a weekly one keeps.
  day: string | null;
  onChange: (value: "once" | "daily" | "weekdays" | "weekly" | "hourly") => void;
}) {
  const menu = useButtonMenu();
  const choices = [
    { value: "once", label: "Once" },
    { value: "daily", label: "Every day" },
    { value: "weekdays", label: "Every weekday" },
    { value: "weekly", label: day ? `Every ${day}` : "Every week" },
    { value: "hourly", label: "Every hour" },
  ] as const;
  return (
    <button
      className="chat-later-again"
      aria-haspopup="menu"
      title="How Often to Say It"
      onClick={menu(() =>
        choices.map((c) => ({ label: c.label, checked: c.value === value, run: () => onChange(c.value) })),
      )}
    >
      <Recurring aria-hidden />
      {choices.find((c) => c.value === value)?.label}
    </button>
  );
}

/// The permission mode, as T3 Code's composer draws it: its glyph and its
/// name, borderless until pointed at, and a menu of the four with what each
/// asks before.
function ModeControl({ mode, onChange }: { mode: string; onChange: (mode: string) => void }) {
  const menu = useButtonMenu();
  const current = MODES.find((m) => m.value === mode) ?? MODES[0];
  const Icon = MODE_ICONS[current.value] ?? ModeAuto;
  return (
    <button
      className="chat-control"
      title={`Permission mode: ${current.label}. ${current.hint}`}
      aria-label={`Permission mode: ${current.label}`}
      aria-haspopup="menu"
      onClick={menu(
        () =>
          MODES.map((m) => ({
            label: m.label,
            hint: m.hint,
            checked: m.value === mode,
            run: () => onChange(m.value),
          })),
        { up: true },
      )}
    >
      <Icon size={15} />
      <span>{current.label}</span>
    </button>
  );
}

/// The model and its reasoning effort as one control, the way T3 Code says
/// it: "Opus 5.5 High", the effort in the quieter grey. One menu holds both,
/// the models over the efforts the one in use takes.
function ModelControl({
  agent,
  models,
  model,
  efforts,
  effort,
  onModel,
  onEffort,
}: {
  /// Whose models: its mark goes before the name, as T3 Code puts the
  /// provider's.
  agent: string;
  models: ChatModel[];
  model: string | null;
  efforts: string[];
  effort: string | null;
  onModel: (model: string) => void;
  onEffort: (effort: string | null) => void;
}) {
  const menu = useButtonMenu();
  // Claude Code reports a full id and offers aliases to switch to, so the
  // control names the one running and the menu holds the choices, each with
  // the model it stands for.
  const chosen = model ?? models[0].value;
  const current = models.find((m) => m.value === chosen || m.resolved === chosen);
  // An alias is named as its entry names it ("opus" is "Opus 5.5"); a full id,
  // as `modelName` reads it.
  const name =
    current && (!model || model === current.value) ? current.display_name : modelName(model ?? chosen);
  return (
    <button
      className="chat-control"
      title={`Model: ${name}${effort ? `, ${effortName(effort)} effort` : ""}`}
      aria-label="Model and effort"
      aria-haspopup="menu"
      onClick={menu(
        () => [
          { heading: "Model" },
          ...models.map((m) => ({
            label: m.display_name,
            hint: m.description.split(" · ")[0],
            checked: m === current,
            run: () => onModel(m.value),
          })),
          ...(efforts.length
            ? [
                "separator" as const,
                { heading: "Effort" },
                { label: "Default", hint: "The model's own", checked: !effort, run: () => onEffort(null) },
                ...efforts.map((e) => ({ label: effortName(e), checked: e === effort, run: () => onEffort(e) })),
              ]
            : []),
        ],
        { up: true },
      )}
    >
      <AgentMark agent={agent} title={false} className="chat-control-mark" />
      <span className="chat-control-name">{name}</span>
      {effort && <span className="chat-control-quiet">{effortName(effort)}</span>}
    </button>
  );
}

/// How often the messages waiting on the server for this session are read
/// again. They change when one is said, which is a turn's end at the soonest.
const WAITING_MS = 5_000;

/// The messages waiting on the server to be said to this session's agent, and
/// adding and removing them. From a server without a queue, there are none.
function useWaiting(server: string, name: string, onError: (message: string | null) => void) {
  const [waiting, setWaiting] = useState<QueuedMessage[]>([]);
  const take = useCallback(
    (all: { messages: QueuedMessage[] }) => setWaiting(all.messages.filter((m) => m.session === name)),
    [name],
  );
  useEffect(() => {
    let live = true;
    const read = () =>
      api
        .queue(server)
        .then((q) => live && take(q))
        .catch(() => {});
    void read();
    const timer = setInterval(read, WAITING_MS);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [server, take]);
  const schedule = useCallback(
    async (text: string, at: number, repeat: Repeat | null) => {
      try {
        take(await api.queueMessage(server, name, text, at, repeat));
        onError(null);
      } catch (e) {
        onError(messageOf(e));
        throw e;
      }
    },
    [server, name, take, onError],
  );
  const unqueue = useCallback(
    (m: QueuedMessage) =>
      void api
        .unqueue(server, m.id)
        .then(take)
        .catch((e) => onError(messageOf(e))),
    [server, take, onError],
  );
  return { waiting, schedule, unqueue };
}

/// The messages waiting to be said, above the box they were written in: each
/// with when, and a way to take it back.
function Waiting({
  messages,
  onRemove,
  onEdit,
}: {
  messages: QueuedMessage[];
  onRemove: (m: QueuedMessage) => void;
  /// Take it off the queue and back into the box, to change before sending.
  onEdit: (m: QueuedMessage) => void;
}) {
  if (!messages.length) return null;
  // T3 Code's queued messages: each a dashed bubble where yours go, not yet
  // solid because not yet said, and under it when it will be and the way to
  // take it back.
  return (
    <ul className="chat-waiting" aria-label="Waiting to be said">
      {messages.map((m) => (
        <li key={m.id}>
          <p className="chat-waiting-text" title={m.text}>
            {m.text}
          </p>
          <div className="chat-waiting-meta">
            {m.repeat ? <Recurring /> : m.at !== null ? <Scheduled /> : <Queued />}
            <span
              title={m.repeat && m.runs ? `Said ${m.runs} ${m.runs === 1 ? "time" : "times"} so far` : undefined}
            >
              {m.repeat && m.at !== null
                ? `${describeRepeat(m.repeat)}; next ${startsAt(m.at)}`
                : m.at !== null
                  ? `Said ${startsAt(m.at)}`
                  : "Said after this turn"}
            </span>
            <button className="chat-waiting-remove" title="Back into the Box, to Change" onClick={() => onEdit(m)}>
              <Edit aria-hidden />
              Edit
            </button>
            <button className="chat-waiting-remove" title="Don't Say It" onClick={() => onRemove(m)}>
              <Close aria-hidden />
              Remove
            </button>
          </div>
        </li>
      ))}
    </ul>
  );
}

export function ChatPane({
  server,
  name,
  conv,
  agent,
  nested = false,
  onBranch,
  usage,
  pull,
  onPull,
  jump = null,
  onJumped,
}: {
  server: string;
  name: string;
  conv: string;
  /// Which agent the session runs, for its mark beside the model.
  agent: string;
  /// A side chat's own pane, which has no side chat or edits of its own.
  nested?: boolean;
  /// Start a conversation from just before an earlier message of this one
  /// (`at`, or from nothing when there is nothing before it) and say `text`
  /// there: "Edit from here".
  onBranch?: (from: string | null, at: string | null, text: string) => void;
  /// The session's, from its poll: what the account's limits are.
  usage: Usage | null;
  /// The open pull request the worktree is on, if the inbox knows it.
  pull: Pull | null;
  onPull: (pull: Pull) => void;
  /// An entry a search found here, to open the conversation at. Taken, and
  /// `onJumped` said, so that coming back to the worktree later does not
  /// take the conversation there again.
  jump?: { seq: number; nonce: number } | null;
  onJumped?: () => void;
}) {
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
  // The working copy's paths, for the composer's `@`.
  const findFiles = useCallback((query: string) => api.findFiles(server, name, query), [server, name]);
  // A file a tool call or a reply names, out of this session's sandbox.
  const fetchFile = useCallback((path: string) => void download(server, name, path), [server, name]);

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
        dispatch({ type: "down", reason: messageOf(e) });
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
  const { waiting, schedule, unqueue } = useWaiting(server, name, setRefused);
  // A queued message taken back into the box to be changed.
  const [loaded, setLoaded] = useState<{ text: string; nonce: number; replace?: boolean } | null>(null);

  // A side chat's question or an edited message's words, said as soon as the
  // conversation is open (see `sayFirst`).
  useEffect(() => {
    const words = firstWords.get(key);
    if (!words || !view.state) return;
    firstWords.delete(key);
    command({ op: "send", text: words });
    // Once, when the conversation first says where it is.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, view.state]);

  // T3 Code's side chat: `/btw` asks in a conversation forked from this one,
  // beside it, and closing it ends it.
  const [side, setSide] = useState<string | null>(null);
  const openSide = useCallback(
    (question: string) => {
      api.newChat(server, name, conv, null).then(
        ({ opened }) => {
          if (!opened) return;
          sayFirst(server, name, opened, question);
          setFollowing(null);
          setSide(opened);
        },
        (e) => setRefused(messageOf(e)),
      );
    },
    [server, name, conv],
  );
  const closeSide = () => {
    if (side) {
      api.closeChat(server, name, side).catch(() => {});
      forgetChat(server, name, side);
    }
    setSide(null);
  };

  // "Edit from here": the message's words back in the box, and sending them
  // forks this conversation from just before it.
  const [branchFrom, setBranchFrom] = useState<number | null>(null);
  const editPrompt = useMemo(
    () =>
      onBranch && !nested
        ? (key: string, text: string) => {
            setBranchFrom(Number(key));
            setLoaded({ text, nonce: Date.now(), replace: true });
          }
        : null,
    [onBranch, nested],
  );
  const branch = (text: string) => {
    if (branchFrom === null || !onBranch) return;
    const at = assistantBefore(view.entries, branchFrom);
    onBranch(at ? conv : null, at, text);
    setBranchFrom(null);
  };
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
      (e) => setRefused(messageOf(e)),
    );
  };
  // Only a drag carrying files is one to take: a tab being dragged past is not.
  const carriesFiles = (e: React.DragEvent) => e.dataTransfer.types.includes("Files");

  const receiptsOn = view.state?.receipts ?? false;
  const { items: built, queued, agents, commands } = useMemo(
    () => (view.caughtUp ? build(view.entries) : build([])),
    [view.caughtUp, view.entries],
  );
  const working = view.state?.activity === "running" || view.state?.activity === "waiting";
  // The rows that have not changed kept as they were, so only the new ones draw.
  const settled = useRef<Item[]>([]);
  const items = useMemo(() => (settled.current = settle(settled.current, group(built, working))), [built, working]);
  const started = useMemo(() => (working ? turnStart(view.entries) : null), [working, view.entries]);
  const liveFold = items.some((i) => i.kind === "steps" && i.live);
  // What you said here, for the composer's up arrow: each prompt once, in the
  // order said, without a repeat of the one just before it.
  const history = useMemo(() => {
    const said: string[] = [];
    for (const { ev } of view.entries) {
      if (ev.kind !== "prompt" || !ev.text.trim() || said[said.length - 1] === ev.text) continue;
      said.push(ev.text);
    }
    return said;
  }, [view.entries]);
  const [target, setTarget] = useState<{ seq: number; nonce: number } | null>(null);
  useEffect(() => {
    if (!jump) return;
    setTarget(jump);
    onJumped?.();
  }, [jump, onJumped]);
  const located = useMemo(() => {
    const at = target && locate(items, target.seq);
    return at && target ? { ...at, nonce: target.nonce } : null;
  }, [items, target]);
  const [following, setFollowing] = useState<string | null>(null);
  const followed = agents.find((a) => a.id === following) ?? null;
  const followedCommand = commands.find((c) => c.id === following) ?? null;
  // A background command's output, from its file in the sandbox.
  const readTail = useCallback((path: string) => api.tailFile(server, name, path), [server, name]);
  const openAgent = useMemo(
    () => (callId: string) => {
      const a = agents.find((x) => x.call?.id === callId);
      if (a) setFollowing(a.id);
      else if (commands.some((c) => c.id === callId)) setFollowing(callId);
    },
    [agents, commands],
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
  const question = pending.find((ask) => ask.tool === "AskUserQuestion") ?? null;
  const answerQuestion = useCallback(
    (decision: ChatDecision) => question && command({ op: "answer", id: question.id, decision }),
    [question, command],
  );
  const receipts = !receiptsOn ? "off" : view.state?.activity === "idle" ? "idle" : "working";
  const nothing = items.length === 0 && queued.length === 0 && !view.live;
  // A `!` command running says so in its own card.
  const last = items.at(-1);
  const shellRunning = last?.kind === "shell" && last.output === null;

  return (
    <div className="chat-split">
      <Ticking.Provider value={working}>
      <OpenAgent.Provider value={openAgent}>
      <EditPrompt.Provider value={editPrompt}>
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
              <Found.Provider value={located}>
              <div className="chat-log" ref={log}>
                {nothing &&
                  (view.notice ? (
                    <Empty icon={Working} note={view.notice} />
                  ) : !view.caughtUp && view.entries.length > 0 ? (
                    <Empty icon={Working} />
                  ) : view.state ? (
                    <Empty icon={ChatGlyph} />
                  ) : null)}
                <Downloader.Provider value={fetchFile}>
                <Pictures.Provider value={readPicture}>
                  <Items items={items} receipts={receipts} />
                  <Writing live={view.live} entries={view.entries} />
                </Pictures.Provider>
                </Downloader.Provider>
                {/* Under whatever is being written, for the whole turn, so a reply
                    that has paused between two tool calls still says it is not
                    finished. The word only while there is nothing else to look at. */}
                {/* T3 Code's line for a turn under way: "Working for" and the
                    clock, quiet, with no spinner. Not while the turn's fold above
                    is already saying it. */}
                {view.state?.activity === "running" && pending.length === 0 && !shellRunning && !liveFold && (
                  <p className="chat-working" role="status">
                    {started !== null ? (
                      <>
                        Working for <Since start={started} />
                      </>
                    ) : (
                      "Working…"
                    )}
                  </p>
                )}
                {/* Permission prompts and plans here, where the work is; a
                    question is asked from the composer (`chat/Questions.tsx`). */}
                {pending
                  .filter((ask) => ask.tool !== "AskUserQuestion")
                  .map((ask) => (
                    <AskCard key={ask.id} ask={ask} onAnswer={(decision) => command({ op: "answer", id: ask.id, decision })} />
                  ))}
                {/* Sent and not yet read: after everything the turn in progress is
                    still doing, since Claude reads them when it is done. */}
                <Items items={queued} receipts={receipts} />
              </div>
              </Found.Provider>
            </div>
            {/* Your messages waiting to be said end the conversation, where
                they will be said; the work going on beside it and the pull
                request sit on the box, apart from them. */}
            <Waiting
              messages={waiting}
              onRemove={unqueue}
              onEdit={(m) => {
                unqueue(m);
                setLoaded({ text: m.text, nonce: Date.now() });
              }}
            />
            <AgentsBar agents={agents} commands={commands} open={following} onOpen={setFollowing} />
            <PullBar pull={pull} onOpen={onPull} />
            {(view.down || view.error || refused) && (
              <p className="error chat-banner">{view.down ?? refused ?? view.error}</p>
            )}
            <Composer
              draftKey={key}
              history={history}
              state={view.state}
              usage={usage}
              onCommand={command}
              files={files}
              onAttach={attach}
              onRemove={(key) => setFiles((had) => had.filter((f) => f.key !== key))}
              onSent={() => {
                setFiles([]);
                setRefused(null);
              }}
              onSchedule={schedule}
              findFiles={findFiles}
              onSideChat={nested ? undefined : openSide}
              branching={branchFrom !== null}
              onBranch={branch}
              onCancelBranch={() => setBranchFrom(null)}
              load={loaded}
              question={question}
              onAnswer={answerQuestion}
              agent={agent}
            />
          </div>
        </Unpin.Provider>
      </EditPrompt.Provider>
      </OpenAgent.Provider>
      {followed && (
        <Downloader.Provider value={fetchFile}>
        <Pictures.Provider value={readPicture}>
          <AgentPanel agent={followed} progress={view.tasks.get(followed.id)} onClose={() => setFollowing(null)} />
        </Pictures.Provider>
        </Downloader.Provider>
      )}
      {followedCommand && (
        <CommandPanel command={followedCommand} readTail={readTail} onClose={() => setFollowing(null)} />
      )}
      {side && !followed && !followedCommand && (
        <aside className="chat-agent-panel chat-side-panel" aria-label="Side chat">
          <header>
            <SideGlyph aria-hidden />
            <h3>Side chat</h3>
            <button className="chat-agent-close" title="Close, and End the Side Chat" onClick={closeSide}>
              <Close aria-label="Close" />
            </button>
          </header>
          <p className="chat-agent-facts">With this conversation's context, which goes on untouched</p>
          <div className="chat-side-body">
            <ChatPane server={server} name={name} conv={side} agent={agent} usage={usage} pull={null} onPull={onPull} nested />
          </div>
        </aside>
      )}
      </Ticking.Provider>
    </div>
  );
}
