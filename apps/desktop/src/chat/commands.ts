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
