// The agent's questions, answered from the composer, as T3 Code does it: the
// question in a drawer standing on the box, its choices numbered so a key
// picks one, and the box itself the place to type an answer of your own.
// Several questions in one ask are taken one at a time, with where you are
// ("2/3") in the drawer's head.
//
// The flow lives in `useQuestions`, which the composer holds, because its
// keys and its send button are what move it on; the drawer only draws it.

import { useCallback, useEffect, useMemo, useState } from "react";

import type { ChatAsk } from "../gen/ChatAsk";
import { Chosen, Close } from "../icons";
import { cn } from "../lib/cn";
import { decided, type Answer } from "./Ask";

type Option = { label?: string; description?: string };
type Question = { question?: string; header?: string; options?: Option[]; multiSelect?: boolean };

/// The questions an `AskUserQuestion` call asks.
function questionsOf(ask: ChatAsk): Question[] {
  return (Array.isArray(ask.input.questions) ? ask.input.questions : []) as Question[];
}

/// Answering one ask's questions: which is showing, what is picked in each,
/// and what was typed for each that has been left.
export function useQuestions(ask: ChatAsk | null, onAnswer: Answer) {
  const questions = useMemo(() => (ask ? questionsOf(ask) : []), [ask]);
  const [step, setStep] = useState(0);
  const [chosen, setChosen] = useState<string[][]>([]);
  const [typed, setTyped] = useState<string[]>([]);
  // A new ask starts at its first question with nothing picked.
  useEffect(() => {
    setStep(0);
    setChosen([]);
    setTyped([]);
  }, [ask?.id]);

  const question = questions[step] ?? null;
  const last = step >= questions.length - 1;
  const picked = chosen[step] ?? [];

  /// What the current question would be answered with: the box's text if any
  /// was typed, or the choices picked.
  const answerWith = (text: string) => text.trim() || picked.join(", ");

  const submit = useCallback(
    (answers: string[]) => {
      if (!ask) return;
      onAnswer(decided({ allow: true, answers: Object.fromEntries(questions.map((q, i) => [q.question ?? "", answers[i] ?? ""])) }));
    },
    [ask, onAnswer, questions],
  );

  return {
    ask,
    questions,
    question,
    step,
    last,
    picked,
    /// Pick or unpick a choice. A single choice moves on to the next question a
    /// moment later, as T3 Code's do, unless it is the last; `onAdvanced` then
    /// empties the box for it.
    toggle: (label: string, onAdvanced?: () => void) => {
      if (!question) return;
      const multi = !!question.multiSelect;
      setChosen((all) => {
        const next = [...all];
        const now = next[step] ?? [];
        next[step] = multi ? (now.includes(label) ? now.filter((l) => l !== label) : [...now, label]) : [label];
        return next;
      });
      if (!multi && !last) {
        setTimeout(() => {
          setStep((at) => Math.min(at + 1, questions.length - 1));
          onAdvanced?.();
        }, 200);
      }
    },
    /// Whether the current question has an answer, from `text` or a choice.
    ready: (text: string) => answerWith(text) !== "",
    /// Keep `text` as the current answer and go to the next question, or send
    /// them all from the last. Answers with what the box should hold next.
    next: (text: string): string => {
      const answers = [...typed];
      answers[step] = answerWith(text);
      if (last) {
        submit(questions.map((_, i) => answers[i] || (chosen[i] ?? []).join(", ")));
        return "";
      }
      setTyped(answers);
      setStep(step + 1);
      return typed[step + 1] && !(chosen[step + 1] ?? []).length ? typed[step + 1] : "";
    },
    /// Back a question, with what was typed for it.
    back: (text: string): string => {
      const answers = [...typed];
      answers[step] = text.trim();
      setTyped(answers);
      setStep(Math.max(0, step - 1));
      return (chosen[step - 1] ?? []).length ? "" : (typed[step - 1] ?? "");
    },
    skip: () => ask && onAnswer(decided({ allow: false, message: "The user did not answer." })),
  };
}

export type QuestionFlow = ReturnType<typeof useQuestions>;

/// The drawer: the question's header and where it is among them, the
/// question, and its choices, each with the number key that picks it.
export function QuestionDrawer({ flow, onPick }: { flow: QuestionFlow; onPick: (label: string) => void }) {
  const q = flow.question;
  if (!q) return null;
  return (
    <section
      data-tw
      aria-live="polite"
      className="mx-[22px] rounded-t-2xl bg-card/90 px-3 pt-2.5 pb-3 text-xs ring-1 ring-border backdrop-blur-md"
    >
      <header className="flex items-center gap-2">
        {q.header && (
          <span className="rounded-sm bg-warning/16 px-1.5 py-px text-2xs font-medium text-warning-foreground uppercase">
            {q.header}
          </span>
        )}
        <span className="text-muted-foreground">Claude is asking</span>
        {flow.questions.length > 1 && (
          <span className="text-muted-foreground tabular-nums">
            {flow.step + 1}/{flow.questions.length}
          </span>
        )}
        <button
          className="ml-auto inline-flex cursor-pointer items-center gap-1 rounded-md px-1.5 py-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
          title="Skip These Questions"
          onClick={flow.skip}
        >
          <Close className="size-3" />
          Skip
        </button>
      </header>
      <p className="mt-1.5 text-sm leading-relaxed text-foreground">{q.question}</p>
      <ul className="mt-2 flex flex-col gap-0.5" role={q.multiSelect ? "group" : "radiogroup"}>
        {(q.options ?? []).map((o, i) => {
          const label = o.label ?? "";
          const on = flow.picked.includes(label);
          return (
            <li key={i}>
              <button
                role={q.multiSelect ? "checkbox" : "radio"}
                aria-checked={on}
                className={cn(
                  "flex w-full cursor-pointer items-start gap-2.5 rounded-lg px-2 py-1.5 text-left hover:bg-accent",
                  on && "bg-accent",
                )}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => onPick(label)}
              >
                <span
                  className={cn(
                    "mt-0.5 flex size-4 shrink-0 items-center justify-center border border-input",
                    q.multiSelect ? "rounded-sm" : "rounded-full",
                    on && "border-primary bg-primary text-primary-foreground",
                  )}
                >
                  {on && <Chosen className="size-3" />}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-medium text-foreground">{label}</span>
                  {o.description && <span className="block text-2xs text-muted-foreground">{o.description}</span>}
                </span>
                {i < 9 && (
                  <kbd className="mt-0.5 inline-flex h-5 min-w-5 shrink-0 items-center justify-center rounded bg-foreground/7 px-1 font-sans text-xs font-medium text-muted-foreground">
                    {i + 1}
                  </kbd>
                )}
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
