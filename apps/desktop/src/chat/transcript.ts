// A conversation's transcript, as things to draw.
//
// The host keeps what happened as it happened: SDK messages whole, your own
// messages as you wrote them, and how each permission request was answered.
// That is the right thing to keep and the wrong thing to draw, because one
// tool call is spread over three of those entries: the assistant message that
// makes it, the user message carrying its result, and the answer to the
// request in between. This folds them back into one item each, with a
// subagent's work inside the call that started it.
//
// Pure, and rebuilt whole from the entries. A transcript is hundreds of
// entries rather than millions, and an incremental fold would be a second
// definition of the same thing to keep correct.

import type { ChatEntry } from "../gen/ChatEntry";
import type { ChatFile } from "../gen/ChatFile";
import type {
  AssistantMessage,
  Block,
  ResultMessage,
  SdkMessage,
  SystemMessage,
  UserMessage,
} from "../sdk";
import type { Picture } from "../gen/Picture";

/// `stopped` is a call that never finished because you interrupted the turn,
/// which Claude Code reports as an error and which is not one.
export type ToolResult = { text: string; isError: boolean; stopped: boolean; images: Picture[] };

/// What Claude Code puts in a tool result it cut short at an interrupt.
const INTERRUPTED = /^(The user doesn't want to proceed with this tool use|\[Request interrupted)/;

export type ToolCall = {
  id: string;
  name: string;
  input: Record<string, unknown>;
  /// `null` while it is running.
  result: ToolResult | null;
  /// What a subagent did inside this call, when the call started one.
  children: Item[];
  /// How a permission request for it was answered, when there was one.
  answer: {
    allow: boolean;
    always: boolean;
    message: string | null;
    answers: { [question in string]?: string } | null;
  } | null;
};

export type Item =
  /// `at` is when it was sent and `read` when Claude Code took it into a turn,
  /// both epoch milliseconds from the sandbox's clock; `read` is null until
  /// then.
  | { kind: "prompt"; key: string; text: string; files: ChatFile[]; at: number; read: number | null }
  /// A `!` command you ran in the sandbox. `output` is null while it runs;
  /// `code` is null when it did not exit by itself.
  | {
      kind: "shell";
      key: string;
      command: string;
      at: number;
      output: string | null;
      code: number | null;
      stopped: boolean;
      truncated: boolean;
    }
  | { kind: "text"; key: string; text: string }
  | { kind: "thinking"; key: string; text: string }
  | { kind: "tool"; key: string; call: ToolCall }
  | { kind: "note"; key: string; text: string; tone: "dim" | "bad" }
  | { kind: "turn"; key: string; ok: boolean; text: string; at: number }
  | { kind: "divider"; key: string; text: string }
  /// The first thing said on a day, so a time is never read as the wrong one.
  | { kind: "day"; key: string; at: number };

/// The same calendar day, where this window is.
function sameDay(a: number, b: number): boolean {
  return new Date(a).toDateString() === new Date(b).toDateString();
}

/// The text in a tool result, however it was sent. An image is left out:
/// it is drawn, by `resultImages`.
function resultText(content: Extract<Block, { type: "tool_result" }>["content"]): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .filter((part) => part.type !== "image")
    .map((part) => (part.type === "text" ? (part.text ?? "") : `[${part.type}]`))
    .join("\n");
}

/// The images in a tool result: what `Read` answers with for a picture, and a
/// screenshot from a browser tool.
function resultImages(content: Extract<Block, { type: "tool_result" }>["content"]): Picture[] {
  if (!Array.isArray(content)) return [];
  return content.flatMap((part) => {
    const source = (part as { source?: { type?: string; media_type?: string; data?: string } }).source;
    return part.type === "image" && source?.type === "base64" && source.media_type && source.data
      ? [{ mime: source.media_type, data: source.data }]
      : [];
  });
}

function seconds(ms: number | undefined): string {
  if (ms === undefined) return "";
  return ms < 60_000 ? `${Math.round(ms / 1000)}s` : `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
}

/// Why an assistant message carries an error, in words.
const ERRORS: Record<string, string> = {
  authentication_failed: "Claude Code is not logged in: this session has no Claude credential attached",
  rate_limit: "rate limited",
  overloaded: "the API is overloaded",
  billing_error: "a billing error",
  max_output_tokens: "the reply hit its length limit",
  model_not_found: "the model was not found",
};

/// A subagent the conversation started, from the task events Claude Code
/// keeps in the transcript. What it did is its call's `children`, which is
/// where its messages are folded, so following one is reading those.
export type Agent = {
  /// The task's id, which a resumed agent keeps.
  id: string;
  /// The call that started it, or null while that call has not arrived.
  call: ToolCall | null;
  description: string;
  /// `general-purpose`, `Explore` or a custom agent's name.
  type: string | null;
  /// Started in the background, so it outlives the turn that started it.
  background: boolean;
  status: "running" | "completed" | "failed" | "stopped";
  started: number;
  ended: number | null;
  prompt: string;
  usage: { total_tokens: number; tool_uses: number; duration_ms: number } | null;
};

/// The transcript as things to draw, your messages Claude has not read yet,
/// which are drawn after everything else, the reply being written too, and
/// the subagents, oldest first.
export type Built = { items: Item[]; queued: Item[]; agents: Agent[] };

/// A task's status in the words of one of four: `killed` is a stop, and
/// `pending` and `paused` are still going.
function status(s: unknown): Agent["status"] {
  switch (s) {
    case "completed":
      return "completed";
    case "failed":
      return "failed";
    case "stopped":
    case "killed":
      return "stopped";
  }
  return "running";
}

/// A message is placed where Claude read it rather than where it was sent: one
/// sent mid-turn arrives in the transcript after that turn, beside the reply to
/// it, and until then waits at the bottom. Only a message the host promised a
/// receipt for waits; one kept before the host gave them stays where it was
/// sent. So does one older than the last message read, which was dropped
/// rather than read: a host restarted with it queued never delivered it.
export function build(entries: ChatEntry[]): Built {
  const items: Item[] = [];
  const calls = new Map<string, ToolCall>();
  const read = new Set<string>();
  let lastRead = 0;
  for (const e of entries) {
    if (e.ev.kind === "read") {
      read.add(e.ev.uuid);
      lastRead = e.seq;
    }
  }
  /// Waiting for their receipt, to be placed where it comes.
  const held = new Map<string, Extract<Item, { kind: "prompt" }>>();
  const agents = new Map<string, Agent & { callId: string | null }>();

  /// Where a message's items go: inside the call that started its subagent,
  /// or the transcript itself.
  const into = (parent: string | null): Item[] =>
    (parent !== null && calls.get(parent)?.children) || items;
  /// Your messages, so their receipts can be filled in as they arrive.
  const prompts = new Map<string, Extract<Item, { kind: "prompt" }>>();
  /// Your `!` commands, so each one's end can be filled in when it comes.
  const shells = new Map<string, Extract<Item, { kind: "shell" }>>();
  let day: number | null = null;

  for (const entry of entries) {
    const ev = entry.ev;
    const key = String(entry.seq);
    switch (ev.kind) {
      case "prompt": {
        const prompt = { kind: "prompt" as const, key, text: ev.text, files: ev.files, at: entry.at, read: null };
        prompts.set(ev.uuid, prompt);
        if (read.has(ev.uuid) || (ev.receipt && entry.seq > lastRead)) held.set(ev.uuid, prompt);
        else place(prompt, entry.at);
        break;
      }
      case "read": {
        const prompt = prompts.get(ev.uuid);
        if (!prompt) break;
        prompt.read = entry.at;
        if (held.delete(ev.uuid)) place(prompt, entry.at);
        break;
      }
      case "answer": {
        const call = calls.get(ev.id);
        const answer = { allow: ev.allow, always: ev.always, message: ev.message, answers: ev.answers };
        if (call) call.answer = answer;
        else if (!ev.allow) {
          items.push({ kind: "note", key, tone: "dim", text: `declined ${ev.tool}${ev.message ? `: ${ev.message}` : ""}` });
        }
        break;
      }
      case "error":
        items.push({ kind: "note", key, tone: "bad", text: ev.message });
        break;
      case "shell": {
        const shell = {
          kind: "shell" as const,
          key,
          command: ev.command,
          at: entry.at,
          output: null,
          code: null,
          stopped: false,
          truncated: false,
        };
        shells.set(ev.uuid, shell);
        place(shell, entry.at);
        break;
      }
      case "exited": {
        const shell = shells.get(ev.uuid);
        if (!shell) break;
        shell.output = ev.output;
        shell.code = ev.code;
        shell.stopped = ev.stopped;
        shell.truncated = ev.truncated;
        break;
      }
      case "sdk":
        message(ev.msg as SdkMessage, key, entry.at);
        break;
    }
  }
  return {
    items,
    queued: [...held.values()],
    agents: [...agents.values()].map(({ callId, ...a }) => ({ ...a, call: (callId && calls.get(callId)) || null })),
  };

  /// A message into the transcript at `at`, the first of its day under a line
  /// naming the day.
  function place(prompt: Item, at: number) {
    if (day === null || !sameDay(day, at)) items.push({ kind: "day", key: `${prompt.key}.day`, at });
    day = at;
    items.push(prompt);
  }

  function message(msg: SdkMessage, key: string, at: number) {
    switch (msg.type) {
      case "assistant": {
        const m = msg as AssistantMessage;
        const list = into(m.parent_tool_use_id);
        // Claude Code reporting a failure in a message of its own, like "Not
        // logged in". Its words, in the colour of a failure, once.
        if (m.error) {
          const said = (m.message?.content ?? [])
            .map((b) => (b.type === "text" && "text" in b ? b.text : ""))
            .join("\n")
            .trim();
          list.push({ kind: "note", key, tone: "bad", text: ERRORS[m.error] ?? (said || m.error) });
          break;
        }
        (m.message?.content ?? []).forEach((block, i) => {
          const k = `${key}.${i}`;
          if (block.type === "text" && "text" in block && block.text.trim()) {
            list.push({ kind: "text", key: k, text: block.text });
          } else if (block.type === "thinking" && "thinking" in block && block.thinking.trim()) {
            list.push({ kind: "thinking", key: k, text: block.thinking });
          } else if (block.type === "tool_use" && "id" in block) {
            const call: ToolCall = {
              id: block.id,
              name: block.name,
              input: block.input ?? {},
              result: null,
              children: [],
              answer: null,
            };
            calls.set(block.id, call);
            list.push({ kind: "tool", key: k, call });
          }
        });
        break;
      }
      case "user": {
        const m = msg as UserMessage;
        const content = m.message?.content;
        if (typeof content === "string") {
          note(m, content, key);
          break;
        }
        (content ?? []).forEach((block, i) => {
          if (block.type === "tool_result" && "tool_use_id" in block) {
            const call = calls.get(block.tool_use_id);
            if (call) {
              const text = resultText(block.content);
              const isError = Boolean(block.is_error);
              call.result = {
                text,
                isError,
                stopped: isError && INTERRUPTED.test(text),
                images: resultImages(block.content),
              };
            }
          } else if (block.type === "text" && "text" in block) {
            note(m, block.text, `${key}.${i}`);
          }
        });
        break;
      }
      case "result": {
        const r = msg as ResultMessage;
        const facts = seconds(r.duration_ms);
        if (r.subtype === "success" && !r.is_error) {
          items.push({ kind: "turn", key, ok: true, text: facts, at });
        } else if (r.subtype === "error_during_execution") {
          // What an interrupt ends with. Stopping a turn is not a failure of it.
          items.push({ kind: "turn", key, ok: true, text: ["stopped", facts].filter(Boolean).join(" · "), at });
        } else {
          const why = r.subtype.replace(/^error_/, "").replaceAll("_", " ");
          items.push({ kind: "turn", key, ok: false, text: [why, facts].filter(Boolean).join(" · "), at });
        }
        break;
      }
      case "system":
        system(msg as SystemMessage, key, at);
        break;
      case "conversation_reset":
        items.push({ kind: "divider", key, text: "conversation cleared" });
        break;
    }
  }

  /// Text in a user message that is not yours: a synthetic note from Claude
  /// Code, like an interrupt, or a skill's content. Yours is a prompt entry.
  function note(m: UserMessage, text: string, key: string) {
    const t = text.trim();
    if (!t) return;
    if (m.parent_tool_use_id !== null) return;
    items.push({ kind: "note", key, tone: "dim", text: t.length > 400 ? `${t.slice(0, 400)}…` : t });
  }

  function system(m: SystemMessage, key: string, at: number) {
    const text = (field: string) => (typeof m[field] === "string" ? (m[field] as string) : "");
    switch (m.subtype) {
      // Subagents only: a background shell's output is a file in the sandbox,
      // not a conversation, and an ambient task is housekeeping.
      case "task_started": {
        if (m.task_type !== "local_agent" || m.skip_transcript || m.ambient) break;
        const id = text("task_id");
        const had = agents.get(id);
        agents.set(id, {
          id,
          // A resumed agent's messages stay under the call that first started it.
          callId: had?.callId ?? (text("tool_use_id") || null),
          call: null,
          description: text("description") || had?.description || "agent",
          type: text("subagent_type") || null,
          background: Boolean(m.is_backgrounded),
          status: "running",
          started: had?.started ?? at,
          ended: null,
          prompt: had?.prompt || text("prompt"),
          usage: had?.usage ?? null,
        });
        break;
      }
      case "task_updated": {
        const agent = agents.get(text("task_id"));
        const patch = (m.patch ?? {}) as { status?: string; end_time?: number; description?: string };
        if (!agent) break;
        if (patch.status) agent.status = status(patch.status);
        if (patch.end_time) agent.ended = patch.end_time;
        if (patch.description) agent.description = patch.description;
        break;
      }
      case "task_notification": {
        const agent = agents.get(text("task_id"));
        if (!agent) break;
        agent.status = status(m.status);
        agent.ended ??= at;
        if (m.usage) agent.usage = m.usage as Agent["usage"];
        break;
      }
      case "compact_boundary":
        items.push({ kind: "divider", key, text: "context compacted" });
        break;
      case "api_retry":
        items.push({
          kind: "note",
          key,
          tone: "dim",
          text: `retrying${m.error_status ? ` after ${m.error_status}` : ""} (attempt ${m.attempt} of ${m.max_retries})`,
        });
        break;
      case "informational":
      case "local_command_output":
        if (text("content")) items.push({ kind: "note", key, tone: "dim", text: text("content") });
        break;
      case "notification":
        if (text("text")) items.push({ kind: "note", key, tone: "dim", text: text("text") });
        break;
      case "permission_denied":
        items.push({
          kind: "note",
          key,
          tone: "dim",
          text: `${text("tool_name")} was not allowed${text("message") ? `: ${text("message")}` : ""}`,
        });
        break;
    }
  }
}

/// How many content blocks of one assistant message have arrived whole, so the
/// live copy of that message can stop drawing them.
export function arrived(entries: ChatEntry[], messageId: string): number {
  let n = 0;
  for (let i = entries.length - 1; i >= 0 && i >= entries.length - 50; i--) {
    const ev = entries[i].ev;
    if (ev.kind !== "sdk") continue;
    const m = ev.msg as AssistantMessage;
    if (m.type === "assistant" && m.parent_tool_use_id === null && m.message?.id === messageId) {
      n += m.message.content?.length ?? 0;
    }
  }
  return n;
}
