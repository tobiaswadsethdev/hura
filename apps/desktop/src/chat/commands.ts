// What is being typed in the composer, as Claude Code's terminal reads it: a
// slash command to offer and colour, a `!` command for the sandbox's shell,
// or a message.
//
// Pure, so the composer and the transcript agree on what a command looks like.

import type { ChatState } from "../gen/ChatState";
import type { SlashCommand } from "../gen/SlashCommand";

/// The commands a host offers, described when it says, and otherwise by name
/// alone: a host from before it described them sends only the names.
export function offeredCommands(state: ChatState | null): SlashCommand[] {
  if (!state) return [];
  if (state.slash_commands?.length) return state.slash_commands;
  return (state.commands ?? []).map((name) => ({ name, description: "", argument_hint: "", aliases: [] }));
}

/// A command's name as it can be typed: a plugin's are `plugin:name`.
const NAME = /^\/([A-Za-z0-9][\w:.-]*)(?=\s|$)/;

export type Typed =
  | { kind: "message" }
  /// `/name` and what follows it. `known` is the command it names, by its
  /// name or one of its aliases, when the host offers one.
  | { kind: "command"; name: string; known: SlashCommand | null; rest: string }
  /// `!` and the command after it, for a host that runs one.
  | { kind: "shell"; command: string };

export function typed(text: string, commands: SlashCommand[], shell: boolean): Typed {
  if (shell && text.startsWith("!")) return { kind: "shell", command: text.slice(1) };
  const m = NAME.exec(text);
  if (!m) return { kind: "message" };
  const name = m[1];
  const known = commands.find((c) => c.name === name) ?? commands.find((c) => c.aliases.includes(name)) ?? null;
  return { kind: "command", name, known, rest: text.slice(m[0].length) };
}

/// The name being typed, while the list of commands is worth showing: from
/// the slash until the first space, after which it is the command's input.
export function naming(text: string): string | null {
  const m = /^\/(\S*)$/.exec(text);
  return m ? m[1] : null;
}

/// The commands that fit what has been typed, best first: those whose name
/// starts with it, then an alias that does, then a name with it at the start
/// of a word (`review` in `security-review`), then anywhere in the name, then
/// in what the command does. Alphabetical within each, so the list does not
/// shuffle as it narrows.
export function matching(query: string, commands: SlashCommand[]): SlashCommand[] {
  const q = query.toLowerCase();
  const rank = (c: SlashCommand): number => {
    const name = c.name.toLowerCase();
    if (name.startsWith(q)) return 0;
    if (c.aliases.some((a) => a.toLowerCase().startsWith(q))) return 1;
    if (name.split(/[-_:.]/).some((part) => part.startsWith(q))) return 2;
    if (name.includes(q)) return 3;
    if (q.length > 2 && c.description.toLowerCase().includes(q)) return 4;
    return -1;
  };
  return commands
    .map((c) => ({ c, r: rank(c) }))
    .filter((x) => x.r >= 0)
    .sort((a, b) => a.r - b.r || a.c.name.localeCompare(b.c.name))
    .map((x) => x.c);
}

/// A sent message's leading command, for colouring it in the transcript, which
/// does not know which commands the host offered: anything shaped like one.
/// A path such as `/etc/hosts` is not.
export function leadingCommand(text: string): { command: string; rest: string } | null {
  const m = NAME.exec(text);
  return m ? { command: m[0], rest: text.slice(m[0].length) } : null;
}

/// An `@` mention: an `@` at the start of the text or after whitespace, and
/// what follows it up to the next space. What Claude Code reads as a path to
/// look at, and what the composer offers to complete from the working copy.
const MENTION = /(^|\s)(@[^\s@]+)/g;

/// What is typed, cut into the stretches that are `@` mentions and the ones
/// that are not, for the composer to colour the first.
export function mentions(text: string): { text: string; mention: boolean }[] {
  const out: { text: string; mention: boolean }[] = [];
  let last = 0;
  for (const m of text.matchAll(MENTION)) {
    const start = (m.index ?? 0) + m[1].length;
    if (start > last) out.push({ text: text.slice(last, start), mention: false });
    out.push({ text: m[2], mention: true });
    last = start + m[2].length;
  }
  if (last < text.length) out.push({ text: text.slice(last), mention: false });
  return out;
}

export function hasMention(text: string): boolean {
  return new RegExp(MENTION.source).test(text);
}

/// The mention being typed at the caret, if one is: where its `@` is, and what
/// follows it so far, which is what the working copy is searched for.
export function mentionAt(text: string, caret: number): { start: number; query: string } | null {
  const m = /(^|\s)@([^\s@]*)$/.exec(text.slice(0, caret));
  return m ? { start: caret - m[2].length - 1, query: m[2] } : null;
}
