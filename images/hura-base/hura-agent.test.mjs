// The parts of hura-agent that decide something, tested without an SDK or a
// sandbox: `node --test images/hura-base/hura-agent.test.mjs`.

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  Host,
  Inbox,
  Output,
  UPLOAD_BYTES,
  attach,
  describeAsk,
  holding,
  kept,
  namedCommands,
  nextName,
  permissionResult,
  plain,
  rateLimits,
  receipt,
  replayFrames,
  sessionStatus,
  shellContext,
  silent,
  slashCommands,
  uploadName,
  usagePayload,
} from "./hura-agent.mjs";

/// A conversation as the aggregate functions see one.
function conversation(over = {}) {
  return {
    pending: new Map(),
    activity: "idle",
    tool: null,
    spend: {},
    activeAt: 0,
    model: null,
    modelName: null,
    version: null,
    context: null,
    ...over,
  };
}

test("the transcript keeps the conversation and leaves out the moments", () => {
  assert.equal(kept({ type: "assistant" }), true);
  assert.equal(kept({ type: "user" }), true);
  assert.equal(kept({ type: "result" }), true);
  assert.equal(kept({ type: "system", subtype: "init" }), true);
  assert.equal(kept({ type: "system", subtype: "compact_boundary" }), true);
  assert.equal(kept({ type: "stream_event" }), false);
  assert.equal(kept({ type: "tool_progress" }), false);
  assert.equal(kept({ type: "rate_limit_event" }), false);
  assert.equal(kept({ type: "system", subtype: "session_state_changed" }), false);
  assert.equal(kept({ type: "system", subtype: "status" }), false);
  // Something a newer SDK sends that this has never heard of is kept: a
  // transcript with an extra line is better than one with a hole.
  assert.equal(kept({ type: "something_new" }), true);
});

test("the inbox hands over what was pushed, in order, and ends when closed", async () => {
  const inbox = new Inbox();
  inbox.push(1);
  const it = inbox[Symbol.asyncIterator]();
  assert.deepEqual(await it.next(), { value: 1, done: false });
  const waiting = it.next();
  inbox.push(2);
  assert.deepEqual(await waiting, { value: 2, done: false });
  inbox.close();
  assert.deepEqual(await it.next(), { value: undefined, done: true });
  inbox.push(3);
  assert.deepEqual(await it.next(), { value: undefined, done: true });
});

test("a denial carries the reason, or says that you declined", () => {
  assert.deepEqual(permissionResult("Bash", {}, [], { allow: false, message: "use the test script" }), {
    behavior: "deny",
    message: "use the test script",
  });
  assert.equal(permissionResult("Bash", {}, [], { allow: false }).message, "The user declined this.");
});

test("an answered question goes back in the tool's own input", () => {
  const input = { questions: [{ question: "Which?" }] };
  const r = permissionResult("AskUserQuestion", input, [], { allow: true, answers: { "Which?": "This one" } });
  assert.deepEqual(r, { behavior: "allow", updatedInput: { ...input, answers: { "Which?": "This one" } } });
});

test("allowing always hands the SDK its own suggestions, and only then", () => {
  const suggestions = [{ type: "addRules", rules: [{ toolName: "Bash" }], behavior: "allow", destination: "session" }];
  const input = { command: "ls" };
  assert.deepEqual(permissionResult("Bash", input, suggestions, { allow: true }), {
    behavior: "allow",
    updatedInput: input,
  });
  assert.deepEqual(permissionResult("Bash", input, suggestions, { allow: true, always: true }), {
    behavior: "allow",
    updatedInput: input,
    updatedPermissions: suggestions,
  });
});

test("approving a plan can choose the mode it leaves for", () => {
  const r = permissionResult("ExitPlanMode", { plan: "p" }, [], { allow: true, mode: "acceptEdits" });
  assert.deepEqual(r.updatedPermissions, [{ type: "setMode", mode: "acceptEdits", destination: "session" }]);
});

test("waiting outranks working, and working outranks idle", () => {
  const now = 1_700_000_000_000;
  const idle = conversation();
  const working = conversation({ activity: "running", tool: "Bash" });
  const asking = conversation({
    pending: new Map([["t1", { request: { id: "t1", tool: "Edit" } }]]),
  });
  assert.deepEqual(sessionStatus([idle], now), { state: "idle", at: 1_700_000_000, detail: "" });
  assert.deepEqual(sessionStatus([idle, working], now), { state: "running", at: 1_700_000_000, detail: "Bash" });
  assert.deepEqual(sessionStatus([working, asking], now), {
    state: "waiting",
    at: 1_700_000_000,
    detail: "Edit",
    ask: "t1",
    prompt: "Edit",
  });
});

test("a request is described by what it would do", () => {
  assert.equal(describeAsk({ tool: "Bash", input: { command: "npm  test\n  -- --watch" } }), "Bash: npm test -- --watch");
  assert.equal(describeAsk({ tool: "Edit", input: { file_path: "/sandbox/repo/README.md" } }), "Edit: /sandbox/repo/README.md");
  assert.equal(describeAsk({ tool: "Bash", title: "Claude wants to run make", input: { command: "make" } }), "Claude wants to run make");
  assert.equal(describeAsk({ tool: "Mystery", input: {} }), "Mystery");
  assert.equal(describeAsk({ tool: "Bash", input: { command: "x".repeat(400) } }).length, 300);
});

test("a request is found by its id in whichever conversation holds it", () => {
  const first = conversation({ pending: new Map([["t1", { request: { id: "t1", tool: "Edit" } }]]) });
  const second = conversation({ pending: new Map([["t2", { request: { id: "t2", tool: "Bash" } }]]) });
  assert.equal(holding([first, second], "t2"), second);
  assert.equal(holding([first, second], "t1"), first);
  assert.equal(holding([first, second], "gone"), null);
});

test("the usage file reads like a status line payload", () => {
  const older = conversation({ activeAt: 1, model: "claude-sonnet-5-5", spend: { cost: 0.5, added: 3 } });
  const newer = conversation({
    activeAt: 2,
    model: "claude-opus-5-5",
    version: "2.1.289",
    context: { percentage: 12, max: 1_000_000 },
    spend: { cost: 0.25, removed: 1 },
  });
  const payload = usagePayload([older, newer], { five_hour: { used: 40, resetsAt: 1_788_434_400 } });
  assert.equal(payload.cost.total_cost_usd, 0.75);
  assert.equal(payload.cost.total_lines_added, 3);
  assert.equal(payload.cost.total_lines_removed, 1);
  assert.equal(payload.model.id, "claude-opus-5-5");
  assert.equal(payload.version, "2.1.289");
  assert.deepEqual(payload.context_window, { used_percentage: 12, context_window_size: 1_000_000 });
  assert.deepEqual(payload.rate_limits, { five_hour: { used_percentage: 40, resets_at: 1_788_434_400 } });
});

test("rate limits come from the plan's rows or from the API's event", () => {
  const fromRows = rateLimits({
    type: "assistant",
    usage_report: {
      rate_limits: {
        limits: [
          { kind: "session", percent: 22, resets_at: "2026-10-07T18:00:00Z" },
          { kind: "weekly_all", percent: 5, resets_at: null },
          { kind: "weekly_scoped", percent: 9, resets_at: null },
        ],
      },
    },
  });
  assert.deepEqual(fromRows, {
    five_hour: { used: 22, resetsAt: Date.parse("2026-10-07T18:00:00Z") / 1000 },
    seven_day: { used: 5, resetsAt: null },
  });

  const fromEvent = rateLimits({
    type: "rate_limit_event",
    rate_limit_info: { status: "allowed", rateLimitType: "seven_day", utilization: 0.31, resetsAt: 1_788_434_400 },
  });
  assert.equal(Math.round(fromEvent.seven_day.used), 31);
  assert.equal(fromEvent.seven_day.resetsAt, 1_788_434_400);

  // What Claude Code 2.1.289 actually sends: both windows, under a key the
  // SDK's types leave out, and no `utilization` beside the type.
  const observed = rateLimits({
    type: "rate_limit_event",
    rate_limit_info: {
      status: "allowed",
      resetsAt: 1_791_393_600,
      rateLimitType: "five_hour",
      unifiedWindows: {
        five_hour: { utilization: 0.01, resetsAt: 1_791_393_600 },
        seven_day: { utilization: 0.11, resetsAt: 1_791_763_200 },
      },
    },
  });
  assert.deepEqual(observed, {
    five_hour: { used: 1, resetsAt: 1_791_393_600 },
    seven_day: { used: 11, resetsAt: 1_791_763_200 },
  });

  assert.deepEqual(rateLimits({ type: "assistant" }), {});
});

test("a new conversation takes the first free number", () => {
  assert.equal(nextName(["agent"]), "chat-1");
  assert.equal(nextName(["agent", "chat-1", "chat-3"]), "chat-2");
});

test("a long replay goes in frames a client can take, each saying where it ends", () => {
  const entry = (seq) => ({ seq, at: 0, ev: { kind: "prompt", text: "x".repeat(100) } });
  const entries = [1, 2, 3, 4, 5, 6].map(entry);
  const one = JSON.stringify(entry(1)).length;

  const frames = replayFrames(entries, 1, 6, one * 2);
  assert.deepEqual(
    frames.map((f) => [f.seq, f.entries.map((e) => e.seq)]),
    [
      [3, [2, 3]],
      [5, [4, 5]],
      [6, [6]],
    ],
  );

  // An entry bigger than a frame still goes, alone.
  assert.deepEqual(
    replayFrames(entries, 4, 6, 1).map((f) => f.entries.length),
    [1, 1],
  );

  // Nothing new is still a frame, which tells the client the seq.
  assert.deepEqual(replayFrames(entries, 6, 6), [{ t: "replay", seq: 6, entries: [] }]);
});

test("a sent file's name can only name a file in its own directory", () => {
  assert.equal(uploadName("../../etc/passwd"), "passwd");
  assert.equal(uploadName("C:\\Users\\me\\shot.png"), "shot.png");
  assert.equal(uploadName(".bashrc"), "bashrc");
  assert.equal(uploadName("a\nb.txt"), "ab.txt");
  assert.equal(uploadName(".."), "file");
  assert.equal(uploadName(undefined), "file");
});

test("a picture goes to the model and every file is named by where it was saved", () => {
  const png = Buffer.from("not really a png").toString("base64");
  const { saved, content } = attach(
    "what is wrong here?",
    [
      { name: "shot.png", mime: "image/png", data: png },
      { name: "shot.png", mime: "text/csv", data: Buffer.from("a,b").toString("base64") },
      { name: "icon.svg", mime: "image/svg+xml", data: Buffer.from("<svg/>").toString("base64") },
    ],
    "/sandbox/.hura/uploads/agent/u1",
  );
  assert.deepEqual(
    saved.map((f) => f.path),
    [
      "/sandbox/.hura/uploads/agent/u1/shot.png",
      "/sandbox/.hura/uploads/agent/u1/shot-2.png",
      "/sandbox/.hura/uploads/agent/u1/icon.svg",
    ],
  );
  assert.equal(saved[1].bytes.toString(), "a,b");
  assert.equal(saved[0].data, png, "a picture the API takes is kept to draw");
  assert.equal(saved[2].data, null, "one it does not is a path");
  assert.equal(content.length, 2);
  assert.deepEqual(content[0], { type: "image", source: { type: "base64", media_type: "image/png", data: png } });
  assert.match(content[1].text, /^what is wrong here\?\n\nFiles sent with this message/);
  assert.match(content[1].text, /- \/sandbox\/\.hura\/uploads\/agent\/u1\/shot-2\.png \(text\/csv, 3 B\)/);
});

test("files without words are still a message, and too many bytes are refused", () => {
  const { content } = attach("", [{ name: "a.txt", mime: "text/plain", data: "aGk=" }], "/u");
  assert.match(content.at(-1).text, /^Files sent with this message/);
  const big = Buffer.alloc(UPLOAD_BYTES + 1).toString("base64");
  assert.throws(() => attach("", [{ name: "big.bin", mime: "", data: big }], "/u"), /a message takes 10 MiB/);
});

test("a picture too large for an image block is only saved", () => {
  const big = Buffer.alloc(4 << 20).toString("base64");
  const { saved, content } = attach("", [{ name: "huge.png", mime: "image/png", data: big }], "/u");
  assert.equal(saved[0].data, null);
  assert.equal(content.length, 1);
});

test("a message is read when Claude Code acknowledges it, once", () => {
  const sent = new Set(["u1"]);
  const read = new Set();
  const ack = { type: "user", isReplay: true, uuid: "u1", message: { role: "user", content: "hi" } };
  assert.equal(receipt(ack, sent, read), "u1");
  read.add("u1");
  assert.equal(receipt(ack, sent, read), null, "already read");
  assert.equal(receipt({ ...ack, uuid: "other" }, sent, read), null, "not one of ours");
  assert.equal(receipt({ ...ack, isReplay: undefined, uuid: "u2" }, new Set(["u2"]), read), null, "not an acknowledgement");
  assert.equal(receipt({ type: "assistant", isReplay: true, uuid: "u1" }, sent, new Set()), null);
});

test("slash commands are described once each, Claude Code's own winning a shared name", () => {
  const described = slashCommands([
    { name: "review", description: "Review a pull request", argumentHint: "<pr>", aliases: ["rv"] },
    { name: "compact", description: "a plugin's compact" },
    { name: "compact", description: "Compact the conversation", argumentHint: "", builtin: true },
    { name: "" },
    null,
  ]);
  assert.deepEqual(described, [
    { name: "review", description: "Review a pull request", argument_hint: "<pr>", aliases: ["rv"] },
    { name: "compact", description: "Compact the conversation", argument_hint: "", aliases: [] },
  ]);
  assert.deepEqual(slashCommands(["clear", "clear", 3, "__remote-workflow"]), [
    { name: "clear", description: "", argument_hint: "", aliases: [] },
  ]);
  assert.deepEqual(slashCommands(undefined), []);
});

test("init's bare names keep what was already known about each", () => {
  const known = slashCommands([{ name: "review", description: "Review a pull request", argumentHint: "<pr>" }]);
  assert.deepEqual(namedCommands(["review", "deploy"], known), [
    { name: "review", description: "Review a pull request", argument_hint: "<pr>", aliases: [] },
    { name: "deploy", description: "", argument_hint: "", aliases: [] },
  ]);
});

test("output is text: no colour codes, and a rewritten line as it ended", () => {
  assert.equal(plain("\x1b[31mred\x1b[0m and \x1b]0;title\x07plain"), "red and plain");
  assert.equal(plain("10%\r50%\r100%\r\ndone\r\n"), "100%\ndone\n");
});

test("long output keeps its start and its end", () => {
  const out = new Output(30);
  out.add("a".repeat(10));
  out.add("b".repeat(50));
  out.add("c".repeat(20));
  assert.ok(out.truncated);
  assert.equal(out.text(), `${"a".repeat(10)}\n[50 characters left out]\n${"c".repeat(20)}`);
  const short = new Output(30);
  short.add("ok\n");
  assert.equal(short.text(), "ok\n");
  assert.ok(!short.truncated);
});

test("Claude reads a ! command in Claude Code's own tags", () => {
  assert.equal(
    shellContext("ls", "a\nb\n", 0, false),
    "<bash-input>ls</bash-input>\n<bash-stdout>a\nb\n</bash-stdout><bash-stderr></bash-stderr>",
  );
  assert.match(shellContext("false", "", 1, false), /<bash-stderr>Exit code 1<\/bash-stderr>/);
  assert.match(shellContext("sleep 9", "", null, true), /Stopped before it finished/);
});

test("a running ! command is not Claude working, so no phone is told Claude finished", () => {
  const busy = conversation({ shells: new Map([["s", {}]]) });
  assert.equal(sessionStatus([conversation(), busy]).state, "idle");
});

/// A host in a directory of its own, with a watcher on its agent's
/// conversation that collects the frames it is sent.
function hostWithWatcher() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "hura-agent-test-"));
  const host = new Host({ dir, cwd: dir });
  const c = host.open("agent");
  const frames = [];
  c.attach({ send: (f) => frames.push(f), end() {} }, 0);
  return { dir, host, c, frames };
}

/// The entry of `kind` once the conversation has one.
async function entryOf(c, kind) {
  for (let i = 0; i < 200; i++) {
    const e = c.entries.find((x) => x.ev.kind === kind);
    if (e) return e;
    await new Promise((r) => setTimeout(r, 25));
  }
  throw new Error(`no ${kind} entry`);
}

test("a ! command runs in the working copy and is kept with what it printed", async () => {
  const { dir, c, frames } = hostWithWatcher();
  await c.shell("  pwd; echo out; echo err >&2; exit 3  ");
  const started = c.entries.find((e) => e.ev.kind === "shell");
  assert.equal(started.ev.command, "pwd; echo out; echo err >&2; exit 3");
  assert.equal(frames.findLast((f) => f.t === "state").state.activity, "running");
  const exited = await entryOf(c, "exited");
  assert.equal(exited.ev.uuid, started.ev.uuid);
  assert.equal(exited.ev.code, 3);
  assert.equal(exited.ev.stopped, false);
  assert.match(exited.ev.output, new RegExp(`^${fs.realpathSync(dir)}\nout\n`));
  assert.match(exited.ev.output, /err/);
  assert.equal(frames.findLast((f) => f.t === "state").state.activity, "idle");
  assert.equal(c.snapshot().shell, true);
  await assert.rejects(c.shell("   "), /nothing to run/);
});

test("stopping the turn stops a ! command and everything it started", async () => {
  const { c } = hostWithWatcher();
  await c.shell("sleep 30 & sleep 30; echo never");
  await c.interrupt();
  const exited = await entryOf(c, "exited");
  assert.equal(exited.ev.code, null);
  assert.equal(exited.ev.stopped, true);
  assert.doesNotMatch(exited.ev.output, /never/);
});

test("a ! command the host never saw end is ended when it starts again", () => {
  const { dir, c } = hostWithWatcher();
  c.append({ kind: "shell", uuid: "s1", command: "sleep 99" });
  const again = new Host({ dir, cwd: dir }).conversations.get("agent");
  const exited = again.entries.find((e) => e.ev.kind === "exited");
  assert.equal(exited.ev.uuid, "s1");
  assert.equal(exited.ev.code, null);
  assert.equal(new Host({ dir, cwd: dir }).conversations.get("agent").entries.filter((e) => e.ev.kind === "exited").length, 1);
});

test("commands one conversation learns are offered by the others, and after a restart", () => {
  const { dir, host, c, frames } = hostWithWatcher();
  const other = host.open("chat-1");
  const commands = slashCommands([{ name: "review", description: "Review a pull request" }, { name: "doctor" }]);
  host.learnCommands(commands, new Set(["doctor"]));
  const offered = commands.slice(0, 1);
  assert.deepEqual(other.snapshot().slash_commands, offered, "not the terminal's own");
  assert.deepEqual(c.snapshot().commands, ["review"]);
  assert.deepEqual(frames.findLast((f) => f.t === "state").state.slash_commands, offered);
  const again = new Host({ dir, cwd: dir });
  assert.deepEqual(again.conversations.get("chat-1").snapshot().slash_commands, offered);
  // A restarted host still knows which are the terminal's, before init says.
  assert.deepEqual(again.learnCommands(commands), offered);
});

test("the empty turn after a ! command's output goes in is not kept", () => {
  const quiet = new Set(["q1"]);
  const result = { type: "result", subtype: "success", num_turns: 0, user_message_uuids: ["q1"] };
  assert.ok(silent(result, quiet));
  assert.ok(!silent({ ...result, user_message_uuids: ["q1", "p1"] }, quiet), "merged with a prompt of yours");
  assert.ok(!silent({ ...result, num_turns: 1 }, quiet));
  assert.ok(!silent({ type: "result", num_turns: 0 }, quiet));
  assert.ok(silent({ type: "result", num_turns: 0, user_message_uuid: "q1" }, quiet));
});
