// A transcript's items, drawn.
//
// Text is markdown. Thinking is the model's own summary of its reasoning,
// folded away because it is there to be looked at when something went wrong
// rather than read every time. A tool call is one card: what it was asked to
// do in its header, and what came back underneath, open when the answer is
// the point (an edit's diff, a plan, the todo list) and closed when it is a
// file the agent read for its own benefit. A `!` command you ran is a card
// too, open, since its output is why it was run. A turn's steps are one row
// that says what they came to, open while the turn runs and shut once it has
// answered -- see `group` in transcript.ts.

import { createContext, memo, useContext, useLayoutEffect, useRef, useState } from "react";

import { Fold, scrollParent, slide } from "../Fold";
import type { ChatFile } from "../gen/ChatFile";
import { Attach, Chosen, Close, Delivered, Read, Twisty, Working } from "../icons";
import { Markdown } from "../Markdown";
import { leadingCommand } from "./commands";
import { type Item, type ToolCall, type ToolResult, TYPED_SKILL } from "./transcript";

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
  if (!text.trim()) return <p className="chat-quiet">No output</p>;
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

/// Opens the subagent a call started beside the conversation, from inside
/// its card. Unset where there is nowhere to open one.
export const OpenAgent = createContext<((callId: string) => void) | null>(null);

/// Whether a Skill call ran its skill as a subagent: one typed as a command,
/// or one whose subagent's messages have started to arrive.
function forkedCall(call: ToolCall): boolean {
  return call.children.length > 0 || call.id.startsWith(TYPED_SKILL);
}

/// What a subagent did, inside the call that started it, and what it reported.
function Subagent({
  call,
  result,
  follow,
}: {
  call: ToolCall;
  result: ToolResult | null;
  follow: ((callId: string) => void) | null;
}) {
  return (
    <>
      {follow && (
        <button className="chat-follow" onClick={() => follow(call.id)}>
          Follow this agent beside the conversation
        </button>
      )}
      {call.children.length > 0 && (
        <div className="chat-nested">
          <Items items={call.children} />
        </div>
      )}
      {result?.text.trim() && (result.isError ? <Output text={result.text} error /> : <Markdown text={result.text} />)}
    </>
  );
}

function ToolBody({ call }: { call: ToolCall }) {
  const openAgent = useContext(OpenAgent);
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
              <dd>{answers[q.question ?? ""] ?? (call.answer && !call.answer.allow ? "Not answered" : "…")}</dd>
            </div>
          ))}
        </dl>
      );
    }
    // A background subagent's report to the agent that started it.
    case "SubagentHandback":
      return <Markdown text={str(input.message)} />;
    case "Task":
    case "Agent":
      return <Subagent call={call} result={result} follow={openAgent} />;
    // A forked skill runs as a subagent and is drawn as one. An inline skill
    // has nothing inside it: its input and Claude Code's line about it, as
    // any other call.
    case "Skill":
      if (forkedCall(call)) return <Subagent call={call} result={result} follow={openAgent} />;
      break;
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
/// are waiting on you, and the card asking you shows the same thing. A call
/// that failed stays shut: its header says so, and an agent that tries a few
/// things on its way to one that works would otherwise leave a column of
/// error output behind it.
function opensItself(call: ToolCall): boolean {
  if (call.result?.images.length) return true;
  switch (call.name) {
    case "TodoWrite":
    case "SubagentHandback":
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
    case "Skill":
      return call.result === null && forkedCall(call);
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
        <Twisty open={shown} />
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
      <Fold open={shown}>
        <div className="chat-tool-body">
          <ToolBody call={call} />
          {call.result?.images.map((p, i) => (
            <img key={i} className="chat-image" src={`data:${p.mime};base64,${p.data}`} alt="" />
          ))}
          {call.answer && !call.answer.allow && call.answer.message && (
            <p className="chat-quiet">You said: {call.answer.message}</p>
          )}
        </div>
      </Fold>
    </div>
  );
});

/// A `!` command you ran, and what it printed. Open, unlike a tool call: you
/// ran it to see this.
const ShellCard = memo(function ShellCard({ item }: { item: Extract<Item, { kind: "shell" }> }) {
  const [open, setOpen] = useState(true);
  const state =
    item.output === null ? "running" : item.stopped ? "stopped" : item.code === 0 ? "done" : "failed";
  return (
    <div className={`chat-tool chat-shell ${state}`}>
      <button className="chat-tool-head" aria-expanded={open} onClick={() => setOpen(!open)}>
        <Twisty open={open} />
        <span className="what">
          <span className="verb">You ran</span>
          <span className="subject" title={item.command}>
            {item.command}
          </span>
        </span>
        <span className="mark" aria-label={state}>
          {state === "running" && <Working />}
          {state === "stopped" && <span className="declined">stopped</span>}
          {state === "failed" && (
            <span className="exit">{item.code === null ? "did not finish" : `exit ${item.code}`}</span>
          )}
        </span>
      </button>
      <Fold open={open && item.output !== null}>
        <div className="chat-tool-body">
          <Output text={item.output ?? ""} />
        </div>
      </Fold>
    </div>
  );
});

/// What was sent with a message: a picture the model saw as itself, and any
/// other file as the path the agent was given.
function SentFiles({ files }: { files: ChatFile[] }) {
  return (
    <ul className="chat-sent">
      {files.map((f) =>
        f.data ? (
          <li key={f.path} title={f.path}>
            <img src={`data:${f.mime};base64,${f.data}`} alt={f.name} />
          </li>
        ) : (
          <li key={f.path} title={f.path}>
            <span className="chat-file">
              <Attach />
              {f.name}
            </span>
          </li>
        ),
      )}
    </ul>
  );
}

function Thinking({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="chat-thinking">
      <button aria-expanded={open} onClick={() => setOpen(!open)}>
        <Twisty open={open} />
        Thinking
      </button>
      <Fold open={open}>
        <div className="body">{text}</div>
      </Fold>
    </div>
  );
}

/// What a turn's steps came to, in a few words: "Read 3 files, ran 2
/// commands". Kinds of work in the order they were first done.
export function summarize(items: Item[]): string {
  const counts = new Map<string, number>();
  const add = (what: string) => counts.set(what, (counts.get(what) ?? 0) + 1);
  for (const item of items) {
    if (item.kind === "thinking") add("thought");
    else if (item.kind === "text") add("message");
    else if (item.kind === "tool") {
      const name = item.call.name;
      if (name === "Read") add("read");
      else if (["Edit", "MultiEdit", "Write", "NotebookEdit"].includes(name)) add("edit");
      else if (name === "Bash") add("command");
      else if (name === "Grep" || name === "Glob") add("search");
      else if (name === "WebFetch" || name === "WebSearch") add("web");
      else if (name === "Task" || name === "Agent") add("agent");
      else if (name === "TodoWrite") add("todo");
      else add("tool");
    }
  }
  const n = (k: number, one: string, many: string) => (k === 1 ? one : `${k} ${many}`);
  const words: Record<string, (k: number) => string> = {
    read: (k) => `read ${n(k, "a file", "files")}`,
    edit: (k) => `edited ${n(k, "a file", "files")}`,
    command: (k) => `ran ${n(k, "a command", "commands")}`,
    search: (k) => `searched ${k === 1 ? "once" : `${k} times`}`,
    web: (k) => `looked up ${n(k, "a page", "pages")}`,
    agent: (k) => `started ${n(k, "an agent", "agents")}`,
    todo: () => "updated the plan",
    tool: (k) => `used ${n(k, "a tool", "tools")}`,
  };
  const parts = [...counts.keys()].filter((k) => words[k]).map((k) => words[k](counts.get(k) ?? 0));
  // Thinking and messages on their way only say so when nothing else does.
  if (parts.length === 0) parts.push(counts.has("thought") ? "thought" : "worked");
  const said = parts.join(", ");
  return said.charAt(0).toUpperCase() + said.slice(1);
}

/// A turn's steps, as one row. Open while the turn runs, so the work can be
/// watched, and shut when it ends, leaving the answer under it; either way
/// until it is clicked, after which it stays as it was put.
const StepsCard = memo(function StepsCard({
  item,
  progress,
}: {
  item: Extract<Item, { kind: "steps" }>;
  progress?: Map<string, number>;
}) {
  const [open, setOpen] = useState<boolean | null>(null);
  const shown = open ?? item.live;
  const calls = item.items.flatMap((i) => (i.kind === "tool" ? [i.call] : []));
  const running = calls.some((c) => c.result === null && !(c.answer && !c.answer.allow));
  const failed = calls.filter((c) => c.result?.isError && !c.result.stopped).length;
  return (
    <div className={`chat-steps${running ? " running" : ""}`}>
      <button className="chat-steps-head" aria-expanded={shown} onClick={() => setOpen(!shown)}>
        <Twisty open={shown} />
        <span className="what">{summarize(item.items)}</span>
        <span className="mark">
          {failed > 0 && <span className="failed">{failed} failed</span>}
          {running && <Working />}
        </span>
      </button>
      <Fold open={shown}>
        <div className="chat-steps-body">
          <Items items={item.items} progress={progress} />
        </div>
      </Fold>
    </div>
  );
});

/// Lets go of the bottom of the conversation, which it otherwise keeps to
/// while it is there. Unset where nothing is kept to the bottom.
export const Unpin = createContext<(() => void) | null>(null);

/// How many lines of a long message show until the rest is asked for, and how
/// many more it has to have before it counts as long: folding away a single
/// line saves no more room than the button to unfold it takes. The iPhone
/// app's `PromptBubble` cuts at the same numbers.
const SHOWN_LINES = 5;
const LONG_LINES = SHOWN_LINES + 2;

/// Space kept above a message that "show less" has brought back into view.
const ABOVE = 12;

/// Something you said. A long one shows its first lines, fading out, with the
/// rest a click away: a pasted log or a page of instructions would otherwise
/// fill the window and push the conversation around it out of sight.
///
/// Long is measured, not counted, since how many lines some text takes
/// depends on how wide the pane is; and measured again when the pane is
/// resized.
/// What you wrote, with a slash command it starts with set apart, as it was
/// while you typed it.
function PromptWords({ text }: { text: string }) {
  const lead = leadingCommand(text);
  if (!lead) return <>{text}</>;
  return (
    <>
      <span className="chat-slash">{lead.command}</span>
      {lead.rest}
    </>
  );
}

function Prompt({ text }: { text: string }) {
  const unpin = useContext(Unpin);
  const bubble = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  /// The height it is cut to, while it is long enough to be.
  const [cut, setCut] = useState<number | null>(null);
  const [all, setAll] = useState(false);
  /// Where it was when it was last opened or closed, for the move to start
  /// from.
  const before = useRef<{ height: number; scroller: HTMLElement | null; scrollTop: number; top: number } | null>(null);

  useLayoutEffect(() => {
    const el = body.current;
    if (!el) return;
    const measure = () => {
      const style = getComputedStyle(el);
      const line = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.3;
      // In lines, with half of one to spare: `scrollHeight` is a whole number
      // of pixels, so seven lines of 19.5 measure 137, which is over 136.5.
      setCut(el.scrollHeight / line > LONG_LINES + 0.5 ? Math.round(line * SHOWN_LINES) : null);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [text]);

  const toggle = () => {
    const el = body.current;
    const box = bubble.current;
    if (!el || !box) return;
    const scroller = scrollParent(box);
    before.current = {
      height: el.getBoundingClientRect().height,
      scroller,
      scrollTop: scroller?.scrollTop ?? 0,
      top: box.getBoundingClientRect().top - (scroller?.getBoundingClientRect().top ?? 0),
    };
    // Opened to be read from where it was cut, so the conversation stops
    // following its bottom: kept there, the message would grow upwards and
    // show its end instead.
    if (!all) unpin?.();
    setAll(!all);
  };

  useLayoutEffect(() => {
    const el = body.current;
    const was = before.current;
    before.current = null;
    if (!el || !was) return;
    for (const a of el.getAnimations()) a.cancel();
    // Closing, the height it ends at is the cut, and is not measured: laying
    // the message out at that height before the move has started shrinks the
    // conversation under the scroll position, which the browser then pulls
    // back, and the window jumps for a frame.
    const to = all || cut === null ? el.getBoundingClientRect().height : cut;
    const moving = slide(el, "maxHeight", was.height, to);
    // Closing, the button you pressed sits at the bottom of a message that may
    // have started well above the window. Kept under the pointer while the
    // message shrinks towards it, by scrolling up as much as it shrinks; but
    // no further than it takes to bring the top of the message into view.
    const shrink = was.height - to;
    const { scroller } = was;
    if (all || shrink <= 0 || !scroller) return;
    const back = Math.min(shrink, Math.max(0, ABOVE - was.top));
    if (back === 0) return;
    if (!moving) {
      scroller.scrollTop = was.scrollTop - back;
      return;
    }
    // On each change of its size rather than each animation frame: an
    // observer is told after layout and before paint, so the scroll moves in
    // the frame the message shrank in, where a frame callback can see the
    // size of the one before.
    const follow = new ResizeObserver(() => {
      const gone = was.height - el.getBoundingClientRect().height;
      scroller.scrollTop = was.scrollTop - back * Math.min(1, gone / shrink);
    });
    follow.observe(el);
    const stop = () => follow.disconnect();
    moving.addEventListener("finish", () => {
      stop();
      scroller.scrollTop = was.scrollTop - back;
    });
    moving.addEventListener("cancel", stop);
    return stop;
  }, [all]);

  const long = cut !== null;
  return (
    <div className={`chat-prompt${long ? " long" : ""}`} ref={bubble}>
      <div
        className={`chat-prompt-text${long && !all ? " cut" : ""}`}
        ref={body}
        style={long && !all ? { maxHeight: cut } : undefined}
      >
        <PromptWords text={text} />
      </div>
      {long && (
        <button className="chat-prompt-more" aria-expanded={all} onClick={toggle}>
          {all ? "Show less" : "Show more"}
        </button>
      )}
    </div>
  );
}

/// Whether receipts are drawn, and if so whether a turn is running: a message
/// not yet read while one is, is waiting for it to end.
export type Receipts = "off" | "idle" | "working";

/// A time of day, as this window's locale says one.
function clock(at: number): string {
  return new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/// A day, relative to today while that is shorter.
function dayName(at: number): string {
  const d = new Date(at);
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString([], {
    weekday: "short",
    day: "numeric",
    month: "short",
    year: d.getFullYear() === today.getFullYear() ? undefined : "numeric",
  });
}

/// When a message was sent, and what has become of it: in the sandbox, waiting
/// behind the turn in progress, or read. One tick for sent and two for read,
/// as a messaging app has them; waiting behind a turn is the one tick too,
/// said in words only on hover -- "queued" beside a tick that already means
/// "not read yet" was the same fact twice.
function Receipt({ at, read, receipts }: { at: number; read: number | null; receipts: Receipts }) {
  const full = new Date(at).toLocaleString();
  if (receipts === "off") {
    return (
      <p className="chat-receipt" title={`Sent ${full}`}>
        {clock(at)}
      </p>
    );
  }
  if (read !== null) {
    return (
      <p className="chat-receipt" title={`Sent ${full}, read ${new Date(read).toLocaleString()}`}>
        {clock(at)}
        <Read aria-label="Read" />
      </p>
    );
  }
  const queued = receipts === "working";
  return (
    <p className="chat-receipt" title={queued ? `Sent ${full}; Claude takes it when this turn is done` : `Sent ${full}`}>
      {clock(at)}
      <Delivered aria-label={queued ? "Queued" : "Sent"} />
    </p>
  );
}

export const ItemView = memo(function ItemView({
  item,
  progress,
  receipts = "off",
}: {
  item: Item;
  progress?: Map<string, number>;
  receipts?: Receipts;
}) {
  switch (item.kind) {
    case "prompt":
      return (
        <>
          {item.files.length > 0 && <SentFiles files={item.files} />}
          {item.text && <Prompt text={item.text} />}
          <Receipt at={item.at} read={item.read} receipts={receipts} />
        </>
      );
    case "day":
      return <p className="chat-divider">{dayName(item.at)}</p>;
    case "text":
      return <Markdown text={item.text} />;
    case "thinking":
      return <Thinking text={item.text} />;
    case "tool":
      return <ToolCard call={item.call} progress={progress?.get(item.call.id)} />;
    case "shell":
      return <ShellCard item={item} />;
    case "note":
      return <p className={`chat-note ${item.tone}`}>{item.text}</p>;
    case "turn":
      return (
        <p className={`chat-turn${item.ok ? "" : " bad"}`} title={new Date(item.at).toLocaleString()}>
          {[clock(item.at), item.text].filter(Boolean).join(" · ")}
        </p>
      );
    case "divider":
      return <p className="chat-divider">{item.text}</p>;
    case "steps":
      return <StepsCard item={item} progress={progress} />;
  }
});

export function Items({
  items,
  progress,
  receipts,
}: {
  items: Item[];
  progress?: Map<string, number>;
  receipts?: Receipts;
}) {
  return (
    <>
      {items.map((item) => (
        <ItemView key={item.key} item={item} progress={progress} receipts={item.kind === "prompt" ? receipts : undefined} />
      ))}
    </>
  );
}
