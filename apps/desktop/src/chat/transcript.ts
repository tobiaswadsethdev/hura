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
import type {
  AssistantMessage,
  Block,
  ResultMessage,
  SdkMessage,
  SystemMessage,
  UserMessage,
} from "../sdk";

/// `stopped` is a call that never finished because you interrupted the turn,
/// which Claude Code reports as an error and which is not one.
export type ToolResult = { text: string; isError: boolean; stopped: boolean };

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
  | { kind: "prompt"; key: string; text: string }
  | { kind: "text"; key: string; text: string }
  | { kind: "thinking"; key: string; text: string }
  | { kind: "tool"; key: string; call: ToolCall }
  | { kind: "note"; key: string; text: string; tone: "dim" | "bad" }
  | { kind: "turn"; key: string; ok: boolean; text: string }
  | { kind: "divider"; key: string; text: string };

/// The text in a tool result, however it was sent.
function resultText(content: Extract<Block, { type: "tool_result" }>["content"]): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((part) => (part.type === "text" ? (part.text ?? "") : `[${part.type}]`))
    .join("\n");
}

function seconds(ms: number | undefined): string {
  if (ms === undefined) return "";
  return ms < 60_000 ? `${Math.round(ms / 1000)}s` : `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
}

function dollars(usd: number | undefined): string {
  return usd === undefined ? "" : `$${usd < 0.01 ? usd.toFixed(3) : usd.toFixed(2)}`;
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

export function build(entries: ChatEntry[]): Item[] {
  const items: Item[] = [];
  const calls = new Map<string, ToolCall>();

  /// Where a message's items go: inside the call that started its subagent,
  /// or the transcript itself.
  const into = (parent: string | null): Item[] =>
    (parent !== null && calls.get(parent)?.children) || items;

  for (const entry of entries) {
    const ev = entry.ev;
    const key = String(entry.seq);
    switch (ev.kind) {
      case "prompt":
        items.push({ kind: "prompt", key, text: ev.text });
        break;
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
      case "sdk":
        message(ev.msg as SdkMessage, key);
        break;
    }
  }
  return items;

  function message(msg: SdkMessage, key: string) {
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
              call.result = { text, isError, stopped: isError && INTERRUPTED.test(text) };
            }
          } else if (block.type === "text" && "text" in block) {
            note(m, block.text, `${key}.${i}`);
          }
        });
        break;
      }
      case "result": {
        const r = msg as ResultMessage;
        const facts = [seconds(r.duration_ms), dollars(r.total_cost_usd)].filter(Boolean).join(" · ");
        if (r.subtype === "success" && !r.is_error) {
          items.push({ kind: "turn", key, ok: true, text: facts });
        } else if (r.subtype === "error_during_execution") {
          // What an interrupt ends with. Stopping a turn is not a failure of it.
          items.push({ kind: "turn", key, ok: true, text: ["stopped", facts].filter(Boolean).join(" · ") });
        } else {
          const why = r.subtype.replace(/^error_/, "").replaceAll("_", " ");
          items.push({ kind: "turn", key, ok: false, text: [why, facts].filter(Boolean).join(" · ") });
        }
        break;
      }
      case "system":
        system(msg as SystemMessage, key);
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

  function system(m: SystemMessage, key: string) {
    const text = (field: string) => (typeof m[field] === "string" ? (m[field] as string) : "");
    switch (m.subtype) {
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
