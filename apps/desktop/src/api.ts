// The Rust side, as functions.
//
// Every type here is generated from the Rust that produces it -- see
// `scripts/gen-bindings.sh` -- so there is no second copy of a message to keep
// in step. `invoke` names have to match `main.rs`'s command names, which is the
// one string pair in this application that a compiler cannot check.

import { invoke } from "@tauri-apps/api/core";

import type { Comment } from "./gen/Comment";
import type { FailureKind } from "./gen/FailureKind";
import type { Inbox } from "./gen/Inbox";
import type { Issue } from "./gen/Issue";
import type { Integrations } from "./gen/Integrations";
import type { ConfiguredTracker } from "./gen/ConfiguredTracker";
import type { Tracker } from "./gen/Tracker";
import type { McpOp } from "./gen/McpOp";
import type { Dir } from "./gen/Dir";
import type { FeedEvent } from "./gen/FeedEvent";
import type { Against } from "./gen/Against";
import type { FileDiff } from "./gen/FileDiff";
import type { FileText } from "./gen/FileText";
import type { GitOp } from "./gen/GitOp";
import type { Status as GitStatus } from "./gen/Status";
import type { NewComment } from "./gen/NewComment";
import type { Picked } from "./gen/Picked";
import type { Listing } from "./gen/Listing";
import type { NewOptions } from "./gen/NewOptions";
import type { NewProject } from "./gen/NewProject";
import type { Project } from "./gen/Project";
import type { NewSession } from "./gen/NewSession";
import type { Poll } from "./gen/Poll";
import type { Session } from "./gen/Session";
import type { Settings } from "./gen/Settings";
import type { SettingsView } from "./gen/SettingsView";
import type { View as PolicyView } from "./gen/View";

export type ServerSummary = { name: string; address: string };

/// What `connect` answers with: the server just paired, the list it is now in,
/// and the version of the `hurad` that answered -- see `Paired` in main.rs.
export type Paired = { server: ServerSummary; servers: ServerSummary[]; version: string };

/// Hand-written because it is the bridge's own shape rather than a message: see
/// `GitAnswer` in main.rs. Both halves are generated types.
export type GitAnswer = { said: string; status: GitStatus };

/// This window's version and, when a server is named, that server's. See
/// `About` in main.rs.
export type About = {
  desktop: string;
  updater: boolean;
  server_version: string | null;
  server_error: string | null;
};

export const api = {
  about: (server: string | null) => invoke<About>("about", { server }),
  servers: () => invoke<ServerSummary[]>("servers"),
  // Pairing, which the CLI spells `hura connect` and `hura remotes --forget`.
  // Same checks either way: both call `hura_client::pair`.
  connect: (pairing: string, name: string | null) =>
    invoke<Paired>("connect", { pairing, name }),
  forget: (name: string) => invoke<ServerSummary[]>("forget", { name }),
  sessions: (server: string) => invoke<Session[]>("sessions", { server }),
  /// End a session and delete its sandbox. Answers with the list that is left.
  destroy: (server: string, name: string) =>
    invoke<Session[]>("destroy", { server, name }),
  poll: (server: string, name: string) => invoke<Poll>("poll", { server, name }),
  policy: (server: string, name: string) => invoke<PolicyView>("policy", { server, name }),
  events: (server: string, name: string) => invoke<FeedEvent[]>("events", { server, name }),
  // Changing what a running session may reach. Each answers with the policy
  // re-read, so the panes say what the sandbox has rather than what was asked.
  // `everywhere` also writes the global list every new session starts from.
  allow: (server: string, name: string, endpoint: string, binaries: string[], everywhere: boolean) =>
    invoke<PolicyView>("allow", { server, name, endpoint, binaries, everywhere }),
  block: (server: string, name: string, endpoint: string, everywhere: boolean) =>
    invoke<PolicyView>("block", { server, name, endpoint, everywhere }),
  // Off the global lists only; no sandbox is touched.
  unlist: (server: string, name: string, endpoint: string) =>
    invoke<PolicyView>("unlist", { server, name, endpoint }),
  diff: (server: string, name: string) => invoke<string>("diff", { server, name }),

  // The working copy, read-only: the agent owns it. One directory at a time,
  // as the tree is expanded -- every listing is an exec into the sandbox.
  files: (server: string, name: string, path: string) =>
    invoke<Dir>("files", { server, name, path }),
  file: (server: string, name: string, path: string) =>
    invoke<FileText>("file", { server, name, path }),

  // Git. Every mutation answers with what git said *and* the status
  // afterwards, re-read rather than assumed: the agent is editing while this
  // runs, so the status after a stage is not the status before it plus one.
  gitStatus: (server: string, name: string) =>
    invoke<GitAnswer>("git_status", { server, name }),
  git: (server: string, name: string, action: GitOp) =>
    invoke<GitAnswer>("git", { server, name, action }),
  gitDiff: (server: string, name: string, path: string, against: Against) =>
    invoke<FileDiff>("git_diff", { server, name, path, against }),

  // Shells beside the agent, in the same sandbox under the same policy. What
  // exists is asked of the sandbox rather than remembered here, so a shell
  // survives this window closing.
  shells: (server: string, name: string) => invoke<string[]>("shells", { server, name }),
  newShell: (server: string, name: string) => invoke<string[]>("new_shell", { server, name }),
  killShell: (server: string, name: string, tmux: string) =>
    invoke<string[]>("kill_shell", { server, name, tmux }),

  // The review. Kept on the server, per session, so an unsent one survives the
  // window closing -- see `hura_core::comments`.
  comments: (server: string, name: string) => invoke<Comment[]>("comments", { server, name }),
  comment: (server: string, name: string, comment: NewComment) =>
    invoke<Comment[]>("comment", { server, name, comment }),
  uncomment: (server: string, name: string, id: number) =>
    invoke<Comment[]>("uncomment", { server, name, id }),
  sendComments: (server: string, name: string) =>
    invoke<string>("send_comments", { server, name }),

  // Projects: the repositories someone has decided to work on, which is what
  // the tree groups worktrees under.
  projects: (server: string) => invoke<Project[]>("projects", { server }),
  newProject: (server: string, project: NewProject) =>
    invoke<Project[]>("new_project", { server, project }),
  forgetProject: (server: string, name: string) =>
    invoke<Project[]>("forget_project", { server, name }),

  // The create flow. `repos` and `inspect` answer about the *server's* disk:
  // a checkout is only a way of naming a remote, but which checkouts exist is a
  // fact about the machine that will do the cloning.
  repos: (server: string) => invoke<Listing>("repos", { server }),
  inspect: (server: string, path: string, branch: string | null) =>
    invoke<Picked>("inspect", { server, path, branch }),
  newOptions: (server: string) => invoke<NewOptions>("new_options", { server }),
  create: (server: string, session: NewSession) => invoke<string>("create", { server, session }),

  // What the server holds on your sessions' behalf. Every action answers with
  // the whole view, re-read: a secret is usually what a container was waiting
  // for, so the rest of the screen changes when one is stored.
  integrations: (server: string) => invoke<Integrations>("integrations", { server }),
  mcp: (server: string, name: string, action: McpOp) =>
    invoke<Integrations>("mcp", { server, name, action }),
  // The value goes one way: there is no command that reads one back.
  secret: (server: string, name: string, value: string | null) =>
    invoke<Integrations>("secret", { server, name, value }),
  // Read and packed on the Rust side of the bridge, because `~/.claude/skills`
  // is on *this* machine and a webview cannot see it.
  uploadSkills: (server: string) => invoke<Integrations>("upload_skills", { server }),
  forgetSkill: (server: string, name: string) =>
    invoke<Integrations>("forget_skill", { server, name }),
  mySkills: () => invoke<string[]>("my_skills"),
  // Trackers live on *this* machine, tokens included, and are never sent to a
  // server: the window reads tickets itself. Each change answers with the
  // whole list, which says whether each has a token and never what it is.
  trackers: () => invoke<ConfiguredTracker[]>("trackers"),
  addTracker: (tracker: Tracker, token: string | null) =>
    invoke<ConfiguredTracker[]>("add_tracker", { tracker, token }),
  // Replaced by the name it has now; its token stays. How filters are edited.
  updateTracker: (name: string, tracker: Tracker) =>
    invoke<ConfiguredTracker[]>("update_tracker", { name, tracker }),
  setTrackerToken: (name: string, token: string) =>
    invoke<ConfiguredTracker[]>("set_tracker_token", { name, token }),
  forgetTracker: (name: string) => invoke<ConfiguredTracker[]>("forget_tracker", { name }),

  // The tickets, read from this machine. The server is asked only for its
  // branch prefix, so a ticket suggests the branch its session will get.
  tickets: (server: string | null) => invoke<Inbox>("tickets", { server }),
  // One of them in full, from the same place with the same token.
  ticket: (tracker: string, key: string) => invoke<Issue>("ticket", { tracker, key }),

  // The editable defaults in the server's config file. The server's, because
  // `branch_prefix` names the branch of every session on that machine and a
  // window holding its own copy would be a second convention. What is this
  // window's -- sidebar widths, the refresh interval -- is in `prefs.ts` and
  // never crosses the bridge at all.
  //
  // The write answers with the file re-read, like the integrations screen:
  // a cleared field comes back as an absent key and an absent key reads as the
  // built-in default, so what was saved is not what was sent.
  settings: (server: string) => invoke<SettingsView>("settings", { server }),
  setSettings: (server: string, settings: Settings) =>
    invoke<SettingsView>("set_settings", { server, settings }),
};

/// A rejected command, as the bridge sends it: see `Failed` in main.rs.
///
/// The message is written for a person and is shown rather than interpreted.
/// The kind is the one thing worth branching on.
export type Failed = { kind: FailureKind; message: string };

function isFailed(e: unknown): e is Failed {
  return typeof e === "object" && e !== null && "kind" in e && "message" in e;
}

/// What kind of failure this was, or `null` for anything that did not come
/// from the bridge.
export function kindOf(e: unknown): FailureKind | null {
  return isFailed(e) ? e.kind : null;
}

/// A command's rejection is a string written for a person, so it is shown
/// rather than interpreted. Anything that is not one is a bug in the bridge,
/// and saying so beats rendering "[object Object]".
export function messageOf(e: unknown): string {
  if (isFailed(e)) return e.message;
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return `unexpected failure: ${JSON.stringify(e)}`;
}
