// What a conversation is waiting on you for, as something to answer.
//
// Three kinds, because Claude Code asks three kinds of thing through the one
// permission callback. A tool it wants to run, which is allowed, allowed from
// now on, or declined with a word about what to do instead. A question with
// choices, answered by choosing. And a plan, approved into a mode to carry on
// in, or sent back.
//
// Nothing here is a security boundary; see `hura_core::chat`. It is the same
// prompt the terminal draws, drawn as buttons.

import { useState } from "react";

import type { ChatAsk } from "../gen/ChatAsk";
import type { ChatDecision } from "../gen/ChatDecision";
import { Markdown } from "../Markdown";
import { describe, Diff, diffLines, relative } from "./Items";

type Answer = (decision: ChatDecision) => void;

const decided = (d: Partial<ChatDecision> & { allow: boolean }): ChatDecision => ({
  always: false,
  message: null,
  answers: null,
  mode: null,
  ...d,
});

/// What the agent wants to do, shown well enough to decide on.
function Preview({ ask }: { ask: ChatAsk }) {
  const input = ask.input;
  const text = (v: unknown) => (typeof v === "string" ? v : "");
  switch (ask.tool) {
    case "Bash":
      return <pre className="chat-command">{text(input.command)}</pre>;
    case "Edit":
      return <Diff lines={diffLines(text(input.old_string), text(input.new_string))} />;
    case "Write":
      return <pre className="chat-input">{text(input.content).split("\n").slice(0, 40).join("\n")}</pre>;
  }
  const { subject } = describe(ask.tool, input);
  return subject ? (
    <p className="chat-ask-subject">{subject}</p>
  ) : (
    <pre className="chat-input">{JSON.stringify(input, null, 2)}</pre>
  );
}

function Permission({ ask, onAnswer }: { ask: ChatAsk; onAnswer: Answer }) {
  const [declining, setDeclining] = useState(false);
  const [message, setMessage] = useState("");
  const { verb, subject } = describe(ask.tool, ask.input);
  const title = ask.title ?? `${verb} ${subject || ask.display_name || ask.tool}`.trim();
  return (
    <>
      <header>
        <strong>{title}</strong>
        {/* Often the subject again, which the title already says. */}
        {ask.description && !title.includes(ask.description) && <p>{ask.description}</p>}
        {ask.reason && <p className="chat-quiet">{ask.reason}</p>}
        {ask.blocked_path && <p className="chat-quiet">outside the working copy: {relative(ask.blocked_path)}</p>}
        {ask.agent && <p className="chat-quiet">asked by a subagent</p>}
      </header>
      <Preview ask={ask} />
      {declining ? (
        <form
          className="chat-ask-actions"
          onSubmit={(e) => {
            e.preventDefault();
            onAnswer(decided({ allow: false, message: message.trim() || null }));
          }}
        >
          <input
            autoFocus
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            placeholder="what to do instead (optional)"
          />
          <button type="submit" className="go">
            decline
          </button>
          <button type="button" className="chat-quiet-button" onClick={() => setDeclining(false)}>
            back
          </button>
        </form>
      ) : (
        <div className="chat-ask-actions">
          <button className="go" autoFocus={!ask.default_to_no} onClick={() => onAnswer(decided({ allow: true }))}>
            allow
          </button>
          {ask.can_remember && (
            <button
              className="chat-quiet-button"
              title="allow this and calls like it for the rest of the session, as Claude Code suggests"
              onClick={() => onAnswer(decided({ allow: true, always: true }))}
            >
              allow, and don't ask again
            </button>
          )}
          <button className="chat-quiet-button" autoFocus={ask.default_to_no} onClick={() => setDeclining(true)}>
            decline…
          </button>
        </div>
      )}
    </>
  );
}

type Option = { label?: string; description?: string };
type Question = { question?: string; header?: string; options?: Option[]; multiSelect?: boolean };

function Questions({ ask, onAnswer }: { ask: ChatAsk; onAnswer: Answer }) {
  const questions = (Array.isArray(ask.input.questions) ? ask.input.questions : []) as Question[];
  const [chosen, setChosen] = useState<Record<string, string[]>>({});
  const [other, setOther] = useState<Record<string, string>>({});

  const answerFor = (q: string) => {
    const typed = other[q]?.trim();
    return typed || (chosen[q] ?? []).join(", ");
  };
  const complete = questions.every((q) => answerFor(q.question ?? ""));

  const toggle = (q: Question, label: string) => {
    const key = q.question ?? "";
    setOther((o) => ({ ...o, [key]: "" }));
    setChosen((c) => {
      const now = c[key] ?? [];
      if (!q.multiSelect) return { ...c, [key]: [label] };
      return { ...c, [key]: now.includes(label) ? now.filter((l) => l !== label) : [...now, label] };
    });
  };

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (!complete) return;
        const answers = Object.fromEntries(questions.map((q) => [q.question ?? "", answerFor(q.question ?? "")]));
        onAnswer(decided({ allow: true, answers }));
      }}
    >
      {questions.map((q, i) => {
        const key = q.question ?? "";
        return (
          <fieldset key={i} className="chat-question">
            <legend>
              {q.header && <span className="chip">{q.header}</span>}
              {q.question}
            </legend>
            <div className="chat-options">
              {(q.options ?? []).map((o, j) => (
                <button
                  type="button"
                  key={j}
                  className={`chat-option${(chosen[key] ?? []).includes(o.label ?? "") ? " on" : ""}`}
                  aria-pressed={(chosen[key] ?? []).includes(o.label ?? "")}
                  onClick={() => toggle(q, o.label ?? "")}
                >
                  <strong>{o.label}</strong>
                  {o.description && <span>{o.description}</span>}
                </button>
              ))}
            </div>
            <input
              value={other[key] ?? ""}
              onChange={(e) => {
                setOther((o) => ({ ...o, [key]: e.target.value }));
                if (e.target.value) setChosen((c) => ({ ...c, [key]: [] }));
              }}
              placeholder="or say something else"
            />
          </fieldset>
        );
      })}
      <div className="chat-ask-actions">
        <button type="submit" className="go" disabled={!complete}>
          answer
        </button>
        <button
          type="button"
          className="chat-quiet-button"
          onClick={() => onAnswer(decided({ allow: false, message: "The user did not answer." }))}
        >
          skip
        </button>
      </div>
    </form>
  );
}

function Plan({ ask, onAnswer }: { ask: ChatAsk; onAnswer: Answer }) {
  const [revising, setRevising] = useState(false);
  const [message, setMessage] = useState("");
  return (
    <>
      <header>
        <strong>Claude has a plan</strong>
      </header>
      <div className="chat-plan">
        <Markdown text={typeof ask.input.plan === "string" ? ask.input.plan : ""} />
      </div>
      {revising ? (
        <form
          className="chat-ask-actions"
          onSubmit={(e) => {
            e.preventDefault();
            onAnswer(decided({ allow: false, message: message.trim() || "Keep planning." }));
          }}
        >
          <input autoFocus value={message} onChange={(e) => setMessage(e.target.value)} placeholder="what to change" />
          <button type="submit" className="go">
            send back
          </button>
          <button type="button" className="chat-quiet-button" onClick={() => setRevising(false)}>
            back
          </button>
        </form>
      ) : (
        <div className="chat-ask-actions">
          <button className="go" autoFocus onClick={() => onAnswer(decided({ allow: true, mode: "acceptEdits" }))}>
            approve, and accept edits
          </button>
          <button className="chat-quiet-button" onClick={() => onAnswer(decided({ allow: true, mode: "default" }))}>
            approve, and ask for each edit
          </button>
          <button className="chat-quiet-button" onClick={() => setRevising(true)}>
            keep planning…
          </button>
        </div>
      )}
    </>
  );
}

export function AskCard({ ask, onAnswer }: { ask: ChatAsk; onAnswer: Answer }) {
  return (
    <section className="chat-ask" aria-live="polite">
      {ask.tool === "AskUserQuestion" ? (
        <Questions ask={ask} onAnswer={onAnswer} />
      ) : ask.tool === "ExitPlanMode" ? (
        <Plan ask={ask} onAnswer={onAnswer} />
      ) : (
        <Permission ask={ask} onAnswer={onAnswer} />
      )}
    </section>
  );
}
