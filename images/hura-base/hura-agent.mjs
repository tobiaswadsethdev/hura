#!/usr/bin/env -S node --disable-warning=UNDICI-EHPA
// hura-agent: Claude Code driven through the Agent SDK, from inside the sandbox.
//
// A chat session's agent is not a terminal. This process owns it instead: one
// or more conversations, each a `query()` from @anthropic-ai/claude-agent-sdk
// in streaming input mode, each running the image's own /usr/local/bin/claude.
// So the process that reaches api.anthropic.com is still inside the sandbox's
// policy, the version is still the image's, and the credential is still the
// one the runtime's proxy swaps in. Running the SDK on the host would have put the
// agent outside the sandbox, which is the one thing hura exists to prevent.
//
// It lives under tmux for the reason the terminal agent does: it survives the
// connection that started it, and `hurad attach` can still look at what it
// printed. Clients reach it on a loopback port, through the same
// `hurad relay` a preview travels over.
//
// What it keeps, it keeps on disk under /sandbox/.hura/chat: which
// conversations exist, the Claude session id behind each, and every message
// each one produced. A window that opens later replays the transcript, and a
// restarted host resumes each conversation by its id.
//
// It also writes the two files the status poll reads, status.json and
// usage.json, in the shapes `hura-status` and the status line write for a
// terminal agent. So the list, the notification and the spend strip work the
// same for both kinds of session, and this side knows exactly when it is
// waiting, where a terminal has to be read off the screen.
//
//   hura-agent serve [--port N] [--cwd DIR] [--task-file FILE]
//   hura-agent list               names, one JSON array
//   hura-agent new                opens a conversation, prints its name
//   hura-agent close NAME         ends one; `agent` cannot be closed
//   hura-agent send NAME          stdin, as one message from you
//
// A message can carry files: a screenshot dropped on the window's composer, a
// photo from the phone. Each is written under /sandbox/.hura/uploads, outside
// the working copy so nothing sent is committed by accident, and the message
// tells the agent where. A picture the API takes goes to the model as well, as
// an image block, which is what Claude Code does with one pasted into its
// terminal; the agent sees it without spending a tool call on reading it.
//
// A message that starts with `!` is a command for the sandbox's shell, as in
// Claude Code's terminal: run here in the working copy, kept in the transcript
// with what it printed, and handed to Claude with the next message without a
// turn of its own. The window could run it in a shell tab instead; doing it
// here is what puts the output in front of the model.
//
// Each message you send is kept as you wrote it, and once Claude Code takes it
// into a turn a `read` entry says so. Claude Code acknowledges a message on
// stdout when it starts the turn that carries it, not when it reads it from
// stdin (asked for with --replay-user-messages), so a message queued behind a
// running turn stays unread until that turn is over: which is what a receipt
// is for.
//
// Nothing here is a security boundary, and nothing could be: the agent runs as
// the same user and can reach this port, just as a terminal agent can type into
// its own tmux pane. The sandbox policy is the boundary.

import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

/// Where clients find this process. Fixed rather than chosen, because the
/// server dials it without asking, and kept in step with `chat::PORT`.
export const PORT = 47681;
/// The port this host is reached on: [`PORT`] in a sandbox, and its own on a
/// server where several worktree sessions' hosts share one loopback.
const LISTEN = Number(process.env.HURA_AGENT_PORT || PORT);
/// The conversation the seeder starts with the task. The one tab a chat
/// session always has, which is why it cannot be closed.
export const AGENT = "agent";

const HURA = process.env.HURA_DIR ?? "/sandbox/.hura";
const CLAUDE = process.env.HURA_CLAUDE ?? "/usr/local/bin/claude";
const STATUS_HEARTBEAT_MS = 30_000;

/// SDK messages that describe a moment rather than the conversation: kept out
/// of the transcript and sent to whoever is watching. A replay of a long
/// session would otherwise be mostly token deltas and timers.
const LIVE = new Set(["stream_event", "tool_progress", "rate_limit_event", "prompt_suggestion", "command_lifecycle"]);
const LIVE_SYSTEM = new Set([
  "session_state_changed",
  "status",
  "hook_started",
  "hook_progress",
  "hook_response",
  "task_progress",
  "thinking_tokens",
  "background_tasks_changed",
  "commands_changed",
]);

/// The most a replay frame carries, about. A client takes a frame as one
/// WebSocket message and caps those (the phone at 64 MiB), and a session that
/// screenshots a lot outgrows that: the transcript is mostly images.
const REPLAY_BYTES = 4 << 20;

/// The transcript after `since`, as replay frames of about `limit` bytes each.
/// Each says the seq of its own last entry rather than the transcript's, so a
/// connection that drops between them asks again from there. The last carries
/// `seq`, and there is always one, even with nothing to replay.
export function replayFrames(entries, since, seq, limit = REPLAY_BYTES) {
  const frames = [];
  let chunk = [];
  let size = 0;
  for (const e of entries) {
    if (e.seq <= since) continue;
    const n = JSON.stringify(e).length;
    if (chunk.length && size + n > limit) {
      frames.push({ t: "replay", seq: chunk.at(-1).seq, entries: chunk });
      chunk = [];
      size = 0;
    }
    chunk.push(e);
    size += n;
  }
  frames.push({ t: "replay", seq, entries: chunk });
  return frames;
}

/// The most a message's files may come to, decoded. A client sends them inline
/// in one WebSocket message, and hurad takes frames of up to 16 MiB; base64 is
/// four thirds of this.
export const UPLOAD_BYTES = 10 << 20;
/// Pictures the API takes as an image block, and the most it takes of one,
/// measured as base64. A larger one is still saved, for the agent to `Read`:
/// Claude Code scales a picture down when it reads one.
const PICTURE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);
const PICTURE_BASE64 = 5 << 20;

/// A file's name, reduced to something that can only name a file in the
/// directory it is written to: the last component, with nothing that walks out
/// of it or hides it.
export function uploadName(name) {
  const base = String(name ?? "")
    .split(/[\\/]/)
    .pop()
    .replace(/[\u0000-\u001f]/g, "")
    .replace(/^\.+/, "")
    .trim();
  return base.slice(0, 200) || "file";
}

/// What a message with files is: where each was saved, which of them go to the
/// model as pictures, and the SDK content that says both. `files` is what a
/// client sent, `{name, mime, data}` with the data in base64; `dir` is where
/// this message's files go. Writes nothing: the caller does, with `bytes`.
export function attach(text, files, dir) {
  const taken = new Set();
  let total = 0;
  const saved = files.map((f) => {
    const bytes = Buffer.from(String(f.data ?? ""), "base64");
    total += bytes.length;
    let name = uploadName(f.name);
    // Two files dropped with one name are two files.
    for (let n = 2; taken.has(name); n++) {
      const ext = path.extname(uploadName(f.name));
      name = `${path.basename(uploadName(f.name), ext)}-${n}${ext}`;
    }
    taken.add(name);
    const mime = String(f.mime || "application/octet-stream");
    const data = bytes.toString("base64");
    const picture = PICTURE_TYPES.has(mime) && data.length <= PICTURE_BASE64;
    return { name, path: path.join(dir, name), mime, size: bytes.length, bytes, data: picture ? data : null };
  });
  if (total > UPLOAD_BYTES) {
    throw new Error(`the files come to ${Math.ceil(total / (1 << 20))} MiB; a message takes ${UPLOAD_BYTES >> 20} MiB`);
  }
  const listed = saved.map((f) => `- ${f.path} (${f.mime}, ${size(f.size)})`).join("\n");
  const said = [text, `Files sent with this message, saved at:\n${listed}`].filter(Boolean).join("\n\n");
  const content = [
    ...saved
      .filter((f) => f.data)
      .map((f) => ({ type: "image", source: { type: "base64", media_type: f.mime, data: f.data } })),
    { type: "text", text: said },
  ];
  return { saved, content };
}

function size(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1 << 20) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1 << 20)).toFixed(1)} MB`;
}

/// Claude Code's slash commands in the protocol's shape (`SlashCommand` in
/// hura-core): from `supportedCommands()` or `commands_changed`, which
/// describe each, or the bare names an `init` message has. Rows can share a
/// name, and then the one Claude Code marks as its own is the one `/name`
/// runs, so that is the one kept. A name starting `__` is Claude Code's own
/// plumbing (`__remote-workflow`), not something to type.
export function slashCommands(list) {
  if (!Array.isArray(list)) return [];
  const byName = new Map();
  for (const c of list) {
    const name = typeof c === "string" ? c : c?.name;
    if (typeof name !== "string" || !name.trim() || name.startsWith("__")) continue;
    const had = byName.get(name);
    if (had && !(c?.builtin && !had.builtin)) continue;
    byName.set(name, {
      name,
      description: typeof c?.description === "string" ? c.description : "",
      argument_hint: typeof c?.argumentHint === "string" ? c.argumentHint : "",
      aliases: Array.isArray(c?.aliases) ? c.aliases.filter((a) => typeof a === "string" && a) : [],
      builtin: Boolean(c?.builtin),
    });
  }
  return [...byName.values()].map(({ builtin: _, ...c }) => c);
}

/// The names an `init` message lists, each with what is already known about
/// it: `init` names the commands and says nothing else, and arrives after
/// `supportedCommands()` has described them.
export function namedCommands(names, known) {
  const described = new Map(known.map((c) => [c.name, c]));
  return slashCommands(names).map((c) => described.get(c.name) ?? c);
}

/// The end of a "turn" that was only a `!` command's output going in: Claude
/// Code reports one for a message that asked for no reply, with nothing in
/// it. Kept, it would draw as a turn that never happened.
export function silent(msg, quiet) {
  if (msg.type !== "result" || msg.num_turns) return false;
  const uuids = msg.user_message_uuids ?? (msg.user_message_uuid ? [msg.user_message_uuid] : []);
  return uuids.length > 0 && uuids.every((u) => quiet.has(u));
}

/// The most of a `!` command's output that is kept: about what Claude Code
/// keeps of a Bash call's, and as much as is worth putting before the model.
export const SHELL_OUTPUT = 30_000;
/// How long a `!` command may run, which is Claude Code's longest for a Bash
/// call. Something meant to run for good belongs in a shell tab.
const SHELL_TIME_MS = 10 * 60_000;

/// Output as text rather than as a terminal draws it: colour and cursor codes
/// taken out, and a line rewritten in place (a progress bar) as it ended.
export function plain(text) {
  return text
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, "")
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "")
    .replace(/\x1b[@-Z\\-_]/g, "")
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((line) => line.split("\r").findLast((part) => part !== "") ?? "")
    .join("\n");
}

/// A command's output as it arrives, keeping its start and its end once it
/// is longer than both: the end is where a command says how it went, and the
/// middle of a long build log is what nobody reads.
export class Output {
  #head = "";
  #tail = "";
  #left = 0;

  constructor(limit = SHELL_OUTPUT) {
    this.headRoom = Math.floor(limit / 3);
    this.tailRoom = limit - this.headRoom;
  }

  add(chunk) {
    let text = String(chunk);
    if (this.#head.length < this.headRoom) {
      const room = this.headRoom - this.#head.length;
      this.#head += text.slice(0, room);
      text = text.slice(room);
    }
    if (!text) return;
    this.#tail += text;
    if (this.#tail.length > this.tailRoom) {
      this.#left += this.#tail.length - this.tailRoom;
      this.#tail = this.#tail.slice(-this.tailRoom);
    }
  }

  get truncated() {
    return this.#left > 0;
  }

  text() {
    const gap = this.#left ? `\n[${this.#left} characters left out]\n` : "";
    return plain(this.#head + gap + this.#tail);
  }
}

/// What Claude is told about a `!` command, in the tags Claude Code's own
/// terminal uses for one, so it reads as the same thing. The output went to
/// one stream, as a terminal shows it; how it ended goes where errors do.
export function shellContext(command, output, code, stopped) {
  const ended = stopped ? "Stopped before it finished." : code === null ? "Did not finish." : code ? `Exit code ${code}` : "";
  return `<bash-input>${command}</bash-input>\n<bash-stdout>${output}</bash-stdout><bash-stderr>${ended}</bash-stderr>`;
}

/// The message of yours that a user message from Claude Code acknowledges, by
/// its uuid, or null for any other: a tool result, a note Claude Code adds, or
/// an acknowledgement already recorded.
export function receipt(msg, sent, read) {
  if (msg.type !== "user" || !msg.isReplay || !msg.uuid) return null;
  if (!sent.has(msg.uuid) || read.has(msg.uuid)) return null;
  return msg.uuid;
}

/// Whether a message belongs in the transcript.
export function kept(msg) {
  if (LIVE.has(msg.type)) return false;
  if (msg.type === "system" && LIVE_SYSTEM.has(msg.subtype)) return false;
  return true;
}

/// The input side of a streaming `query()`: a queue the SDK reads from for as
/// long as the conversation lasts.
export class Inbox {
  #items = [];
  #waiting = null;
  #closed = false;

  push(item) {
    if (this.#closed) return;
    if (this.#waiting) {
      const wake = this.#waiting;
      this.#waiting = null;
      wake({ value: item, done: false });
    } else {
      this.#items.push(item);
    }
  }

  close() {
    this.#closed = true;
    if (this.#waiting) {
      this.#waiting({ value: undefined, done: true });
      this.#waiting = null;
    }
  }

  [Symbol.asyncIterator]() {
    return {
      next: () => {
        if (this.#items.length) return Promise.resolve({ value: this.#items.shift(), done: false });
        if (this.#closed) return Promise.resolve({ value: undefined, done: true });
        return new Promise((wake) => (this.#waiting = wake));
      },
      return: () => {
        this.close();
        return Promise.resolve({ value: undefined, done: true });
      },
    };
  }
}

/// One line saying what a request asks, for a notification that can answer
/// it: Allow on a lock screen is only an answer if the screen says what is
/// being allowed. Claude Code's own sentence when it has one, otherwise the
/// tool and the thing it would act on.
export function describeAsk(request) {
  const input = request.input ?? {};
  const target = [input.command, input.file_path, input.url, input.path, input.pattern].find(
    (v) => typeof v === "string" && v.trim(),
  );
  const line = request.title?.trim() || (target ? `${request.tool}: ${target.trim()}` : request.tool);
  // Short enough for a notification body, and one line of it.
  const flat = line.replace(/\s+/g, " ");
  return flat.length > 300 ? flat.slice(0, 299) + "…" : flat;
}

/// The conversation holding the request `id`, among them all.
///
/// A request id is the tool call's own, so it names one request in one
/// conversation; an answer from outside the window -- a notification on a
/// phone -- knows the id and not which conversation it came from.
export function holding(conversations, id) {
  for (const c of conversations) if (c.pending.has(id)) return c;
  return null;
}

/// What the answer to a permission request becomes, for the SDK.
///
/// Two tools are questions rather than permissions, and answering them is
/// allowing them with something added: AskUserQuestion carries the choices
/// back in its input, and approving a plan may switch the mode it leaves for.
export function permissionResult(tool, input, suggestions, decision) {
  if (!decision.allow) {
    return {
      behavior: "deny",
      message: decision.message?.trim() || "The user declined this.",
    };
  }
  if (tool === "AskUserQuestion") {
    return { behavior: "allow", updatedInput: { ...input, answers: decision.answers ?? {} } };
  }
  const result = { behavior: "allow", updatedInput: input };
  if (tool === "ExitPlanMode" && decision.mode) {
    result.updatedPermissions = [{ type: "setMode", mode: decision.mode, destination: "session" }];
  } else if (decision.always && suggestions?.length) {
    result.updatedPermissions = suggestions;
  }
  return result;
}

/// The session's state across every conversation, as `hura-status` writes it.
///
/// Waiting wins, because a conversation waiting on you is the thing the list
/// exists to say; then working; then idle. A wait also names the request it
/// is waiting on, which is what lets a notification on a phone answer that
/// request and no other: see `hura-agent answer`.
export function sessionStatus(conversations, now = Date.now()) {
  let running = null;
  for (const c of conversations) {
    const ask = c.pending.values().next().value;
    if (ask) {
      return {
        state: "waiting",
        at: Math.floor(now / 1000),
        detail: ask.request.tool,
        ask: ask.request.id,
        prompt: describeAsk(ask.request),
      };
    }
    if (c.activity === "running" && !running) running = c;
  }
  if (running) {
    return { state: "running", at: Math.floor(now / 1000), detail: running.tool ?? "" };
  }
  return { state: "idle", at: Math.floor(now / 1000), detail: "" };
}

/// What has been spent, in the shape the status line hands `hura-usage`, so
/// `usage::parse` reads a chat session without knowing it is one.
///
/// Cost and lines add up across conversations. The model and the context are
/// the most recently active one's, since a strip can only show one of each.
/// The rate-limit windows are the account's, so whichever reported last.
export function usagePayload(conversations, limits) {
  const recent = [...conversations].sort((a, b) => b.activeAt - a.activeAt)[0];
  const sum = (key) => conversations.reduce((n, c) => n + (c.spend[key] ?? 0), 0);
  const payload = {
    cost: {
      total_cost_usd: sum("cost"),
      total_duration_ms: sum("duration"),
      total_lines_added: sum("added"),
      total_lines_removed: sum("removed"),
    },
  };
  if (recent?.model) payload.model = { id: recent.model, display_name: recent.modelName ?? recent.model };
  if (recent?.version) payload.version = recent.version;
  if (recent?.context) {
    payload.context_window = {
      used_percentage: recent.context.percentage,
      context_window_size: recent.context.max,
    };
  }
  const windows = {};
  for (const [key, w] of Object.entries(limits)) {
    windows[key] = { used_percentage: w.used, ...(w.resetsAt ? { resets_at: w.resetsAt } : {}) };
  }
  if (Object.keys(windows).length) payload.rate_limits = windows;
  return payload;
}

/// The windows `usage::parse` knows, by the status line's names for them.
const WINDOWS = new Set(["five_hour", "seven_day"]);

/// Rate-limit windows from one SDK message, keyed the way the status line
/// keys them. Two sources: the plan's own usage rows, which some assistant
/// messages carry, and the events the API's headers produce.
export function rateLimits(msg) {
  const found = {};
  const rows = msg.type === "assistant" ? msg.usage_report?.rate_limits?.limits : null;
  if (Array.isArray(rows)) {
    for (const row of rows) {
      const key = { session: "five_hour", weekly_all: "seven_day" }[row.kind];
      if (!key || typeof row.percent !== "number") continue;
      const at = row.resets_at ? Date.parse(row.resets_at) : NaN;
      found[key] = { used: row.percent, resetsAt: Number.isNaN(at) ? null : Math.floor(at / 1000) };
    }
  }
  if (msg.type === "rate_limit_event") {
    const info = msg.rate_limit_info ?? {};
    // Every window at once, which is what Claude Code 2.1.289 sends: the
    // event's own `utilization` is absent and the windows are under
    // `unifiedWindows`, which the SDK's types do not mention. Read first, so
    // the documented field below wins when a later version sends both.
    for (const [key, w] of Object.entries(info.unifiedWindows ?? {})) {
      if (WINDOWS.has(key) && typeof w?.utilization === "number") {
        found[key] = { used: w.utilization * 100, resetsAt: w.resetsAt ?? null };
      }
    }
    if (WINDOWS.has(info.rateLimitType) && typeof info.utilization === "number") {
      // A fraction on the wire; the status line speaks in percent.
      found[info.rateLimitType] = { used: info.utilization * 100, resetsAt: info.resetsAt ?? null };
    }
  }
  return found;
}

/// The next free name, numbered like a shell: `chat-1`, `chat-2`.
export function nextName(taken) {
  for (let n = 1; ; n++) {
    const name = `chat-${n}`;
    if (!taken.includes(name)) return name;
  }
}

function log(...parts) {
  const at = new Date().toISOString().slice(11, 19);
  console.log(at, ...parts);
}

/// Write and rename, so a reader polling the file never sees half of it.
function writeAtomically(file, text) {
  const tmp = `${file}.${process.pid}`;
  fs.writeFileSync(tmp, text);
  fs.renameSync(tmp, file);
}

/// Exported for the tests, which run `!` commands through one without an SDK.
export class Host {
  constructor({ dir, cwd }) {
    this.dir = path.join(dir, "chat");
    this.hura = dir;
    this.cwd = cwd;
    /// Files sent with messages, a directory per conversation and in it one
    /// per message. Made now, because Claude Code is told it may read here.
    this.uploads = path.join(dir, "uploads");
    /** @type {Map<string, Conversation>} */
    this.conversations = new Map();
    this.limits = {};
    fs.mkdirSync(this.dir, { recursive: true });
    fs.mkdirSync(this.uploads, { recursive: true });
    /// The slash commands Claude Code last described, in any conversation.
    /// They are the working copy's and the image's, so the same in every
    /// conversation; kept on disk, so one not yet started, or a host just
    /// restarted, can offer them before Claude Code has said a word.
    /// `terminal` names those that only work in Claude Code's terminal,
    /// which `init` lists among the rest; a chat does not offer them.
    ({ commands: this.commands, terminal: this.terminal } = this.#readCommands());
    /// Whether this host has never run here before: what decides that the
    /// agent's conversation is to be started, rather than its absence, which
    /// is also what closing it leaves.
    this.fresh = !fs.existsSync(this.#registry());
    for (const record of this.#readRegistry()) {
      this.conversations.set(record.name, new Conversation(this, record));
    }
  }

  #registry() {
    return path.join(this.dir, "conversations.json");
  }

  #readCommands() {
    try {
      const kept = JSON.parse(fs.readFileSync(path.join(this.dir, "commands.json"), "utf8"));
      const terminal = new Set((Array.isArray(kept.terminal) ? kept.terminal : []).filter((n) => typeof n === "string"));
      return { commands: slashCommands(kept.commands), terminal };
    } catch {
      return { commands: [], terminal: new Set() };
    }
  }

  /// The commands one conversation has just heard of, for the rest: each one
  /// Claude Code is not running for has nothing better to offer. Answers
  /// them as a chat offers them, without the terminal's own.
  learnCommands(heard, terminal = this.terminal) {
    this.terminal = terminal;
    const commands = heard.filter((c) => !terminal.has(c.name));
    if (JSON.stringify(commands) === JSON.stringify(this.commands)) return this.commands;
    this.commands = commands;
    try {
      const kept = { commands, terminal: [...terminal] };
      writeAtomically(path.join(this.dir, "commands.json"), JSON.stringify(kept) + "\n");
    } catch (e) {
      log(`could not keep the commands: ${e.message}`);
    }
    for (const c of this.conversations.values()) {
      if (c.query || c.commands === commands) continue;
      c.commands = commands;
      c.broadcast({ t: "state", state: c.snapshot() });
    }
    return commands;
  }

  #readRegistry() {
    try {
      const list = JSON.parse(fs.readFileSync(this.#registry(), "utf8"));
      return Array.isArray(list) ? list.filter((r) => typeof r?.name === "string") : [];
    } catch {
      return [];
    }
  }

  saveRegistry() {
    const records = [...this.conversations.values()].map((c) => c.record);
    writeAtomically(this.#registry(), JSON.stringify(records, null, 2) + "\n");
  }

  names() {
    return [...this.conversations.keys()];
  }

  open(name = nextName(this.names())) {
    let c = this.conversations.get(name);
    if (c) return c;
    c = new Conversation(this, { name, session: randomUUID(), resumable: false, created: Date.now() });
    this.conversations.set(name, c);
    this.saveRegistry();
    log(`opened ${name}`);
    return c;
  }

  close(name) {
    const c = this.conversations.get(name);
    if (!c) throw new Error(`no conversation named ${name}`);
    c.end();
    this.conversations.delete(name);
    this.saveRegistry();
    fs.rmSync(c.file, { force: true });
    // Its files with it: the next conversation to take the name is a new one.
    fs.rmSync(path.join(this.uploads, name), { recursive: true, force: true });
    this.report();
    log(`closed ${name}`);
  }

  /// Who something said to no conversation in particular is for: the agent's
  /// while it is there, then whichever was most recently in use, and a new one
  /// when every conversation has been closed.
  addressee() {
    const agent = this.conversations.get(AGENT);
    if (agent) return agent;
    const recent = [...this.conversations.values()].sort((a, b) => b.activeAt - a.activeAt)[0];
    return recent ?? this.open();
  }

  /// Something changed that the poll reads. Both files are small and written
  /// whole; the poll is every couple of seconds, so there is no point being
  /// cleverer than that.
  report() {
    const list = [...this.conversations.values()];
    try {
      writeAtomically(path.join(this.hura, "status.json"), JSON.stringify(sessionStatus(list)) + "\n");
      writeAtomically(
        path.join(this.hura, "usage.json"),
        JSON.stringify(usagePayload(list, this.limits)) + "\n",
      );
    } catch (e) {
      log(`could not write the status: ${e.message}`);
    }
  }
}

class Conversation {
  constructor(host, record) {
    this.host = host;
    /// `{name, session, resumable, created}`. `session` is the Claude session
    /// id, chosen here so it is known before the first message rather than
    /// learned from it; `resumable` once Claude Code has written a transcript
    /// to resume from.
    this.record = record;
    this.file = path.join(host.dir, `${record.name}.jsonl`);
    this.entries = this.#load();
    this.seq = this.entries.at(-1)?.seq ?? 0;
    /** @type {Set<Client>} */
    this.watchers = new Set();
    this.query = null;
    this.starting = null;
    this.inbox = null;
    /** @type {Map<string, {request: object, resolve: Function, input: object, suggestions?: object[]}>} */
    this.pending = new Map();
    this.sent = new Set();
    /// Those of `sent` that Claude Code has taken into a turn.
    this.read = new Set();
    this.activity = "idle";
    this.tool = null;
    this.status = null;
    // Chosen in the window, and kept with the conversation: Claude Code
    // starts a resumed session in the settings' mode, which would quietly
    // undo a choice made before the host restarted.
    this.mode = record.mode ?? null;
    this.model = record.model ?? null;
    this.effort = record.effort ?? null;
    this.modelName = null;
    this.version = null;
    this.models = [];
    /// `SlashCommand`s. The host's until Claude Code says for itself.
    this.commands = host.commands;
    /// The `!` commands running, by uuid: `{child, stopped}`.
    this.shells = new Map();
    /// Messages pushed to Claude Code that are not prompts of yours: a `!`
    /// command's output. Its echo is not a note to draw.
    this.quiet = new Set();
    /// Those of `quiet` Claude Code has not yet said it took in.
    this.taking = new Set();
    this.context = null;
    this.spend = {};
    this.activeAt = record.created ?? 0;
    this.#endOrphans();
  }

  get name() {
    return this.record.name;
  }

  #load() {
    let text;
    try {
      text = fs.readFileSync(this.file, "utf8");
    } catch {
      return [];
    }
    const entries = [];
    for (const line of text.split("\n")) {
      if (!line) continue;
      try {
        entries.push(JSON.parse(line));
      } catch {
        // A line cut short by a crash mid-write. The rest is still worth having.
      }
    }
    return entries;
  }

  /// A `!` command that was running when the host stopped never said how it
  /// ended, and a window would show it running for ever. It is said now.
  #endOrphans() {
    const ended = new Set(this.entries.filter((e) => e.ev.kind === "exited").map((e) => e.ev.uuid));
    for (const e of this.entries.filter((e) => e.ev.kind === "shell" && !ended.has(e.ev.uuid))) {
      this.append({
        kind: "exited",
        uuid: e.ev.uuid,
        output: "The chat host restarted before this finished.",
        code: null,
        stopped: false,
        truncated: false,
      });
    }
  }

  append(ev) {
    const entry = { seq: ++this.seq, at: Date.now(), ev };
    this.entries.push(entry);
    try {
      fs.appendFileSync(this.file, JSON.stringify(entry) + "\n");
    } catch (e) {
      log(`${this.name}: could not keep an entry: ${e.message}`);
    }
    this.broadcast({ t: "entry", entry });
  }

  broadcast(frame) {
    for (const w of this.watchers) w.send(frame);
  }

  snapshot() {
    return {
      activity: this.pending.size ? "waiting" : this.shells.size ? "running" : this.activity,
      pending: [...this.pending.values()].map((p) => p.request),
      mode: this.mode,
      model: this.model,
      effort: this.effort,
      models: this.models,
      commands: this.commands.map((c) => c.name),
      slash_commands: this.commands,
      status: this.status,
      cost_usd: this.spend.cost ?? null,
      context_percentage: this.context?.percentage ?? null,
      started: this.query !== null,
      files: true,
      receipts: true,
      shell: true,
    };
  }

  changed() {
    this.broadcast({ t: "state", state: this.snapshot() });
    this.host.report();
  }

  attach(client, since) {
    this.watchers.add(client);
    for (const frame of replayFrames(this.entries, since, this.seq)) client.send(frame);
    client.send({ t: "state", state: this.snapshot() });
  }

  async start() {
    const { query } = await import("@anthropic-ai/claude-agent-sdk");
    this.inbox = new Inbox();
    const options = {
      cwd: this.host.cwd,
      pathToClaudeCodeExecutable: CLAUDE,
      // Everything the terminal agent reads: the image's settings, the
      // skills and MCP servers the seeder installed, the repository's
      // CLAUDE.md. Said rather than left to the default, which has moved.
      settingSources: ["user", "project", "local"],
      // Where files sent with a message are saved, readable without a
      // permission prompt: they were handed to the agent to read. The whole
      // directory rather than this conversation's, since a message can point
      // at a file sent in another.
      additionalDirectories: [this.host.uploads],
      systemPrompt: { type: "preset", preset: "claude_code" },
      includePartialMessages: true,
      // A summary of the reasoning, where the default sends empty blocks and
      // a long silence reads as a hang.
      thinking: { type: "adaptive", display: "summarized" },
      canUseTool: (tool, input, opts) => this.#ask(tool, input, opts),
      // HURA_CHAT tells `hura-status` that this process reports the state,
      // so the hooks in settings.json do not write over it. The other asks
      // Claude Code to say when a turn starts, waits and ends, which it
      // otherwise keeps to itself.
      env: {
        ...process.env,
        HURA_CHAT: "1",
        CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS: "1",
        CLAUDE_AGENT_SDK_CLIENT_APP: "hura",
      },
      stderr: (text) => log(`${this.name}: ${text.trimEnd()}`),
      // A subagent's whole conversation, not only its tool calls, so a window
      // can show what it said and thought; and a line on what each one is
      // doing, for the list of them.
      forwardSubagentText: true,
      agentProgressSummaries: true,
      // An acknowledgement of each message as its turn starts, which is the
      // read receipt.
      extraArgs: { "replay-user-messages": null },
    };
    if (this.mode) options.permissionMode = this.mode;
    if (this.model) options.model = this.model;
    if (this.effort) options.effort = this.effort;
    if (this.record.resumable) options.resume = this.record.session;
    else options.sessionId = this.record.session;

    const q = query({ prompt: this.inbox, options });
    this.query = q;
    log(`${this.name}: started${this.record.resumable ? " (resumed)" : ""}`);
    this.changed();
    q.supportedModels()
      .then((models) => {
        this.models = models.map((m) => ({
          value: m.value,
          display_name: m.displayName,
          description: m.description,
          resolved: m.resolvedModel ?? null,
          efforts: m.supportsEffort === false ? [] : (m.supportedEffortLevels ?? []),
        }));
        this.changed();
      })
      .catch(() => {});
    q.supportedCommands()
      .then((commands) => this.#commands(slashCommands(commands)))
      .catch(() => {});
    this.#pump(q);
  }

  #commands(heard, terminal) {
    if (!heard.length) return;
    this.commands = this.host.learnCommands(heard, terminal);
    this.changed();
  }

  async #pump(q) {
    try {
      for await (const msg of q) this.#receive(msg);
    } catch (e) {
      log(`${this.name}: ${e.stack ?? e}`);
      this.append({ kind: "error", message: String(e.message ?? e) });
    } finally {
      if (this.query === q) {
        this.query = null;
        this.inbox = null;
        this.activity = "idle";
        this.tool = null;
        for (const p of this.pending.values()) p.resolve({ behavior: "deny", message: "The conversation ended." });
        this.pending.clear();
        this.changed();
        log(`${this.name}: ended`);
      }
    }
  }

  #receive(msg) {
    this.activeAt = Date.now();
    const limits = rateLimits(msg);
    if (Object.keys(limits).length) {
      Object.assign(this.host.limits, limits);
      this.host.report();
    }
    if (msg.type === "rate_limit_event") log(`rate limit: ${JSON.stringify(msg.rate_limit_info)}`);

    if (silent(msg, this.quiet)) {
      // Not a turn: a `!` command's output going in. Nothing was spent and
      // nothing is drawn, and the activity is what it was. Claude Code has
      // written the session down, though, so it can be resumed.
      for (const u of msg.user_message_uuids ?? [msg.user_message_uuid]) this.taking.delete(u);
      this.#resumable();
      this.changed();
      return;
    }
    if (msg.type === "result") for (const u of msg.user_message_uuids ?? []) this.taking.delete(u);

    switch (msg.type) {
      case "stream_event":
        // The main thread's text as it is written. A subagent's would be
        // interleaved with it, and its result arrives whole anyway.
        if (msg.parent_tool_use_id === null) this.broadcast({ t: "partial", ev: msg });
        break;
      case "tool_progress":
        this.broadcast({ t: "progress", ev: msg });
        break;
      case "system":
        this.#system(msg);
        break;
      case "assistant": {
        if (msg.context_usage) {
          this.context = { percentage: msg.context_usage.percentage, max: msg.context_usage.raw_max_tokens };
        }
        const report = msg.usage_report?.session;
        if (report) {
          this.spend = {
            ...this.spend,
            cost: report.total_cost_usd,
            duration: report.total_duration_ms,
            added: report.total_lines_added,
            removed: report.total_lines_removed,
          };
        }
        // `<synthetic>` is Claude Code speaking for itself, as when it is not
        // logged in, and not a model anyone chose.
        const model = msg.message?.model;
        if (model && model !== "<synthetic>") this.model ??= model;
        if (msg.parent_tool_use_id === null) {
          const use = msg.message?.content?.findLast?.((b) => b.type === "tool_use");
          if (use) this.tool = use.name;
        }
        this.#resumable();
        break;
      }
      case "result":
        this.spend = { ...this.spend, cost: msg.total_cost_usd, duration: msg.duration_ms };
        this.tool = null;
        // The end of a turn, unless another message is queued behind it. The
        // state events say so as well; this is for a Claude Code that does
        // not send them.
        if (!(msg.queued_turn_count > 0)) this.activity = "idle";
        this.#resumable();
        this.#measure();
        break;
      case "user": {
        // Your own message, echoed back as Claude Code starts on it. It is in
        // the transcript already, as you wrote it; what is new is that it has
        // been read.
        const uuid = receipt(msg, this.sent, this.read);
        if (uuid) {
          this.read.add(uuid);
          this.append({ kind: "read", uuid });
        }
        if (msg.isReplay || (msg.uuid && (this.sent.has(msg.uuid) || this.quiet.has(msg.uuid)))) return;
        break;
      }
    }

    if (kept(msg)) this.append({ kind: "sdk", msg });
    if (msg.type === "result" || msg.type === "assistant") this.changed();
  }

  /// How full the context is, asked once a turn ends: assistant messages carry
  /// it only sometimes, and this is the number that says a conversation is
  /// about to compact.
  #measure() {
    this.query
      ?.getContextUsage()
      .then((u) => {
        this.context = { percentage: u.percentage, max: u.rawMaxTokens ?? u.maxTokens };
        this.changed();
      })
      .catch(() => {});
  }

  /// Claude Code has answered, so it has written a transcript, and a restart
  /// can resume this conversation instead of starting it again.
  #resumable() {
    if (this.record.resumable) return;
    this.record.resumable = true;
    this.host.saveRegistry();
  }

  #system(msg) {
    switch (msg.subtype) {
      case "task_progress":
        // A moment, like a tool's progress: kept nowhere, sent to whoever is
        // watching.
        this.broadcast({ t: "task", ev: msg });
        break;
      case "init":
        // What Claude Code says it is running is the truth about the session;
        // a mode or model asked for is passed to it on start, so the two
        // agree unless Claude Code refused one.
        this.model = msg.model;
        this.mode = msg.permissionMode;
        // The effort it runs at, after the model's own limits: only a newer
        // Claude Code says, and a choice stands until it does.
        if (msg.effort !== undefined) this.effort = msg.effort;
        this.version = msg.claude_code_version;
        this.#commands(
          namedCommands(msg.slash_commands, this.commands),
          Array.isArray(msg.terminal_slash_commands) ? new Set(msg.terminal_slash_commands) : undefined,
        );
        this.changed();
        break;
      case "commands_changed":
        // Skills found as the agent works in a directory that has some: the
        // whole list again, to replace the one held.
        this.#commands(slashCommands(msg.commands));
        break;
      case "session_state_changed":
        // Claude Code says it is running while it takes in a `!` command's
        // output, with no reply to come. A conversation idle before is still
        // idle: the session list, and a phone told "Claude finished", would
        // otherwise see a turn that never happened.
        if (msg.state === "running" && this.activity === "idle" && this.taking.size) break;
        this.activity = msg.state === "requires_action" ? "waiting" : msg.state;
        if (msg.state === "idle") this.tool = null;
        this.changed();
        break;
      case "status":
        this.status = msg.status ?? null;
        if (msg.permissionMode) this.mode = msg.permissionMode;
        this.changed();
        break;
    }
  }

  #ask(tool, input, opts) {
    return new Promise((resolve) => {
      const id = opts.toolUseID ?? randomUUID();
      const request = {
        id,
        tool,
        input,
        title: opts.title ?? null,
        description: opts.description ?? null,
        display_name: opts.displayName ?? null,
        reason: opts.decisionReason ?? null,
        blocked_path: opts.blockedPath ?? null,
        can_remember: Boolean(opts.suggestions?.length) && !opts.suppressAlwaysAllowRule,
        default_to_no: Boolean(opts.defaultToNo),
        agent: opts.agentID ?? null,
      };
      this.pending.set(id, { request, resolve, input, suggestions: opts.suggestions });
      // An interrupt cancels the call, and the question with it.
      opts.signal?.addEventListener(
        "abort",
        () => {
          if (this.pending.delete(id)) {
            resolve({ behavior: "deny", message: "Cancelled." });
            this.changed();
          }
        },
        { once: true },
      );
      log(`${this.name}: waiting on ${tool}`);
      this.changed();
    });
  }

  answer(id, decision) {
    const p = this.pending.get(id);
    if (!p) throw new Error("that question has already been answered");
    this.pending.delete(id);
    // Answered, so working again until Claude Code says otherwise; it does
    // not always say so before the tool has run.
    if (!this.pending.size && this.activity === "waiting") this.activity = "running";
    const tool = p.request.tool;
    p.resolve(permissionResult(tool, p.input, p.suggestions, decision));
    this.append({
      kind: "answer",
      id,
      tool,
      allow: Boolean(decision.allow),
      always: Boolean(decision.always),
      message: decision.message ?? null,
      answers: decision.answers ?? null,
    });
    this.changed();
  }

  async send(text, files = []) {
    const uuid = randomUUID();
    let content = text;
    let saved = [];
    if (files.length) {
      const dir = path.join(this.host.uploads, this.name, uuid);
      const message = attach(text, files, dir);
      fs.mkdirSync(dir, { recursive: true });
      for (const f of message.saved) fs.writeFileSync(f.path, f.bytes);
      content = message.content;
      saved = message.saved.map(({ bytes: _, ...f }) => f);
    } else if (!text) {
      throw new Error("nothing to send");
    }
    this.sent.add(uuid);
    this.append({ kind: "prompt", text, uuid, files: saved, receipt: true });
    await this.#running();
    this.inbox.push({
      type: "user",
      message: { role: "user", content },
      parent_tool_use_id: null,
      // Typed by a person, which an SDK host has to say: a message with no
      // origin is treated as unattributed.
      origin: { kind: "human" },
      uuid,
    });
    this.activity = "running";
    this.changed();
  }

  /// Claude Code, started if it is not running. Shared, so a second message
  /// sent while the first is still starting the conversation waits for that
  /// start rather than making another.
  async #running() {
    if (!this.query) await (this.starting ??= this.start().finally(() => (this.starting = null)));
  }

  /// A `!` command: run in the working copy, kept with what it printed, and
  /// then put before Claude without a turn of its own, to be read with the
  /// next message. Answers once it has started; its end is an entry.
  async shell(command) {
    const text = String(command ?? "").trim();
    if (!text) throw new Error("nothing to run");
    const uuid = randomUUID();
    const output = new Output();
    const child = spawn("bash", ["-c", text], {
      cwd: this.host.cwd,
      // A pager or a prompt for a password would wait on a terminal that is
      // not there; with no input, either gives up at once.
      env: { ...process.env, PAGER: "cat", GIT_PAGER: "cat", GIT_TERMINAL_PROMPT: "0" },
      stdio: ["ignore", "pipe", "pipe"],
      // Its own process group, so stopping it stops what it started too.
      detached: true,
    });
    const run = { child, stopped: false, late: false };
    this.shells.set(uuid, run);
    this.append({ kind: "shell", uuid, command: text });
    this.changed();
    log(`${this.name}: running ${JSON.stringify(text)}`);
    child.stdout.setEncoding("utf8").on("data", (d) => output.add(d));
    child.stderr.setEncoding("utf8").on("data", (d) => output.add(d));
    const limit = setTimeout(() => {
      run.late = true;
      this.#stop(run);
    }, SHELL_TIME_MS);
    const ended = new Promise((resolve) => {
      child.on("error", (e) => {
        output.add(`${e.message}\n`);
        resolve(null);
      });
      child.on("close", (code) => resolve(code));
    });
    void ended.then(async (code) => {
      clearTimeout(limit);
      this.shells.delete(uuid);
      if (run.late) output.add(`\n[stopped after ${SHELL_TIME_MS / 60_000} minutes]`);
      const said = output.text();
      const exit = run.stopped || run.late ? null : code;
      this.append({ kind: "exited", uuid, output: said, code: exit, stopped: run.stopped, truncated: output.truncated });
      this.changed();
      try {
        await this.#running();
        const id = randomUUID();
        this.quiet.add(id);
        this.taking.add(id);
        this.inbox.push({
          type: "user",
          message: { role: "user", content: shellContext(text, said, exit, run.stopped) },
          parent_tool_use_id: null,
          origin: { kind: "human" },
          shouldQuery: false,
          uuid: id,
        });
      } catch (e) {
        log(`${this.name}: could not hand a command's output to Claude: ${e.message ?? e}`);
      }
    });
  }

  #stop(run) {
    if (run.child.exitCode !== null || run.child.signalCode !== null) return;
    run.stopped ||= !run.late;
    const group = (signal) => {
      try {
        process.kill(-run.child.pid, signal);
      } catch {
        // Gone already.
      }
    };
    group("SIGTERM");
    setTimeout(() => group("SIGKILL"), 3_000).unref();
  }

  async interrupt() {
    for (const run of this.shells.values()) this.#stop(run);
    await this.query?.interrupt();
  }

  async setMode(mode) {
    this.mode = mode;
    this.record.mode = mode;
    this.host.saveRegistry();
    if (this.query) await this.query.setPermissionMode(mode);
    this.changed();
  }

  async setModel(model) {
    this.model = model ?? null;
    this.record.model = this.model;
    this.host.saveRegistry();
    if (this.query) await this.query.setModel(model ?? undefined);
    this.changed();
  }

  /// `null` goes back to the model's default effort, which is what
  /// `applyFlagSettings` does with a cleared `effortLevel`.
  async setEffort(effort) {
    this.effort = effort ?? null;
    this.record.effort = this.effort;
    this.host.saveRegistry();
    if (this.query) await this.query.applyFlagSettings({ effortLevel: this.effort });
    this.changed();
  }

  end() {
    for (const run of this.shells.values()) this.#stop(run);
    for (const w of this.watchers) w.end();
    this.inbox?.close();
    this.query?.close();
  }
}

/// One connection from a client. Its first line says what it is: a watcher of
/// one conversation, which may then send it commands, or a single request.
class Client {
  constructor(host, socket) {
    this.host = host;
    this.socket = socket;
    this.conversation = null;
    socket.setNoDelay(true);
    socket.on("error", () => {});
    socket.on("close", () => this.conversation?.watchers.delete(this));
    const lines = readline.createInterface({ input: socket, crlfDelay: Infinity });
    // A client that goes away mid-line is the socket's business, handled above.
    lines.on("error", () => {});
    lines.on("line", (line) => {
      this.#line(line).catch((e) => this.send({ t: "error", message: String(e.message ?? e) }));
    });
  }

  send(frame) {
    if (!this.socket.destroyed) this.socket.write(JSON.stringify(frame) + "\n");
  }

  end() {
    this.socket.end();
  }

  async #line(line) {
    if (!line.trim()) return;
    const req = JSON.parse(line);
    const c = this.conversation;
    switch (req.op) {
      case "attach": {
        const target = this.host.conversations.get(req.conv);
        if (!target) throw new Error(`no conversation named ${req.conv}`);
        this.conversation = target;
        target.attach(this, Number(req.since) || 0);
        return;
      }
      case "list":
        return this.#reply({ names: this.host.names() });
      case "new":
        return this.#reply({ name: this.host.open().name });
      case "close":
        this.host.close(req.conv);
        return this.#reply({});
      case "tell": {
        const target = req.conv == null ? this.host.addressee() : this.host.conversations.get(req.conv);
        if (!target) throw new Error(`no conversation named ${req.conv}`);
        await target.send(String(req.text ?? ""));
        return this.#reply({});
      }
      // Answer a request by its id alone, without attaching: what an answer
      // from a notification is, since it knows which request and nothing
      // else. Refused once the request has been answered from anywhere.
      case "decide": {
        const id = String(req.id);
        const target = holding(this.host.conversations.values(), id);
        if (!target) throw new Error("that question has already been answered");
        target.answer(id, req.decision ?? {});
        return this.#reply({});
      }
    }
    if (!c) throw new Error("attach to a conversation first");
    switch (req.op) {
      case "send":
        return c.send(String(req.text ?? ""), Array.isArray(req.files) ? req.files : []);
      case "interrupt":
        return c.interrupt();
      case "answer":
        return c.answer(String(req.id), req.decision ?? {});
      case "mode":
        return c.setMode(String(req.mode));
      case "model":
        return c.setModel(req.model ?? null);
      case "effort":
        return c.setEffort(req.effort ?? null);
      case "shell":
        return c.shell(req.command);
      default:
        throw new Error(`unknown op ${req.op}`);
    }
  }

  #reply(body) {
    this.send({ t: "done", ...body });
    this.socket.end();
  }
}

function serve(args) {
  const host = new Host({ dir: HURA, cwd: args.cwd ?? "/sandbox/repo" });
  const port = Number(args.port ?? LISTEN);

  // The agent's own conversation, started with the task the first time
  // through. A restart finds it in the registry and leaves it be; it resumes
  // when it is next spoken to. One that was closed stays closed.
  if (host.fresh) {
    const agent = host.open(AGENT);
    const task = args["task-file"] ? readTask(args["task-file"]) : "";
    if (task) agent.send(task).catch((e) => log(`agent: ${e.stack ?? e}`));
  }

  const server = net.createServer((socket) => new Client(host, socket));
  server.on("error", (e) => {
    log(`cannot listen on ${port}: ${e.message}`);
    process.exit(1);
  });
  server.listen(port, "127.0.0.1", () => log(`listening on 127.0.0.1:${port}`));

  host.report();
  // Kept fresh while nothing happens, because a status file that has gone
  // quiet is how the poll tells a stopped host from an idle one.
  setInterval(() => host.report(), STATUS_HEARTBEAT_MS).unref();

  const stop = () => {
    for (const c of host.conversations.values()) c.end();
    process.exit(0);
  };
  process.on("SIGTERM", stop);
  process.on("SIGINT", stop);
}

function readTask(file) {
  try {
    return fs.readFileSync(file, "utf8").trim();
  } catch {
    return "";
  }
}

/// A one-shot request to the running host, for the commands below `serve`.
function request(body, port = LISTEN) {
  return new Promise((resolve, reject) => {
    const socket = net.connect(port, "127.0.0.1");
    socket.on("error", (e) => reject(new Error(`hura-agent is not running: ${e.message}`)));
    // Settles nothing once a reply has: a promise is settled once.
    socket.on("close", () => reject(new Error("hura-agent closed the connection without answering")));
    socket.on("connect", () => {
      socket.write(JSON.stringify(body) + "\n");
      const lines = readline.createInterface({ input: socket });
      // The socket's errors are the socket's to report, above; readline
      // re-emits them, and an unheard one would end the process.
      lines.on("error", () => {});
      lines.once("line", (line) => {
        const reply = JSON.parse(line);
        socket.end();
        if (reply.t === "error") reject(new Error(reply.message));
        else resolve(reply);
      });
    });
  });
}

function readStdin() {
  return new Promise((resolve) => {
    let text = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk) => (text += chunk));
    process.stdin.on("end", () => resolve(text));
  });
}

function parseArgs(argv) {
  const args = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) args[a.slice(2)] = argv[++i];
    else args._.push(a);
  }
  return args;
}

async function main(argv) {
  const args = parseArgs(argv);
  const [command, name] = args._;
  switch (command) {
    case "serve":
      return serve(args);
    case "list":
      console.log(JSON.stringify((await request({ op: "list" })).names));
      return;
    case "new":
      console.log((await request({ op: "new" })).name);
      return;
    case "close":
      await request({ op: "close", conv: name });
      return;
    case "send": {
      const text = (await readStdin()).trim();
      if (!text) throw new Error("nothing to send");
      await request({ op: "tell", conv: name, text });
      return;
    }
    // `hura-agent answer ID`, with the decision as JSON on stdin:
    // `{"allow": true}`. On stdin for the reason `send` takes its text there.
    case "answer": {
      if (!name) throw new Error("usage: hura-agent answer ID");
      const decision = JSON.parse((await readStdin()).trim() || "{}");
      await request({ op: "decide", id: name, decision });
      return;
    }
    default:
      throw new Error("usage: hura-agent serve|list|new|close NAME|send [NAME]|answer ID");
  }
}

// Run when executed, not when a test imports the helpers above. Compared as
// real paths, because /usr/local/bin/hura-agent is a link to this file.
const invoked = process.argv[1] && fs.realpathSync(process.argv[1]) === fileURLToPath(import.meta.url);
if (invoked) {
  main(process.argv.slice(2)).catch((e) => {
    console.error(`hura-agent: ${e.message ?? e}`);
    process.exit(1);
  });
}
