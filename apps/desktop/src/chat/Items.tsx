// A transcript's items, drawn.
//
// Text is markdown. Thinking is the model's own summary of its reasoning,
// folded away because it is there to be looked at when something went wrong
// rather than read every time. A tool call is one card: what it was asked to
// do in its header, and what came back underneath, open when the answer is
// the point (an edit's diff, a plan, the todo list) and closed when it is a
// file the agent read for its own benefit.

import { memo, useState } from "react";

import { Chevron, Chosen, Close, Working } from "../icons";
import { Markdown } from "../Markdown";
import type { Item, ToolCall } from "./transcript";

/// Where the working copy is in the sandbox. Paths under it are shown relative
/// to it, which is how the file tree and the git pane show them.
const REPO = "/sandbox/repo/";

export function relative(path: unknown): string {
  const p = typeof path === "string" ? path : "";
  return p.startsWith(REPO) ? p.slice(REPO.length) : p;
}

const str = (v: unknown): string => (typeof v === "string" ? v : v === undefined ? "" : JSON.stringify(v));

/// A tool call in a few words: what kind of thing, and what it was done to.
export function describe(name: string, input: Record<string, unknown>): { verb: string; subject: string } {
  switch (name) {
    case "Bash":
      return { verb: "Run", subject: str(input.description) || str(input.command) };
    case "Read":
      return { verb: "Read", subject: relative(input.file_path) };
    case "Edit":
    case "MultiEdit":
      return { verb: "Edit", subject: relative(input.file_path) };
    case "Write":
      return { verb: "Write", subject: relative(input.file_path) };
    case "NotebookEdit":
      return { verb: "Edit", subject: relative(input.notebook_path) };
    case "Grep":
      return { verb: "Search", subject: `${str(input.pattern)}${input.path ? ` in ${relative(input.path)}` : ""}` };
    case "Glob":
      return { verb: "Find", subject: str(input.pattern) };
    case "WebFetch":
      return { verb: "Fetch", subject: str(input.url) };
    case "WebSearch":
      return { verb: "Search the web", subject: str(input.query) };
    case "TodoWrite":
      return { verb: "Plan", subject: "" };
    case "Task":
    case "Agent":
      return { verb: str(input.subagent_type) || "Agent", subject: str(input.description) };
    case "ExitPlanMode":
      return { verb: "Plan", subject: "ready for review" };
    case "AskUserQuestion":
      return { verb: "Asked", subject: "" };
    case "Skill":
      return { verb: "Skill", subject: str(input.skill) };
  }
  if (name.startsWith("mcp__")) {
    const [, server, tool] = name.split("__");
    return { verb: server ?? name, subject: tool ?? "" };
  }
  return { verb: name, subject: "" };
}

/// Lines added and removed between two texts, with the unchanged ones between
/// them, by longest common subsequence. Quadratic, which is why a large edit
/// falls back to all of one then all of the other: an agent's edit is a
/// handful of lines, and one that is not reads as a rewrite anyway.
type Line = { sign: " " | "-" | "+"; text: string };
export function diffLines(before: string, after: string): Line[] {
  // A text ending in a newline is that many lines, not one more empty one.
  const lines = (text: string) => (text === "" ? [] : text.replace(/\n$/, "").split("\n"));
  const a = lines(before);
  const b = lines(after);
  if (a.length * b.length > 90_000) {
    return [...a.map((text) => ({ sign: "-" as const, text })), ...b.map((text) => ({ sign: "+" as const, text }))];
  }
  const lcs: number[][] = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const out: Line[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ sign: " ", text: a[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      out.push({ sign: "-", text: a[i++] });
    } else {
      out.push({ sign: "+", text: b[j++] });
    }
  }
  while (i < a.length) out.push({ sign: "-", text: a[i++] });
  while (j < b.length) out.push({ sign: "+", text: b[j++] });
  return out;
}

export function Diff({ lines }: { lines: Line[] }) {
  return (
    <pre className="chat-diff">
      {lines.map((l, i) => (
        <div key={i} className={l.sign === "+" ? "add" : l.sign === "-" ? "del" : undefined}>
          <span className="sign">{l.sign}</span>
          {l.text || " "}
        </div>
      ))}
    </pre>
  );
}

/// Output that can be long: the end of it, which is where a command says how
/// it went, with the rest a click away.
function Output({ text, error }: { text: string; error?: boolean }) {
  const [all, setAll] = useState(false);
  const lines = text.replace(/\n+$/, "").split("\n");
  const LIMIT = 30;
  const shown = all || lines.length <= LIMIT ? lines : lines.slice(-LIMIT);
  if (!text.trim()) return <p className="chat-quiet">no output</p>;
  return (
    <>
      {shown.length < lines.length && (
        <button className="chat-more" onClick={() => setAll(true)}>
          {lines.length - shown.length} earlier lines
        </button>
      )}
      <pre className={`chat-output${error ? " bad" : ""}`}>{shown.join("\n")}</pre>
    </>
  );
}

type Todo = { content?: string; status?: string; activeForm?: string };

function Todos({ todos }: { todos: Todo[] }) {
  return (
    <ul className="chat-todos">
      {todos.map((t, i) => (
        <li key={i} className={t.status ?? "pending"}>
          <span className="box" aria-hidden>
            {t.status === "completed" ? <Chosen /> : t.status === "in_progress" ? <Working /> : null}
          </span>
          <span>{t.status === "in_progress" ? (t.activeForm ?? t.content) : t.content}</span>
        </li>
      ))}
    </ul>
  );
}

type Question = { question?: string; header?: string };

/// The choices out of Claude Code's own sentence about them, for a transcript
/// kept before the answers were: `"Which one?"="This one"`, once a question.
function said(call: ToolCall): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of call.result?.text.matchAll(/"([^"]+)"="([^"]*)"/g) ?? []) out[m[1]] = m[2];
  return out;
}

function ToolBody({ call }: { call: ToolCall }) {
  const { input } = call;
  // An interrupted call has nothing worth reading in its result: Claude
  // Code's sentence about the user not wanting to proceed, which the card's
  // own "stopped" already says.
  const result = call.result?.stopped ? null : call.result;
  const failed = result?.isError ? <Output text={result.text} error /> : null;
  switch (call.name) {
    case "Bash":
      return (
        <>
          <pre className="chat-command">{str(input.command)}</pre>
          {result && <Output text={result.text} error={result.isError} />}
        </>
      );
    case "Edit":
      return (
        <>
          <Diff lines={diffLines(str(input.old_string), str(input.new_string))} />
          {failed}
        </>
      );
    case "Write":
      return (
        <>
          <Diff lines={diffLines("", str(input.content))} />
          {failed}
        </>
      );
    case "TodoWrite":
      return <Todos todos={Array.isArray(input.todos) ? (input.todos as Todo[]) : []} />;
    case "ExitPlanMode":
      return <Markdown text={str(input.plan)} />;
    case "AskUserQuestion": {
      const questions = Array.isArray(input.questions) ? (input.questions as Question[]) : [];
      const answers = call.answer?.answers ?? (input.answers as Record<string, string> | undefined) ?? said(call);
      return (
        <dl className="chat-answers">
          {questions.map((q, i) => (
            <div key={i}>
              <dt>{q.question}</dt>
              <dd>{answers[q.question ?? ""] ?? (call.answer && !call.answer.allow ? "not answered" : "…")}</dd>
            </div>
          ))}
        </dl>
      );
    }
    case "Task":
    case "Agent":
      return (
        <>
          {call.children.length > 0 && (
            <div className="chat-nested">
              <Items items={call.children} />
            </div>
          )}
          {result && (result.isError ? <Output text={result.text} error /> : <Markdown text={result.text} />)}
        </>
      );
  }
  return (
    <>
      {Object.keys(input).length > 0 && <pre className="chat-input">{JSON.stringify(input, null, 2)}</pre>}
      {result && <Output text={result.text} error={result.isError} />}
    </>
  );
}

/// Whether a call's card starts open: when what it did is the thing to read.
///
/// An edit, a question and a plan only once they are done. Before that they
/// are waiting on you, and the card asking you shows the same thing.
function opensItself(call: ToolCall): boolean {
  if (call.result?.isError && !call.result.stopped) return true;
  switch (call.name) {
    case "TodoWrite":
      return true;
    case "ExitPlanMode":
    case "AskUserQuestion":
      return call.result !== null;
    case "Edit":
      return (
        call.result !== null &&
        str(call.input.old_string).split("\n").length + str(call.input.new_string).split("\n").length <= 40
      );
    case "Task":
    case "Agent":
      return call.result === null;
  }
  return false;
}

export const ToolCard = memo(function ToolCard({ call, progress }: { call: ToolCall; progress?: number }) {
  const [open, setOpen] = useState<boolean | null>(null);
  const shown = open ?? opensItself(call);
  const { verb, subject } = describe(call.name, call.input);
  const state =
    call.answer && !call.answer.allow
      ? "declined"
      : call.result === null
        ? "running"
        : call.result.stopped
          ? "stopped"
          : call.result.isError
            ? "failed"
            : "done";
  return (
    <div className={`chat-tool ${state}`}>
      <button className="chat-tool-head" aria-expanded={shown} onClick={() => setOpen(!shown)}>
        <Chevron open={shown} />
        {/* One group, so the two faces share a baseline: centring them as
            separate boxes sets the sans verb and the mono subject at
            different heights, because their metrics differ. */}
        <span className="what">
          <span className="verb">{verb}</span>
          <span className="subject" title={subject}>
            {subject}
          </span>
        </span>
        <span className="mark" aria-label={state}>
          {state === "running" && (
            <>
              {progress !== undefined && progress >= 5 && <span className="elapsed">{Math.round(progress)}s</span>}
              <Working />
            </>
          )}
          {state === "failed" && <Close />}
          {(state === "declined" || state === "stopped") && <span className="declined">{state}</span>}
        </span>
      </button>
      {shown && (
        <div className="chat-tool-body">
          <ToolBody call={call} />
          {call.answer && !call.answer.allow && call.answer.message && (
            <p className="chat-quiet">you said: {call.answer.message}</p>
          )}
        </div>
      )}
    </div>
  );
});

function Thinking({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="chat-thinking">
      <button aria-expanded={open} onClick={() => setOpen(!open)}>
        <Chevron open={open} />
        thinking
      </button>
      {open && <div className="body">{text}</div>}
    </div>
  );
}

export const ItemView = memo(function ItemView({ item, progress }: { item: Item; progress?: Map<string, number> }) {
  switch (item.kind) {
    case "prompt":
      return <div className="chat-prompt">{item.text}</div>;
    case "text":
      return <Markdown text={item.text} />;
    case "thinking":
      return <Thinking text={item.text} />;
    case "tool":
      return <ToolCard call={item.call} progress={progress?.get(item.call.id)} />;
    case "note":
      return <p className={`chat-note ${item.tone}`}>{item.text}</p>;
    case "turn":
      return <p className={`chat-turn${item.ok ? "" : " bad"}`}>{item.text}</p>;
    case "divider":
      return <p className="chat-divider">{item.text}</p>;
  }
});

export function Items({ items, progress }: { items: Item[]; progress?: Map<string, number> }) {
  return (
    <>
      {items.map((item) => (
        <ItemView key={item.key} item={item} progress={progress} />
      ))}
    </>
  );
}
