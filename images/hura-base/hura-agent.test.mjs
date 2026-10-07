// The parts of hura-agent that decide something, tested without an SDK or a
// sandbox: `node --test images/hura-base/hura-agent.test.mjs`.

import assert from "node:assert/strict";
import test from "node:test";

import {
  Inbox,
  kept,
  nextName,
  permissionResult,
  rateLimits,
  sessionStatus,
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
    pending: new Map([["t1", { request: { tool: "Edit" } }]]),
  });
  assert.deepEqual(sessionStatus([idle], now), { state: "idle", at: 1_700_000_000, detail: "" });
  assert.deepEqual(sessionStatus([idle, working], now), { state: "running", at: 1_700_000_000, detail: "Bash" });
  assert.deepEqual(sessionStatus([working, asking], now), { state: "waiting", at: 1_700_000_000, detail: "Edit" });
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
