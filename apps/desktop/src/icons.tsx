// The icons: lucide, pinned to one grid, plus the file tree's, which are VS
// Code's.
//
// This file used to argue against an icon set, and the argument was really
// about consistency rather than about packages: a set arrives with its own idea
// of optical size and stroke weight, and fifteen icons from it beside fifteen
// drawn here would read as two families. `LucideProvider` in `main.tsx`
// settles that centrally -- every lucide icon in the window renders at one size
// and one stroke, whatever the library's own defaults are -- so the objection
// is answered rather than accepted. The file and folder icons at the bottom are
// the exception, and say why.
//
// **Every glyph the window uses is named here, and nothing else imports
// `lucide-react`.** That rule is what keeps the vocabulary honest now that the
// window says most of what it has to say in pictures: an icon has become a
// *term*, and a term used in two panes has to be the same picture in both. A
// `RefreshCw` imported straight from the library in one pane beside a
// `RotateCw` in another would be two words for one idea, which is the one
// confusion an icon-led interface cannot afford -- there is no label beside it
// to correct the guess. So the renames below are the vocabulary: `Stop`,
// `Store`, `Publish` say which button they sit on rather than what shape they
// are, and the day one of them is drawn better the change is a line in this
// file instead of a find-and-replace across the app.
//
// `absoluteStrokeWidth` is the part that makes the pinning work. lucide draws
// on a 24-grid and scales the stroke with the icon, so one `strokeWidth` at
// 14px and at 20px are two different weights on screen; with it set, the number
// *is* the rendered width in pixels, and an icon can be resized without
// changing weight. See `ICON_STROKE` below.
//
// Monaco does bundle codicons, and reusing them was still the wrong
// alternative: it would tie the window's chrome to a version of an editor it
// happens to embed, and the file tree would change shape the day Monaco is
// swapped.

import {
  Activity,
  AppWindow,
  Gauge,
  Repeat,
  Settings2,
  Palette,
  Keyboard,
  Layers,
  FlaskConical,
  Hammer,
  ListChecks,
  ArrowDownToLine,
  ArrowLeft,
  ArrowUp,
  ArrowUpFromLine,
  BadgeInfo,
  Binary,
  BookOpen,
  Box,
  Bot,
  Bug,
  CalendarClock,
  ChevronDown,
  ChevronRight,
  Check,
  CheckCheck,
  CircleAlert,
  CircleArrowUp,
  CircleCheck,
  CircleQuestionMark,
  CircleUserRound,
  CircleX,
  CircleDashed,
  CloudDownload,
  Download as DownloadGlyph,
  CloudUpload,
  ClipboardPaste,
  Copy as CopyGlyph,
  ExternalLink,
  FileDiff,
  FolderGit2,
  FolderOpen,
  FolderPlus,
  FolderSearch,
  FolderTree,
  Ghost,
  Globe,
  GlobeLock,
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
  GitPullRequestDraft,
  Eye,
  Hourglass,
  Info,
  LoaderCircle,
  Monitor,
  Package,
  PanelRightClose,
  PanelRightOpen,
  PanelLeft,
  PanelBottom,
  PanelRight,
  SquarePen,
  Paperclip,
  KeyRound,
  List,
  ListMinus,
  ListPlus,
  ListTree,
  MessageSquare,
  MessagesSquare,
  Pencil,
  Play,
  Plug,
  Plus as PlusGlyph,
  Minus as MinusGlyph,
  RefreshCw,
  RotateCw,
  Smartphone,
  ScrollText,
  Save,
  Search,
  SlidersHorizontal,
  Server as ServerGlyph,
  ServerOff,
  Settings as SettingsGlyph,
  Shield,
  ShieldBan,
  ShieldCheck,
  ShieldOff,
  ShieldQuestion,
  Lock,
  FileText,
  FilePen,
  CornerUpRight,
  Bookmark,
  Pin as PinGlyph,
  MessageCircleMore,
  AlarmClock,
  ListTodo,
  Wrench,
  PenLine,
  NotebookPen,
  Sparkles,
  Square,
  Tag,
  Terminal as TerminalGlyph,
  TriangleAlert,
  Trash,
  Undo2,
  Unplug,
  X,
  TextSelect,
  MoveUp as MoveUpGlyph,
  MoveDown as MoveDownGlyph,
  Ellipsis,
  Brain,
} from "lucide-react";

import type { State } from "./gen/State";

/// The grid every icon in this window is on: 14 pixels across, with a stroke
/// of `ICON_STROKE` *actual* pixels. Applied to lucide through its provider in
/// `main.tsx` and to the hand-drawn glyphs below by hand, which is the whole
/// reason both numbers are named here rather than written twice.
export const ICON_SIZE = 14;
/// 1.75 rather than lucide's 2, which is what T3 Code sets its icons to: a
/// full two pixels beside 13 and 14 pixel text reads as bold, and the 1.25
/// this used to be reads as faint on its black.
export const ICON_STROKE = 1.75;
/// The one deliberate exception to the 14-grid: the glyph that stands in for a
/// pane with nothing in it. See `Empty.tsx`.
///
/// Passed as lucide's `size` rather than set in CSS, and that is not a style
/// preference -- it is the only way to resize one of these without changing its
/// weight. `absoluteStrokeWidth` works by dividing the stroke by the size at
/// render time, so a `width: 26px` in a stylesheet scales a stroke computed for
/// 14 and lands at 2.3 actual pixels: a bold icon in a window that has none.
export const ICON_BIG = 26;
/// The same glyph when it has the whole middle of the window to itself -- a
/// first run with no server, or no worktree selected. Larger because the
/// alternative at this scale is a 26-pixel mark adrift in six hundred, which
/// reads as a rendering fault rather than as a statement.
export const ICON_PAGE = 40;

// ---------------------------------------------------------------------------
// The vocabulary, grouped by where it is spoken. A rename per icon is worth
// it: `Forget` says which button it is on and `Trash` does not.
// ---------------------------------------------------------------------------

// The header's five destinations. Icon-only, and the window's own nav: see
// `App.tsx` for why the labels went and what replaced them.
export const NewProject = FolderPlus;
/// The window's frame, as T3 Code draws its toggles: the sidebar on the left,
/// the panel of files, git and the sandbox on the right, and a new worktree in
/// the project in front.
export const SidebarToggle = PanelLeft;
export const DrawerToggle = PanelBottom;
/// A session's state in the sidebar, as T3 Code labels a thread's: working
/// (and being set up), waiting on you, and failed. Static, as T3 Code's are;
/// the word beside each says it.
export const StatusWorking = CircleDashed;
export const StatusNeedsYou = ShieldQuestion;
export const StatusFailed = CircleAlert;
/// A tool call's kind, on the left of its row in a turn's work, as T3 Code's
/// work log draws them.
export const ToolRead = FileText;
export const ToolEdit = FilePen;
export const ToolRun = TerminalGlyph;
export const ToolSearch = Search;
export const ToolTodo = ListTodo;
export const ToolOther = Wrench;
/// The send button while a turn runs, T3 Code's two: say it after the turn
/// (the default), or stop the turn and say it now (with Ctrl or Command held).
export const SendQueued = ListPlus;
/// Prompts put aside, T3 Code's stash: a bookmark, as its is.
export const Stash = Bookmark;
/// A side chat: a question asked beside the conversation (`/btw`).
export const SideChat = MessageCircleMore;
/// Keeping the sidebar to what needs you (`triage.ts`): pinned to the top,
/// snoozed until a time, settled as done for now, and the way back.
export const Pin = PinGlyph;
export const Snooze = AlarmClock;
export const Settle = CircleCheck;
export const Undo = Undo2;
export const SendSteer = CornerUpRight;
/// A group's twisty, turned by CSS rather than swapped, as T3 Code's is.
export const Disclosure = ChevronDown;
export const PanelToggle = PanelRight;
export const NewSession = SquarePen;
export const Integrations = Plug;
export const Servers = ServerGlyph;
export const Settings = SettingsGlyph;
export const Account = CircleUserRound;
/// What did not fit, behind a button: the sidebar foot's overflow.
export const More = Ellipsis;
/// What the agent thought, where a turn under way shows its latest: T3 Code's
/// glyph for reasoning.
export const Thought = Brain;
/// What the agent said on the way, beside it.
export const Said = MessageSquare;
/// Versions, updates and links. Opened from the wordmark rather than the
/// strip, but headed by this glyph like every other screen.
export const About = BadgeInfo;

// The dock's five panes, in the order the strip shows them.
export const Files = FolderTree;
export const Branch = GitBranch;
export const Events = Activity;
export const Policy = Shield;
/// The session record -- what it was created with, and what it has spent.
/// `Info` and not a document: it is a read-out, not a file.
export const Record = Info;
/// What is listening in the sandbox, and the previews of it.
export const Ports = Globe;

// The events pane's two readings of one feed: folded by endpoint, the way a
// tree folds files under a folder, or flat, as the runtime said it.
export const ByEndpoint = ListTree;
export const AsLog = List;

// The traffic feed, where an endpoint is opened or closed. Both are the
// policy pane's shield with a verdict on it: the change is to the rules, made
// from beside the evidence.
export const Grant = ShieldCheck;
export const Revoke = ShieldBan;

// git, where the four operations are four different arrows on purpose.
//
// `Fetch` and `Pull` are the distinction worth drawing: both bring refs down
// and only one touches the working copy, so one is a cloud and the other is an
// arrow at a line. `Publish` is a first push -- a branch the remote has never
// heard of -- and wears a different glyph from `Push` for the same reason the
// button used to carry a different word.
export const Fetch = CloudDownload;
export const Pull = ArrowDownToLine;
export const Push = ArrowUpFromLine;
export const Publish = CloudUpload;
export const Refresh = RefreshCw;
export const Revert = Undo2;
/// The commit button, beside its word.
export const Commit = GitCommitHorizontal;
/// Every file in a section at once: the row's `Plus` and `Minus`, as a list.
export const StageAll = ListPlus;
export const UnstageAll = ListMinus;

// The integrations screen: containers to run, credentials to hold, trackers to
// read, skills to push.
export const Start = Play;
/// A session or a message waiting for a time.
export const Scheduled = CalendarClock;
/// A session waiting for room in the queue, or a message for its agent's turn
/// to end.
export const Queued = Hourglass;
export const Stop = Square;
export const Restart = RotateCw;
/// Storing a secret. A save and not a tick: the value is written somewhere the
/// window can never read it back, which is a filing action rather than a
/// confirmation.
export const Store = Save;
export const Secret = KeyRound;
/// A ticket, and the trackers they come from: a tag, as a label on a piece of
/// work. The ticket stub it replaced turned to noise at 14 pixels.
export const Tracker = Tag;
// A pull request, and one still a draft: the forge's own two marks for them.
export const PullRequest = GitPullRequest;
export const PullDraft = GitPullRequestDraft;
/// Starting a session that reads a pull request and reports back, rather than
/// one that works on it.
export const Review = Eye;
// A pull request's checks: passed, failed, still running.
export const Passing = CircleCheck;
export const Failing = CircleX;
export const Pending = CircleDashed;
// The sidebar on the right, folded to its rail and opened again.
export const Fold = PanelRightClose;
export const Unfold = PanelRightOpen;
/// Moving a project or a worktree one place along the sidebar.
export const MoveUp = MoveUpGlyph;
export const MoveDown = MoveDownGlyph;
/// Editing a line in place: a tracker's filter.
export const Edit = Pencil;
/// A ticket's comments, beside how many there are.
export const Comments = MessageSquare;
/// The tickets screen's other half: trackers, their tokens and filters. Not
/// the settings gear, which is the header's own destination.
export const Setup = SlidersHorizontal;
export const Skill = Sparkles;

// The create form's chips: each field is its glyph and its value, and the
// label is the tooltip. `Branch`, `Policy`, `Secret` and `Skill` above are
// reused rather than redrawn -- a policy is the same shield here as in the dock.
/// A toolchain in the sandbox image.
export const Toolchain = Package;
/// The agent the session runs, and the version of it.
export const Agent = Bot;

// The chat: a conversation with the agent, as opposed to its terminal.
/// A conversation tab, and the menu entry that opens one. Two bubbles rather
/// than the one `Comments` already means, so the review and the chat are not
/// one word.
export const Chat = MessagesSquare;
/// Sending a message. An arrow up, which is where the message goes.
export const Send = ArrowUp;
/// A file sent with a message: the composer's button that picks one, and the
/// file once it is in the message.
export const Attach = Paperclip;
/// The permission modes, as the composer's mode control draws them: T3 Code's
/// glyphs for the same four ideas.
export const ModeAuto = ShieldCheck;
export const ModeAsk = Lock;
export const ModeEdits = PenLine;
export const ModePlan = NotebookPen;
/// A file taken out of the sandbox, into this computer's Downloads folder.
export const Download = DownloadGlyph;
/// A message the sandbox has, and one Claude Code has taken into a turn: the
/// receipts a messenger uses, because they are the ones already understood.
export const Delivered = Check;
export const Read = CheckCheck;
/// A shell, in the menu that opens either: a prompt to type at.
export const Shell = TerminalGlyph;
/// This window, as opposed to the server it talks to.
export const Desktop = Monitor;

// The facts and policy panes: what a session is made of.
/// The repository a session cloned.
export const Repo = FolderGit2;
/// The sandbox itself, the box everything else is inside.
export const Sandbox = Box;
/// A session with no sandbox: a worktree on the server, with its rights.
export const Unsandboxed = ShieldOff;

// The about screen.
/// A newer release is on offer. The header's badge and the screen's button.
export const Upgrade = CircleArrowUp;
export const Notes = ScrollText;
export const Docs = BookOpen;
export const Report = Bug;
/// Something in progress that has no other picture: a check, a download.
export const Busy = LoaderCircle;
/// Worth reading before going ahead -- work that stays behind on the host.
export const Heads = TriangleAlert;

// The chrome.
export const Plus = PlusGlyph;
export const Minus = MinusGlyph;
export const Close = X;
export const Back = ArrowLeft;
export const Find = Search;
/// Opening a ticket in the browser. The one icon in the window that means
/// "this leaves the window".
export const Elsewhere = ExternalLink;
/// A port's page, in a tab of the window rather than a browser.
export const PreviewHere = AppWindow;
/// The preview at a phone's width.
export const Phone = Smartphone;
/// A project's scripts (`scripts.ts`), by the icon each was given.
export const ScriptRun = Play;
export const ScriptTest = FlaskConical;
export const ScriptBuild = Hammer;
export const ScriptLint = ListChecks;
export const ScriptDebug = Bug;
export const ScriptConfigure = Wrench;
/// The settings' own pages, as T3 Code's nav marks them.
export const SettingsGeneral = Settings2;
export const SettingsAppearance = Palette;
export const SettingsKeys = Keyboard;
/// What a new worktree starts with: the server's defaults.
export const SettingsSessions = Layers;
export const SettingsDiffs = FileDiff;
export const SettingsProjects = FolderGit2;
/// What the worktrees have spent, and what the account has left.
export const UsageGlyph = Gauge;
/// A group of related worktrees within a project, in the sidebar.
export const GroupGlyph = FolderPlus;
/// A worktree that starts again and again, on the server's queue.
export const Recurring = Repeat;
export const Forget = Trash;
/// The current choice in a dropdown.
export const Chosen = Check;
/// Putting a name, a path or an endpoint on the clipboard, from a menu.
export const Copy = CopyGlyph;
/// Putting the clipboard into a terminal, from its menu.
export const Paste = ClipboardPaste;
/// Selecting everything a terminal has on screen.
export const SelectAll = TextSelect;
/// Back to the live screen from a terminal scrolled into its history.
export const Latest = ArrowDownToLine;
/// Opening a file's diff, from the change list's menu.
export const Diff = FileDiff;

// ---------------------------------------------------------------------------
// The absences. One glyph per kind of nothing, and they are a vocabulary in
// their own right -- see `Empty.tsx`, which is the only thing that renders
// them.
//
// Each is the *subject* of the pane it stands in, not a general-purpose shrug:
// a pane with no policy decisions shows the feed's own glyph gone quiet, which
// says "this is the feed, and it is empty" in one mark. A single shared "no
// data" symbol would have said "something is missing" five times without ever
// saying what.
// ---------------------------------------------------------------------------

/// Not paired with anything. The first screen a new install shows.
export const NoServer = ServerOff;
/// No repositories where the server was told to look.
export const NoRepos = FolderSearch;
/// A directory with nothing in it, and the tree's own shape for it.
export const NoFiles = FolderOpen;
/// Nothing has changed in the working copy. A tick, because a clean tree is a
/// state rather than a shortfall -- the one absence in the window that is good
/// news.
export const Clean = CircleCheck;
/// No MCP servers configured. The integrations glyph with the plug pulled.
export const NoIntegrations = Unplug;
/// Nothing listening in the sandbox. The ports glyph, closed.
export const NoPorts = GlobeLock;
/// A file Monaco will not be shown.
export const NotText = Binary;
/// A session whose sandbox is gone, or was never made: the record outliving
/// the thing it describes. Not a box with a cross through it, which lucide
/// does not have, and not `PackageX`, which would read as no toolchain: the
/// package is the toolchain glyph, beside the sandbox's box in the facts pane.
export const NoSandbox = Ghost;

type Props = { className?: string; title?: string };

/// The file tree's twisty. Down when expanded, right when not -- the rotation
/// is two glyphs rather than a CSS transform so the stroke ends stay on the
/// pixel grid at 14px.
export const Chevron = ({ open, ...p }: Props & { open: boolean }) =>
  open ? <ChevronDown {...p} /> : <ChevronRight {...p} />;

/// The twisty of a fold that opens by growing (`Fold` in `Fold.tsx`), which
/// turns as it opens rather than swapping glyphs. The right-pointing one, a
/// quarter turn by CSS: turned about the middle of a 14-pixel box, every point
/// on the grid lands on the grid, and `ChevronRight` turned a quarter is
/// `ChevronDown` point for point, so at rest it is the same pixels as
/// `Chevron`.
export const Twisty = ({ open, className, ...p }: Props & { open: boolean }) => (
  <ChevronRight {...p} className={`twisty${open ? " open" : ""}${className ? ` ${className}` : ""}`} />
);

/// What the agent in a session is doing, as one fixed-size mark.
///
/// Fixed-size is the requirement, not a detail: this sits in a column to the
/// left of every worktree's name, and a mark that changed size with the state
/// would shuffle the name of every row each time an agent started or stopped.
/// Every branch below therefore renders into the same 14-pixel box.
///
/// The colours are in `style.css`, keyed on the state name, for the same reason
/// the palette is: one place to change what `waiting` looks like.
/// An agent working, or a session being made ready for one: T3 Code's
/// spinner, lucide's `LoaderCircle` turning once a second (`.working` in
/// style.css). Sized by `size` like the other glyphs, so the empty states can
/// pass theirs, and coloured by `currentColor` like them too.
export function Working({ size = ICON_SIZE, className }: { size?: number; className?: string }) {
  return <LoaderCircle size={size} className={`working ${className ?? ""}`} aria-hidden />;
}

export function StateDot({ state, className }: { state: State; className?: string }) {
  const box = `state-dot ${state} ${className ?? ""}`;

  switch (state) {
    // In progress, and the two are worth telling apart: `running` is an agent
    // working, `creating`/`seeding` is the sandbox not being there yet. Same
    // mark, different hue, because the thing you do about them is the same
    // (wait) and the thing they mean is not.
    case "running":
    case "creating":
    case "seeding":
      return (
        <span className={box} role="img" aria-label={state}>
          <Working />
        </span>
      );

    // The one state the window exists to tell you about, so it gets a glyph
    // rather than a dot: a shape is findable in a list of twelve rows in a way
    // that a colour is not, and it is the row you are *not* looking at.
    case "waiting":
      return (
        <span className={box} role="img" aria-label="waiting for input">
          <CircleQuestionMark />
        </span>
      );

    case "failed":
    case "dead":
      return (
        <span className={box} role="img" aria-label={state}>
          <CircleAlert />
        </span>
      );

    // Healthy and doing nothing. A plain dot, and deliberately the quietest
    // mark here: it is what most rows are most of the time, and a list where
    // every row draws attention has none left for the row that should.
    default:
      return (
        <span className={box} role="img" aria-label={state}>
          <span className="dot" />
        </span>
      );
  }
}
// ---------------------------------------------------------------------------
// Files and folders: T3 Code's, from Pierre's file-tree icons. See
// `fileIcons.tsx`.
// ---------------------------------------------------------------------------

export { FILE_ICON_SIZE, FileIcon, Folder } from "./fileIcons";
