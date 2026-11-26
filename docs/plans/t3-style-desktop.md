# Plan: the desktop in T3 Code's shape

The desktop window takes the look and the layout of
[T3 Code](https://github.com/pingdotgg/t3code), an MIT-licensed desktop for
coding agents, and keeps the two things T3 Code does not have: a sandbox per
session whose network policy you can see and change, and trackers, with your
tickets and pull requests beside the work they belong to.

This is a plan, worked on in increments. Each phase below is meant to be a pull
request or a few, and to leave the window working when it lands. Tick a phase
off here when it does.

## Why

T3 Code reads as one calm surface: a list of conversations on the left, the
conversation in the middle at a comfortable width, and everything else one
toggle away. Its composer does a lot without looking busy. hura's window has
grown four columns that are all open all the time (the worktree tree, the
chat, the dock and the tickets sidebar), a title bar of its own above them, and
a composer that is a text box with a row of words under it.

The reasons to use hura rather than T3 Code are not in that chrome: they are
the sandbox, the policy and events panes, and the trackers. They should get
*easier* to reach in the new shape, not lost in it.

## Decisions

| | |
|---|---|
| **What is taken** | The layout, the palette, the type, spacing and radii, the composer's anatomy and features, the sidebar row's anatomy, the collapsed work log ("Worked for 2m 14s"), the changed-files card, the right panel and the terminal drawer. Measured from T3 Code's source, not approximated from screenshots. |
| **What is not** | T3 Code's name, wordmark, logos, app icons and anything else its [TRADEMARK.md](https://github.com/pingdotgg/t3code/blob/main/TRADEMARK.md) lists. hura keeps its own name and icon, and nothing in the window says or implies T3. |
| **Rebuilt where the shape changes, restyled where it does not** | The shell (the layout in `App.tsx`), the sidebar, the composer, the conversation's work log, the right panel with the terminal drawer, and the ticket and pull request views are rebuilt as new components in T3 Code's structure, each replacing the old one in a single step. Settings, the create forms, dialogs and the insides of the panes (events, policy, ports, facts, files, git) keep their logic and take the new look through the tokens and a restyle. |
| **The tokens are the contract** | Every colour, radius and shadow comes from the custom properties at the top of `style.css`, which hold T3 Code's values. Old and rebuilt components read the same ones, so the window looks like one thing while it is half moved. |
| **Rebuilt components use Tailwind v4 and base-ui** | T3 Code's stack, so its class strings and its primitives (menu, popover, tooltip, dialog, combobox) carry over nearly as they are, and a port reads like its source. Tailwind's theme is mapped onto the same tokens, and its reset (preflight) is left out, so the components not yet rebuilt are drawn exactly as before. A primitive taken from T3 Code keeps its MIT notice. |
| **The font** | Inter, bundled (`@fontsource-variable/inter`, SIL OFL), in T3 Code's order: the Apple faces on a Mac, Inter everywhere else. On Windows the window was Segoe UI, which is most of why it did not look like T3 Code there. |
| **Code is copied only with credit** | Where a T3 Code component is ported rather than re-derived, the file says so and carries the MIT notice. Most of the work is re-deriving, since the stack differs. |

## What has to survive

Nothing below may be lost in the move; each phase checks its part.

- **The sandbox, in view.** Events (allow and deny, grouped by endpoint, with
  the right-click to open or block), Policy, Ports and previews, Facts, the
  mark on a session that is not sandboxed, and toolchains in the create form.
- **Trackers.** Tickets and pull requests that are yours, a ticket or pull
  request opened beside the work, starting a session from either, review
  comments sent to the agent, and the notifications.
- **The list's meaning.** Projects and their sessions, the order kept on the
  server (so the phone shows the same one), queued and scheduled sessions, a
  session waiting on a permission prompt standing out from one that is working.
- **More than one conversation, and shells,** per session.
- **Search across kept conversations,** servers, account, integrations,
  settings, updates.

## The shape, before and after

| Now | After |
|---|---|
| A title bar across the window: the wordmark and version, the server picker, and five icons (new project, search, integrations, servers, account, settings). | No bar of its own. The sidebar's top holds the name and a collapse toggle, then search and the new-session and new-project buttons. Integrations, servers, account and settings move to the sidebar's foot. |
| The tree: projects as headings, a two-line row per worktree. | The same groups, with T3 Code's row: the title at full weight, the branch under it, the state as a pill or a time on the right, the provider or sandbox mark beside the branch. A search field filters it. |
| Tabs over the middle: Agent, more chats, shells, files. | A header with the breadcrumb (project / session title) and toggles on the right for the terminal drawer and the right panel. Extra chats stay as tabs under the header; shells move to the drawer. |
| The dock, always open: files, git, events, policy, ports, facts. | The right panel, opened by its toggle or by what needs it (a diff, a denial). Its tabs: Changes, Files, Sandbox (events and policy), Ports, Facts. |
| The tickets sidebar, a fourth column. | Tickets and pull requests in the sidebar, under the sessions and above the foot, with a count; opening one shows it in the middle, as T3 Code shows an issue. |

## Phases

- [ ] **1. Foundation.** The palette (T3 Code's neutrals in dark and light),
  Inter, the type scale, radii and the shared controls (buttons, selects,
  menus, tooltips, inputs) restyled through the tokens. No layout change, so
  every screen gets the new material at once and nothing moves.
  *Started:* the palette in both themes, the states in T3 Code's hues, Inter,
  icons at its 1.75 stroke, the shared menu, and the conversation's prose and
  bubble; buttons, inputs and selects came with phase 8. Tooltips are T3
  Code's: every `title` in the window is drawn by one layer (`tips.ts`) as
  its popover card, with Base UI's timing (600 ms to open, the next at once)
  and a trailing shortcut such as "(Ctrl+Shift+F)" drawn as a key, so no
  control had to change. Still to do: the type scale outside the
  conversation, sidebar and composer.
- [ ] **2. The composer.** First the restyle, then the rebuild.
  *Done in the restyle:* T3 Code's card and footer (attach on the left; the
  mode, the model and effort as one "Opus 5.5 High" control, the context
  meter, and the round send and stop on the right), the `/` menu as a drawer
  on the card's edge, 64 pixel attachment tiles, a draft kept per
  conversation, and earlier prompts on the up arrow.
  *Done in the rebuild:* `@` to mention a file or folder in the working copy.
  `hurad` answers a new `FindFiles` request by listing what git tracks (or
  every file, outside git) in the sandbox and ranking it (`files::rank`: the
  name, then the path, then the letters in order); the composer asks a moment
  after typing stops, offers the paths in the drawer with their file icons,
  and inserts `@path`, which Claude Code reads as a file to look at. A mention
  is drawn as a chip in T3 Code's colour by the highlight layer under the
  textarea, so no rich-text editor was needed for it. A server too old to
  answer says so in the drawer. Queued messages are dashed bubbles on your
  side of the conversation, with when they will be said and Remove.
  While a turn runs, Enter queues a message for after it, as before, and
  Ctrl+Enter (Command and Return on a Mac), or a click with the key held,
  stops the turn and sends it now: T3 Code's steer. The send button's glyph
  says which, and changes while the key is held. A queued message's Edit
  takes it off the queue and back into the box. The agent's questions are
  asked from a drawer on the box: the question, its choices numbered so a key
  picks one (a single choice moves on to the next question by itself), the
  box for an answer of your own, and Previous, Next Question or Submit in
  place of send; permission prompts and plans stay in the conversation, as
  in T3 Code.
  *Not done, on purpose:* a model picker with search. T3 Code's is for
  dozens of models across providers; Claude Code offers a handful, which the
  menu holds.
- [ ] **3. The shell.** The title bar goes; the sidebar gets its head and
  foot; the middle gets the header with the breadcrumb and the panel toggles;
  the dock becomes the right panel and closes by default.
  *Done:* Tailwind v4 and base-ui in, with T3 Code's token names mapped onto
  the palette and its reset scoped to rebuilt components (`data-tw`); its
  button and toggle ported (`src/ui`). The title bar is gone. The sidebar
  (`shell/Sidebar.tsx`) has the collapse toggle and the name, a search field
  that narrows the list, new project and new worktree, and a foot of icons:
  settings, tickets and pull requests with their counts (which open and put
  away the tickets sidebar), conversation search, integrations, servers and
  account. It stays beside every screen. The header over the conversation
  (`shell/WorkspaceHeader.tsx`) has the breadcrumb and the panel's toggle,
  and folded panels leave no rail. macOS draws the sidebar black rather than
  in the system's material.
  *Still to do:* the dock as T3 Code's right panel with its own 52 pixel tab
  strip (phase 6), and checking the traffic lights on a Mac.
- [x] **4. The sidebar's rows,** and search in the list.
  Rebuilt in `Tree.tsx` on T3 Code's project folders and thread cards: a
  32 pixel folder row per project with its monogram (T3 Code's, hashed from
  the name to the same colour), a count when folded and a `+` on hover; each
  worktree a card with its branch and its state as a glyph and a word
  ("Working", "Setting up", "Needs you", "Failed") or its age, the title, and
  a line for its pull request and checks, the diff, and the sandbox's
  signals; a queued session as a slim row. Ordering by drag or Option and an
  arrow, the menus and renaming in place are as they were.
- [ ] **5. The conversation.** Your messages in a quiet bubble rather than the
  accent, the agent's replies as prose, a turn's steps folded into "Worked
  for …", and the changed-files card with a way into the diff.
  *Done:* the bubble and the prose (phase 1), the column at T3 Code's 736
  pixels, and a turn's work as its work log: "Worked for 4m 46s" by Claude
  Code's own count of the turn ("Working for" and a running clock while it
  runs, what the steps came to in its tooltip), and each tool call a one-line
  row with a glyph for its kind and the chevron at the end, its answer
  indented under it. *Still to do:* actions on your messages on hover, and
  the queued messages as dashed bubbles. T3 Code no longer has a per-turn
  changed-files card, so that part is dropped.
- [ ] **6. The right panel and the drawer.** The Sandbox tab (events and
  policy together), Changes with the git actions T3 Code puts in its summary
  card, and shells in a drawer under the conversation.
  *Done:* the panel stands beside the conversation's column at full height,
  with T3 Code's 52 pixel strip of labelled tabs level with the header. The
  Sandbox tab holds the feed and the policy together, as Activity and Rules.
  Shells live in a drawer under the conversation, T3 Code's: its own strip of
  them, a grip to make it taller (remembered), a toggle at the end of the tab
  strip and Ctrl+` to show it, a new shell opening it on that shell, and every
  terminal kept attached while it is shut. Loading is T3 Code's everywhere:
  lucide's LoaderCircle turning once a second where something waits, no
  spinner on "Working for", and a running tool call's label swept by its
  shine. *Not done:* a separate Changes tab, since the Git tab already is one.
- [x] **7. Trackers in the sidebar,** and the ticket and pull request views in
  the middle in T3 Code's issue style.
  Tickets and Pull Requests are pages in the middle, opened from the
  sidebar's foot (where each has its count) or the shortcut, and the column
  on the right is gone. A page has T3 Code's header (the title and how many,
  the switch between the two, refresh and close) and its rows at a reading
  width; a ticket or pull request opens in the panel on the right, now docked
  at the window's edge with a hairline, and is put away when the screen
  changes. "Seen" (what clears the phone's badge) is now the page being open.
- [x] **Agents' marks, for more than one agent.** `agents.tsx` is the
  registry: Claude Code (Anthropic's mark) and Codex (OpenAI's) so far, keyed
  by the agent a session's record names, and a plain glyph with the name for
  one it does not know. The mark is where T3 Code puts a provider's: before
  the model in the composer, at the end of each sidebar card, and on the
  agent's tab. The marks are their makers' and are used only to say which
  agent a session runs.
- [x] **Background work in view.** The strip above the composer that held
  the subagents holds the commands left running in the background too: a
  Bash call with `run_in_background`, followed through Claude Code's
  `local_bash` task events (and `KillShell`), with its status and how long it
  has run. One opens beside the conversation with the command, Claude Code's
  summary when it ends, and the end of its output, which `hurad` reads from
  the file Claude Code writes it to (a new `TailFile` request, the last 64 KB,
  read again every two seconds while it runs). The call's card in the
  conversation links to it.
- [x] **8. The rest:** Settings, the create forms, dialogs, the empty states,
  and the light theme checked screen by screen.
  Through the shared controls rather than screen by screen: fields and
  selects at T3 Code's 30 pixels and 8 round, its default and outline
  buttons, its settings groups (12 round, a softened border, the card colour
  at low strength), screen headers at 52 pixels with the title at the
  breadcrumb's size, and dialogs as its card with a hairline ring. Rebuilt
  components are kept out of the old field rules (`[data-tw]`). Checked in
  dark and light: the workspace, Settings and the create form.

## More of T3 Code

Taken after the phases, from T3 Code's features list. Each was checked in the
running window against `scripts/fake-hurad.py`, which now answers the new
requests too.

- [x] **Command palette.** Ctrl+K (Command+K on a Mac): worktrees, projects,
  tickets and pull requests, and things to do, in groups, narrowed as you
  type. The things to do include each project script and each port to
  preview. Ctrl+K in a terminal stays the shell's.
- [x] **Sidebar triage.** A worktree can be pinned to a shelf at the top,
  snoozed until an hour from now, this evening, tomorrow or next week, or
  settled into a shelf at the bottom once its work is done. One waiting on
  you is never hidden by any of it. Kept in this window (`triage.ts`).
- [x] **Project scripts.** A project's commands (its dev server, its tests)
  in the header over the conversation: the first as a button, the rest in its
  menu, a dialog to change them, each with one of T3 Code's six icons. One
  runs in a new shell in the drawer, named for the script: `NewShell` takes
  the command (`run`) and the server types it in once the shell is up. Kept
  in this window per server and project (`scripts.ts`).
- [x] **Preview in the window.** A port's page in a tab beside the
  conversation, through the same preview a browser gets (`previews.rs`), with
  reload, a path, a phone's width and the way out to a browser. A sandboxed
  frame without top navigation; the CSP allows frames from `localhost` only.
- [x] **Edit from here.** Hovering a message you sent offers to say
  something else in its place. That starts a new conversation forked from
  just before it (`NewChat` with `fork` and `at`, the host's `forkSession`
  and `resumeSessionAt`), and the one edited stays as it was.
- [x] **Side chat.** `/btw` (or `/side`) asks a question in a conversation
  forked from this one, in a panel beside it, so the turn in progress goes on.
  Closing the panel ends it.
- [x] **Prompt stash.** Ctrl+S (Command+S) in the composer puts what is
  written aside; the tab over the box gives it back. One stash for the
  window (`chat/stash.ts`).
- [x] **Appearance and keys.** A VS Code colour theme taken in from its JSON
  (a file or the clipboard) becomes the palette, in its own light or dark,
  terminal colours included (`look.ts`). The faces for text and code, the
  code size (terminals, files, diffs), and the whole window's size. Every
  shortcut in one registry (`keys.ts`), rebound in Settings by pressing the
  new keys; the sidebar, a new worktree and the first script have none until
  given one.

- [x] **Settings as a card.** T3 Code's `AppRouteModal`: up to 80 by 60 rem
  over the window, which stays drawn behind it blurred, with its pages down
  the left in groups (General, Appearance and Keybindings unheaded; Server:
  New Worktrees and Integrations; Connections: Servers, Trackers and
  Account; About), a search over the pages and their settings that goes to
  the setting and lights it, and a header with the breadcrumb, the page's
  own action (Save for the server's defaults) and close. The window's own
  pages and the server's defaults are rebuilt in T3 Code's rows, with its
  Switch and Input ported; Integrations, Servers, Trackers, Account and
  About are the old screens drawn as pages without their header. The
  sidebar foot's buttons open the card at their page.
- [x] **The root at 16 pixels.** It was 13, so every rem in the rebuilt
  components (all of Tailwind's sizes) came out at 13/16 of T3 Code's, while
  the restyled controls were already at its pixels. The body keeps 13.

- [x] **T3 Code's file icons.** Pierre's file-tree set (`@pierre/trees`,
  Apache-2.0) with its colour per kind of file, and the icons T3 Code adds
  over it that are not its own drawing: documents and video from Lucide,
  language marks from Simple Icons, test files and a few tools from Pierre's
  VS Code icons. Folders are Lucide's, muted. One sprite in the document;
  the Material Icon Theme and its twelve hundred files are gone.

- [x] **Worktrees on a schedule.** T3 Code's scheduled tasks, for
  worktrees: every day, every weekday or one day a week at a time, or every
  hour. A repeating entry is a template on the server's queue
  (`hura_core::queue::Repeat`) that never starts itself: each time it comes
  round a copy starts under a fresh name and its time moves on, once after a
  server was down rather than once per missed time. The time of day is on the
  server's clock, which the queue now says (`utc_offset`), and the form moves
  a time picked here onto it. Offered only by a server that says it.

- [x] **A new worktree is a draft.** T3 Code's new thread: no dialog, the
  workspace itself, with "What should we build in ‹project›?" over a
  composer, the project a menu in the headline. Enter sends, and sending is
  starting; Escape in an empty box leaves. The agent and when it starts are
  the composer's controls; where it runs (a sandbox or a worktree), the base
  branch, the policy and the name are the strip at its foot; toolchains,
  credentials, skills and MCP servers are under More.

- [x] **Usage.** T3 Code's usage card, from the sidebar's foot: the
  account's rate-limit windows, what was spent in all, and each worktree by
  cost, from the status polls. A row goes to its worktree.
- [x] **Diffs.** T3 Code's Diffs settings: side by side or stacked, and
  whitespace-only changes hidden or not, for every diff, the ones open too.
- [x] **The server in the draft.** With more than one paired, the strip under
  the draft's box says which server it starts on and offers the others; the
  draft follows to the same repository's project there.
- [x] **Per-project defaults.** Kept with each project on the server
  (`ProjectDefaults`): agent, sandbox or worktree, policy, base branch,
  toolchains and credentials, each the server's unless set. Settings has a
  Projects page; the draft opens with them. `hurad new` works by repository,
  not project, so it does not read them yet.
- [x] **Recurring messages.** A message scheduled in the composer can come
  round again, once a day, on weekdays, on its weekday or every hour, the
  same `Repeat` a worktree's schedule is; the queue moves its time on each
  time it is said.
- [x] **Groups within a project.** hura's own, in T3 Code's look: named,
  foldable groups of related worktrees inside a project's folder, its groups
  first and the rest after. Made from the project's menu or a worktree's ("New
  Group…"), joined from a worktree's menu or by dropping it on the header, left
  by "Take Out of the Group" or a drop among the ungrouped. Folded, a group
  says how many need you. This window's, per server (`groups.ts`), as the
  triage is.
- [ ] **Later: Codex as a second agent.** The marks are ready; it needs a
  Codex host in the sandbox image beside `hura-agent.mjs`.

Needs a release: `NewChat`'s `fork` and `at`, `Enqueue`'s and `QueueMessage`'s `repeat`, `SetProjectDefaults`, and `NewShell`'s `run`, are new
requests to `hurad`, and forking needs the new `hura-agent.mjs` in the image.
An older server ignores the fields: the side chat and an edited message start
empty, and a script's shell opens without running it. The window offers a
schedule only to a server whose queue says its clock, since an older one would
start a repeating worktree once, now.
