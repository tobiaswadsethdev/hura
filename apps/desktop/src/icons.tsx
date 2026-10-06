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
  ArrowDownToLine,
  ArrowLeft,
  ArrowUpFromLine,
  BadgeInfo,
  Binary,
  BookOpen,
  Bot,
  Bug,
  ChevronDown,
  ChevronRight,
  Check,
  CircleAlert,
  CircleArrowUp,
  CircleCheck,
  CircleQuestionMark,
  CloudDownload,
  CloudUpload,
  Copy as CopyGlyph,
  ExternalLink,
  FileDiff,
  FolderOpen,
  FolderPlus,
  FolderSearch,
  FolderTree,
  Globe,
  GlobeLock,
  GitBranch,
  GitCommitHorizontal,
  Info,
  LoaderCircle,
  Monitor,
  Package,
  KeyRound,
  ListMinus,
  ListPlus,
  MessageSquare,
  Pencil,
  Play,
  Plug,
  Plus as PlusGlyph,
  Minus as MinusGlyph,
  RefreshCw,
  RotateCw,
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
  Sparkles,
  Square,
  Ticket,
  TriangleAlert,
  Trash,
  Undo2,
  Unplug,
  X,
} from "lucide-react";

import {
  fileExtensions,
  fileNames,
  folderNames,
  folderNamesExpanded,
} from "material-icon-theme/dist/material-icons.json";

import type { State } from "./gen/State";

/// The grid every icon in this window is on: 14 pixels across, with a stroke
/// of `ICON_STROKE` *actual* pixels. Applied to lucide through its provider in
/// `main.tsx` and to the hand-drawn glyphs below by hand, which is the whole
/// reason both numbers are named here rather than written twice.
export const ICON_SIZE = 14;
/// 1.25 rather than lucide's 2. The window's text is 12 and 13 pixels, and a
/// two-pixel stroke beside it reads as bold -- which is what an icon set at its
/// default weight looks like dropped into an interface built at this scale.
export const ICON_STROKE = 1.25;
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
export const Integrations = Plug;
export const Servers = ServerGlyph;
export const Settings = SettingsGlyph;
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
export const Stop = Square;
export const Restart = RotateCw;
/// Storing a secret. A save and not a tick: the value is written somewhere the
/// window can never read it back, which is a filing action rather than a
/// confirmation.
export const Store = Save;
export const Secret = KeyRound;
export const Tracker = Ticket;
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
/// This window, as opposed to the server it talks to.
export const Desktop = Monitor;

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
export const Forget = Trash;
/// The current choice in a dropdown.
export const Chosen = Check;
/// Putting a name, a path or an endpoint on the clipboard, from a menu.
export const Copy = CopyGlyph;
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

type Props = { className?: string; title?: string };

/// The file tree's twisty. Down when expanded, right when not -- the rotation
/// is two glyphs rather than a CSS transform so the stroke ends stay on the
/// pixel grid at 14px.
export const Chevron = ({ open, ...p }: Props & { open: boolean }) =>
  open ? <ChevronDown {...p} /> : <ChevronRight {...p} />;

/// What the agent in a session is doing, as one fixed-size mark.
///
/// Fixed-size is the requirement, not a detail: this sits in a column to the
/// left of every worktree's name, and a mark that changed size with the state
/// would shuffle the name of every row each time an agent started or stopped.
/// Every branch below therefore renders into the same 14-pixel box.
///
/// The colours are in `style.css`, keyed on the state name, for the same reason
/// the palette is: one place to change what `waiting` looks like.
export function StateDot({ state, className }: { state: State; className?: string }) {
  const box = `state-dot ${state} ${className ?? ""}`;

  switch (state) {
    // In progress, and the two are worth telling apart: `running` is an agent
    // working, `creating`/`seeding` is the sandbox not being there yet. Same
    // spinner, different hue, because the thing you do about them is the same
    // -- wait -- and the thing they mean is not.
    case "running":
    case "creating":
    case "seeding":
      return (
        <span className={box} role="img" aria-label={state}>
          <span className="spinner" />
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
// Files and folders: the Material Icon Theme, as VS Code draws them.
//
// These used to be hand-drawn -- a page outline with a mark in it for eight
// kinds of file -- and they were the one place a monochrome line icon was the
// wrong answer. The tree is scanned, not read, and what the eye scans a
// directory for is the shape and colour of a file kind it already knows from
// the editor it spends the rest of the day in. So this borrows that editor's
// most-installed icon theme wholesale, mapping and all, rather than drawing a
// smaller one that is nearly it.
//
// They are full-colour pictures and not glyphs, so they sit outside the grid
// above: 16 pixels rather than 14, which is what VS Code renders them at, and
// no stroke to pin. Served as files rather than inlined -- `vite.config.ts`
// says why -- so a directory listing fetches the dozen it shows, not all of
// them.
// ---------------------------------------------------------------------------

/// What VS Code renders these at, and the row height of its explorer follows.
export const FILE_ICON_SIZE = 16;

/// Every icon the theme ships, by name: `rust` to the URL of `rust.svg`. A few
/// are recoloured copies of another, shipped as `<name>.clone.svg` and named
/// in the mapping without the `.clone`.
const URLS: Record<string, string> = Object.fromEntries(
  Object.entries(
    import.meta.glob<string>("/node_modules/material-icon-theme/icons/*.svg", {
      eager: true,
      query: "?url",
      import: "default",
    }),
  ).map(([path, url]) => [path.slice(path.lastIndexOf("/") + 1).replace(/(\.clone)?\.svg$/, ""), url]),
);

const lookup = (table: Record<string, string>, key: string) =>
  Object.hasOwn(table, key) ? table[key] : undefined;

/// The theme's icon for a filename, matched the way VS Code matches it: the
/// whole name first (`package.json`, `Dockerfile`), then the extension from the
/// longest down, so `foo.test.ts` is a test before it is TypeScript.
function fileIconName(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1).toLowerCase();
  const whole = lookup(fileNames, name);
  if (whole) return whole;
  for (let i = name.indexOf("."); i !== -1; i = name.indexOf(".", i + 1)) {
    const ext = lookup(fileExtensions, name.slice(i + 1));
    if (ext) return ext;
  }
  return "file";
}

function Icon({ icon, fallback, className }: { icon: string; fallback: string; className?: string }) {
  return (
    <img
      className={`file-icon ${className ?? ""}`}
      src={URLS[icon] ?? URLS[fallback]}
      width={FILE_ICON_SIZE}
      height={FILE_ICON_SIZE}
      // Decorative: the filename beside it is the name.
      alt=""
      draggable={false}
    />
  );
}

/// A file's icon, by what it is. Takes a path or a bare name.
export const FileIcon = ({ name, className }: { name: string; className?: string }) => (
  <Icon icon={fileIconName(name)} fallback="file" className={className} />
);

/// A directory, open or shut -- and, like VS Code, a `src` or a `.github` gets
/// a folder of its own.
export function Folder({ name, open, className }: { name: string; open: boolean; className?: string }) {
  const lower = name.toLowerCase();
  return open ? (
    <Icon icon={lookup(folderNamesExpanded, lower) ?? "folder-open"} fallback="folder-open" className={className} />
  ) : (
    <Icon icon={lookup(folderNames, lower) ?? "folder"} fallback="folder" className={className} />
  );
}
