# Plan: the phone in T3 Code's shape

The desktop took T3 Code's look and its features in
[t3-style-desktop.md](t3-style-desktop.md). This is the same for the iOS app:
the palette, the list's rows, the conversation and its composer, and the
features that came with it, as far as they make sense on a phone.

The phone stays a phone. It keeps iOS's navigation (a list that pushes a
worktree, sheets, menus, swipe actions and long presses) rather than imitating
a desktop sidebar, and takes what T3 Code's shape *says*: the calm black (or
near white) canvas, cards one step up, the states in T3 Code's hues and words,
a quiet bubble for your messages, a turn's work folded into a work log, and a
composer that does a lot without looking busy.

## Decisions

| | |
|---|---|
| **The tokens are the contract** | `Hura/Views/Theme.swift` holds the desktop's custom properties as colours (`.canvas`, `.surface`, `.surface2`, `.ink`, `.dim`, `.line`, and the states `.working`, `.attention`, `.ok`, `.bad`) in both appearances, and the desktop's radii. Screens ask for these, not for greys. |
| **The states** | T3 Code's: blue for working, amber for waiting on you (purple before), emerald for ready, red for failed, and the sidebar's words: Working, Setting up, Needs you, Failed. |
| **The font** | The system's. T3 Code uses the Apple faces on Apple platforms. |
| **What is the phone's own** | As on the desktop, triage (pin, snooze, settle), the prompt stash and drafts are kept on the device, not the server. Project defaults and the queue's repeats are the server's, so the phone and the desktop agree. |
| **Not taken** | The command palette, rebindable keys, VS Code themes, fonts and zoom: they are a keyboard's and a window's. |

## Phases

- [x] **0. Foundation.** The palette, radii and state hues (`Theme.swift`);
  T3 Code's blue as the accent. HuraKit learns the new requests: `FindFiles`,
  `TailFile`, `NewChat` with `fork` and `at`, `NewShell` with `run`, `Enqueue`
  and `QueueMessage` with `repeat`, the queue's `utc_offset`, and
  `SetProjectDefaults` with each project's `defaults`.
- [ ] **1. The list.** A server's projects as T3 Code's folders (a monogram,
  foldable, a count when folded), each worktree a card: the title, the branch
  under it, the state as a glyph and a word or its age, its pull request and
  the sandbox's mark. Pin, snooze and settle, with the pinned shelf at the top
  and the settled one at the bottom; one waiting on you is never hidden. A
  search field narrows the list. Queued sessions as slim rows, a repeating
  one saying its rule.
- [ ] **2. The conversation.** Your messages in a quiet bubble rather than the
  accent, replies as prose, a turn's steps as its work log ("Worked for 2m
  14s", one line per tool call with a glyph for its kind). Queued messages as
  dashed bubbles on your side, with Edit and Remove. Edit from here, from a
  long press on your message, forks the conversation from just before it.
- [ ] **3. The composer.** T3 Code's card and footer: attach on the left; the
  mode, the model and effort, the context meter, and the round send and stop
  on the right. `@` mentions a file from the working copy (`FindFiles`).
  While a turn runs, send queues; a long press on send stops the turn and
  sends now (T3 Code's steer). A draft kept per conversation; the prompt
  stash; a message scheduled for later can repeat. `/btw` asks on the side in
  a forked conversation. Background commands beside the subagents, with their
  output followed by `TailFile`.
- [ ] **4. New worktrees.** A draft rather than a form: "What should we build
  in ‹project›?" over a composer, with where it runs, the base branch, the
  policy and the name in a strip under it, and the rest under More. It opens
  with the project's defaults, which a project's menu can change. A worktree
  can start later or come round again on a schedule, where the server's queue
  says its clock. Project scripts run in a new shell.
- [ ] **5. Usage,** and the rest of the screens (the dock's panes, trackers,
  settings) through the tokens.
