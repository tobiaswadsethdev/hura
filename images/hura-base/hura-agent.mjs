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
// Nothing here is a security boundary, and nothing could be: the agent runs as
// the same user and can reach this port, just as a terminal agent can type into
// its own tmux pane. The sandbox policy is the boundary.

import { randomUUID } from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

/// Where clients find this process. Fixed rather than chosen, because the
/// server dials it without asking, and kept in step with `chat::PORT`.
export const PORT = 47681;
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
/// exists to say; then working; then idle.
export function sessionStatus(conversations, now = Date.now()) {
  let running = null;
  for (const c of conversations) {
    const ask = c.pending.values().next().value;
    if (ask) return { state: "waiting", at: Math.floor(now / 1000), detail: ask.request.tool };
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

class Host {
  constructor({ dir, cwd }) {
    this.dir = path.join(dir, "chat");
    this.hura = dir;
    this.cwd = cwd;
    /** @type {Map<string, Conversation>} */
    this.conversations = new Map();
    this.limits = {};
    fs.mkdirSync(this.dir, { recursive: true });
    for (const record of this.#readRegistry()) {
      this.conversations.set(record.name, new Conversation(this, record));
    }
  }

  #registry() {
    return path.join(this.dir, "conversations.json");
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
    if (name === AGENT) throw new Error("the agent's own conversation cannot be closed");
    const c = this.conversations.get(name);
    if (!c) throw new Error(`no conversation named ${name}`);
    c.end();
    this.conversations.delete(name);
    this.saveRegistry();
    fs.rmSync(c.file, { force: true });
    this.report();
    log(`closed ${name}`);
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
    this.activity = "idle";
    this.tool = null;
    this.status = null;
    // Chosen in the window, and kept with the conversation: Claude Code
    // starts a resumed session in the settings' mode, which would quietly
    // undo a choice made before the host restarted.
    this.mode = record.mode ?? null;
    this.model = record.model ?? null;
    this.modelName = null;
    this.version = null;
    this.models = [];
    this.commands = [];
    this.context = null;
    this.spend = {};
    this.activeAt = record.created ?? 0;
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
      activity: this.pending.size ? "waiting" : this.activity,
      pending: [...this.pending.values()].map((p) => p.request),
      mode: this.mode,
      model: this.model,
      models: this.models,
      commands: this.commands,
      status: this.status,
      cost_usd: this.spend.cost ?? null,
      context_percentage: this.context?.percentage ?? null,
      started: this.query !== null,
    };
  }

  changed() {
    this.broadcast({ t: "state", state: this.snapshot() });
    this.host.report();
  }

  attach(client, since) {
    this.watchers.add(client);
    client.send({ t: "replay", seq: this.seq, entries: this.entries.filter((e) => e.seq > since) });
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
    };
    if (this.mode) options.permissionMode = this.mode;
    if (this.model) options.model = this.model;
    if (this.record.resumable) options.resume = this.record.session;
    else options.sessionId = this.record.session;

    const q = query({ prompt: this.inbox, options });
    this.query = q;
    log(`${this.name}: started${this.record.resumable ? " (resumed)" : ""}`);
    this.changed();
    q.supportedModels()
      .then((models) => {
        this.models = models.map((m) => ({ value: m.value, display_name: m.displayName, description: m.description }));
        this.changed();
      })
      .catch(() => {});
    this.#pump(q);
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
      case "user":
        // Your own message, echoed back. It is in the transcript already, as
        // you wrote it.
        if (msg.isReplay || (msg.uuid && this.sent.has(msg.uuid))) return;
        break;
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
      case "init":
        // What Claude Code says it is running is the truth about the session;
        // a mode or model asked for is passed to it on start, so the two
        // agree unless Claude Code refused one.
        this.model = msg.model;
        this.mode = msg.permissionMode;
        this.version = msg.claude_code_version;
        this.commands = msg.slash_commands ?? [];
        this.changed();
        break;
      case "session_state_changed":
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

  async send(text) {
    const uuid = randomUUID();
    this.sent.add(uuid);
    this.append({ kind: "prompt", text, uuid });
    // Shared, so a second message sent while the first is still starting the
    // conversation waits for that start rather than making another.
    if (!this.query) await (this.starting ??= this.start().finally(() => (this.starting = null)));
    this.inbox.push({
      type: "user",
      message: { role: "user", content: text },
      parent_tool_use_id: null,
      // Typed by a person, which an SDK host has to say: a message with no
      // origin is treated as unattributed.
      origin: { kind: "human" },
      uuid,
    });
    this.activity = "running";
    this.changed();
  }

  async interrupt() {
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

  end() {
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
        const target = this.host.conversations.get(req.conv);
        if (!target) throw new Error(`no conversation named ${req.conv}`);
        await target.send(String(req.text ?? ""));
        return this.#reply({});
      }
    }
    if (!c) throw new Error("attach to a conversation first");
    switch (req.op) {
      case "send":
        return c.send(String(req.text ?? ""));
      case "interrupt":
        return c.interrupt();
      case "answer":
        return c.answer(String(req.id), req.decision ?? {});
      case "mode":
        return c.setMode(String(req.mode));
      case "model":
        return c.setModel(req.model ?? null);
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
  const port = Number(args.port ?? PORT);

  // The agent's own conversation, started with the task the first time
  // through. A restart finds it in the registry and leaves it be; it resumes
  // when it is next spoken to.
  if (!host.conversations.has(AGENT)) {
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
function request(body, port = PORT) {
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
      await request({ op: "tell", conv: name ?? AGENT, text });
      return;
    }
    default:
      throw new Error("usage: hura-agent serve|list|new|close NAME|send NAME");
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
