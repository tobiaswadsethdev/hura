// The parts of the Agent SDK's messages this window reads.
//
// The messages themselves are Claude Code's: the transcript carries them whole,
// and the generated types call them `unknown` because hura does not model
// them. These are the fields the chat pane draws from, written down from
// `@anthropic-ai/claude-agent-sdk`'s own `sdk.d.ts` (0.3.289) rather than
// installed from it, because the package brings a Claude Code binary with it
// and the window needs a dozen field names. Every field is optional on
// purpose: the shapes grow, and a pane that insists on one breaks on an
// upgrade instead of showing less.

export type Block =
  | { type: "text"; text: string }
  | { type: "thinking"; thinking: string }
  | { type: "redacted_thinking" }
  | { type: "tool_use"; id: string; name: string; input: Record<string, unknown> }
  | {
      type: "tool_result";
      tool_use_id: string;
      content?: string | Array<{ type: string; text?: string }>;
      is_error?: boolean;
    }
  | { type: "image" }
  | { type: string };

export type AssistantMessage = {
  type: "assistant";
  uuid?: string;
  parent_tool_use_id: string | null;
  message: { id?: string; model?: string; content?: Block[] };
  error?: string;
};

export type UserMessage = {
  type: "user";
  uuid?: string;
  parent_tool_use_id: string | null;
  message: { content?: string | Block[] };
  isSynthetic?: boolean;
  isReplay?: boolean;
};

export type ResultMessage = {
  type: "result";
  subtype: string;
  is_error?: boolean;
  duration_ms?: number;
  num_turns?: number;
  total_cost_usd?: number;
  result?: string;
};

export type SystemMessage = {
  type: "system";
  subtype: string;
  [field: string]: unknown;
};

export type SdkMessage = AssistantMessage | UserMessage | ResultMessage | SystemMessage | { type: string };

/// The raw API stream events a `stream_event` carries, as far as the pane
/// follows them: enough to draw text and thinking as they are written.
export type StreamEvent =
  | { type: "message_start"; message: { id: string } }
  | {
      type: "content_block_start";
      index: number;
      content_block: { type: string; name?: string };
    }
  | {
      type: "content_block_delta";
      index: number;
      delta:
        | { type: "text_delta"; text: string }
        | { type: "thinking_delta"; thinking: string }
        | { type: "input_json_delta"; partial_json: string }
        | { type: string };
    }
  | { type: "content_block_stop"; index: number }
  | { type: "message_delta" }
  | { type: "message_stop" };

export type PartialMessage = {
  type: "stream_event";
  event: StreamEvent;
  parent_tool_use_id: string | null;
};

export type ToolProgress = {
  type: "tool_progress";
  tool_use_id: string;
  tool_name: string;
  elapsed_time_seconds: number;
};
