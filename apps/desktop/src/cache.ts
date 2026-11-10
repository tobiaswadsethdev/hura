// What the panes last read, kept for when they are opened again.
//
// Switching worktree, or from the files to git and back, tears the panes down,
// and each used to start again from nothing: the whole transcript replayed,
// the tree read a directory at a time, git asked from scratch, with a spinner
// in place of what was there a second ago. Now a pane draws what it had at
// once and reads again behind it -- so what is shown is never older than one
// read, and the agent's edits still arrive, just without the blank.
//
// In memory and bounded: a window that has visited a hundred worktrees keeps
// the most recent few, and nothing outlives the window.

const all = new Set<Remembered<unknown>>();

/// The key for something of one worktree's, so `forgetSession` finds it.
export function keyFor(server: string, name: string, ...rest: string[]): string {
  return [server, name, ...rest].join("\n");
}

/// Answers by key, the least recently used dropped past `limit`.
export class Remembered<T> {
  private map = new Map<string, T>();

  constructor(private limit: number) {
    all.add(this as Remembered<unknown>);
  }

  get(key: string): T | undefined {
    const value = this.map.get(key);
    if (value !== undefined) {
      // To the back of the line, which is the end that stays.
      this.map.delete(key);
      this.map.set(key, value);
    }
    return value;
  }

  set(key: string, value: T) {
    this.map.delete(key);
    this.map.set(key, value);
    while (this.map.size > this.limit) {
      const oldest = this.map.keys().next().value;
      if (oldest === undefined) break;
      this.map.delete(oldest);
    }
  }

  delete(key: string) {
    this.map.delete(key);
  }

  /// Everything under a key: itself, and anything with more after it.
  forgetUnder(prefix: string) {
    for (const key of [...this.map.keys()]) {
      if (key === prefix || key.startsWith(`${prefix}\n`)) this.map.delete(key);
    }
  }
}

/// A worktree that has gone, and everything remembered about it: one made
/// later with the same name is a different worktree.
export function forgetSession(server: string, name: string) {
  const prefix = keyFor(server, name);
  for (const remembered of all) remembered.forgetUnder(prefix);
}
