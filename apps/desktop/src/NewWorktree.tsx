// Starting a worktree: what this one is for, as T3 Code starts a thread. A
// draft in the workspace rather than a dialog: the project asked in the
// headline, the task in a composer, and sending it the start. What differs
// between one worktree and the next is a handful of choices with defaults good
// enough to send on sight, in the composer's controls and the strip under it.
//
// Nothing here decides anything `hura new` decides differently. The name is
// derived by the server when this leaves it blank, the policy list and the
// ticked toolchains and credentials arrive from the server, and the skills and
// MCP servers are shown rather than offered, because they are one decision made
// in the config file.

import { useEffect, useRef, useState } from "react";

import { api, messageOf } from "./api";
import { type MenuItem, useButtonMenu } from "./ContextMenu";
import { Waiting } from "./Empty";
import {
  Agent,
  Branch,
  Busy,
  Disclosure,
  Edit,
  Heads,
  Integrations,
  Policy,
  Publish,
  PullRequest as PullMark,
  Queued,
  Recurring,
  Sandbox,
  Scheduled,
  Secret,
  Send,
  Servers as ServerGlyph,
  Skill,
  Start,
  Toolchain,
  Tracker,
  Unsandboxed,
} from "./icons";
import type { Facts } from "./gen/Facts";
import type { Interface } from "./gen/Interface";
import type { NewOptions } from "./gen/NewOptions";
import type { Picked } from "./gen/Picked";
import type { Project } from "./gen/Project";
import type { QueueView } from "./gen/QueueView";
import type { PullOrigin } from "./gen/PullOrigin";
import type { PullRequest } from "./gen/PullRequest";
import type { Task } from "./gen/Task";
import { pullName } from "./inboxNotify";
import type { PullStart } from "./Pull";
import { Pill } from "./Pill";
import { CADENCES, type Cadence, otherClock, repeatOf } from "./repeats";

/// A new worktree, as T3 Code starts a thread: no dialog, but the workspace
/// itself, empty, with what to build asked over a composer, and sending it
/// the start. Where it runs and the rest sit in a strip at the composer's
/// foot; the rarer choices are under More.
export function NewWorktreeDraft({
  server,
  servers,
  onServer,
  projects,
  project,
  onProject,
  from,
  fromPull,
  onClose,
  onCreated,
  onQueued,
  canRepeat,
}: {
  server: string;
  /// Every paired server, by name, and taking the draft to another: T3 Code's
  /// machine picker on the strip, offered when there is more than one.
  servers: string[];
  onServer: (server: string) => void;
  /// Every project, for the headline's picker.
  projects: Project[];
  project: Project;
  onProject: (project: Project) => void;
  /// Whether the server can start one again and again: one whose queue says
  /// its clock (`utc_offset`). See `repeats.ts`.
  canRepeat: boolean;
  // The ticket this is being started from, when it came from the tickets
  // screen. What it fills in (the task, the name, the branch) is the
  // server's answer, derived in `hura_core::tracker`.
  from?: Task | null;
  /// Or the pull request, and whether it is to be worked on or reviewed.
  fromPull?: { pull: PullRequest; mode: PullStart } | null;
  /// Leave the draft: Escape in an empty box.
  onClose: () => void;
  onCreated: (name: string) => void;
  /// Started later instead: the queue as the server now has it.
  onQueued: (queue: QueueView) => void;
}) {
  const [options, setOptions] = useState<NewOptions | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api
      .newOptions(server)
      .then((o) => live && setOptions(o))
      .catch((e) => live && setError(messageOf(e)));
    return () => {
      live = false;
    };
  }, [server]);

  return (
    <div className="draft">
      {/* Keyed by the project, so another one is a fresh draft with its own
          checkout's answers rather than the last one's. */}
      <Form
        key={project.name}
        server={server}
        servers={servers}
        onServer={onServer}
        projects={projects}
        project={project}
        onProject={onProject}
        from={from ?? null}
        fromPull={fromPull ?? null}
        options={options}
        loadError={error}
        onClose={onClose}
        onCreated={onCreated}
        onQueued={onQueued}
        canRepeat={canRepeat}
      />
    </div>
  );
}

/// What kind of worktree.
function Form({
  server,
  servers,
  onServer,
  projects,
  onProject,
  project,
  from,
  fromPull,
  options,
  loadError,
  onClose,
  onCreated,
  onQueued,
  canRepeat,
}: {
  server: string;
  servers: string[];
  onServer: (server: string) => void;
  projects: Project[];
  onProject: (project: Project) => void;
  project: Project;
  canRepeat: boolean;
  from: Task | null;
  fromPull: { pull: PullRequest; mode: PullStart } | null;
  /// Null until the server has answered; the server's choices wait for it.
  options: NewOptions | null;
  loadError: string | null;
  onClose: () => void;
  onCreated: (name: string) => void;
  onQueued: (queue: QueueView) => void;
}) {
  // A ticket fills three fields and leaves the rest alone. The prompt carries
  // the key and the link as well as the title: the agent's first instruction is
  // the whole of what it knows about why it exists, and `PROJ-123` in it is
  // what makes its commits and its pull request say so too.
  const [task, setTask] = useState(
    from
      ? `${from.key}: ${from.title}\n\n${from.url}`
      : fromPull
        ? pullTask(fromPull.pull, fromPull.mode)
        : "",
  );
  // A name as a person writes one. The server makes the slug the sandbox and
  // the branch are keyed by from it, so a ticket's is its key and summary.
  const [name, setName] = useState(from ? `${from.key} ${from.title}` : "");
  // Only sent when it came from a ticket or a pull request. Left empty, the
  // server names the branch by the convention in its own config file, which is
  // the one place that decides it. A pull request's is its own branch, and its
  // base is the branch it merges into, so the diff pane measures the whole of
  // it and not only what the agent adds.
  const [branch] = useState(from?.branch ?? fromPull?.pull.branch ?? "");
  const [base, setBase] = useState(fromPull?.pull.base ?? "");
  // The server's defaults, once it has said them. Its choices are disabled
  // until then, so nothing here can have been changed before they land.
  const [policy, setPolicy] = useState("");
  const [toolchains, setToolchains] = useState<string[]>([]);
  const [providers, setProviders] = useState<string[]>([]);
  const [agentInterface, setAgentInterface] = useState<Interface>("chat");
  // A sandbox unless this server allows otherwise and you say so.
  const [worktree, setWorktree] = useState(false);
  const [facts, setFacts] = useState<Facts | null>(null);
  const [busy, setBusy] = useState(false);
  // Now, once the server's queue has room, or at a time. The time is this
  // machine's, in the form a datetime input gives it, and goes to the server
  // as an instant.
  const [when, setWhen] = useState<When>("now");
  const [at, setAt] = useState(() => localInput(Date.now() + 60 * 60 * 1000));
  // Or again and again: how often, and at what time of day on this machine's
  // clock.
  const [cadence, setCadence] = useState<Cadence>("weekdays");
  const [time, setTime] = useState("09:00");
  const [error, setError] = useState<string | null>(null);
  // The skills on *this* machine, pushed to the server on submit. Read on the
  // Rust side of the bridge: `~/.claude/skills` is here, and a webview cannot
  // see it.
  const [mine, setMine] = useState<string[]>([]);
  // What the checkout said, which outranks the config file's defaults for the
  // base and the credentials -- and may come back before or after them.
  const [picked, setPicked] = useState<Picked | null>(null);

  // The project's own defaults over the server's: see Settings, Projects.
  const own = project.defaults ?? {};

  useEffect(() => {
    if (!options) return;
    setPolicy(own.policy ?? options.default_policy);
    setAgentInterface(own.interface ?? options.default_interface);
    if (own.worktree != null && options.worktrees) setWorktree(own.worktree);
    // The project's are read once, with the server's: it is keyed by the
    // project, so another one is another form.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [options]);

  // The base and the credentials, from whichever of the two answers says the
  // most. The checkout's branch beats the config's default, which is only for
  // a detached HEAD; a branch never pushed is blanked, since a clone of it
  // would fail; a pull request's base is not the checkout's to decide. The
  // checkout's credentials replace the config's defaults when it names any.
  useEffect(() => {
    if (!options) return;
    if (!fromPull) {
      if (own.base) setBase(own.base);
      else if (picked && !picked.facts.base_on_remote) setBase("");
      else setBase(picked?.branch || options.default_base || "");
    }
    setProviders(
      own.providers ?? (picked && picked.providers.length > 0 ? picked.providers : options.default_providers),
    );
  }, [options, picked, fromPull]);

  useEffect(() => {
    let live = true;
    api.mySkills().then((names) => live && setMine(names));
    return () => {
      live = false;
    };
  }, []);

  // The repository has already answered two of these questions, so the form
  // arrives with them answered rather than asking. It costs subprocesses and a
  // runtime call on the server, which is why it happens once, here, and not for
  // every row of the picker.
  useEffect(() => {
    let live = true;
    api
      // The project stores a path; which branch that checkout is on is a fact
      // only the server can read, so it comes back with the rest.
      .inspect(server, project.path, fromPull?.pull.base ?? null)
      .then((picked: Picked) => {
        if (!live) return;
        setFacts(picked.facts);
        setToolchains(own.toolchains ?? picked.facts.toolchains);
        // The base and the credentials are settled above, with the options.
        setPicked(picked);
      })
      .catch((e) => live && setError(messageOf(e)));
    return () => {
      live = false;
    };
  }, [server, project.path, fromPull]);

  const toggle = (list: string[], set: (v: string[]) => void, value: string) =>
    set(list.includes(value) ? list.filter((v) => v !== value) : [...list, value]);


  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      // **The skills go up before the session is asked for**, and that is what
      // keeps the pointer-not-copy property across two machines: the server's
      // library is a copy of a directory on this one, and re-pushing it here
      // means editing a skill on your laptop still reaches the next session.
      // Only when there are any -- the command refuses an empty upload rather
      // than pretending it did something.
      //
      // A failure stops the create rather than being swallowed, because the
      // server already tolerates the per-skill kind: it only fails this when
      // *nothing* landed, which means the library is unwritable or the server
      // is unreachable, and a session created on stale skills would be a quiet
      // wrong answer.
      if (mine.length > 0) await api.uploadSkills(server);

      const session = {
        project: project.name,
        branch: branch || null,
        // A note on the session of where the work came from.
        ticket: from ? ticketOf(from) : null,
        // The same for a pull request, and what has the server check out its
        // branch rather than make one.
        pull: fromPull ? pullOrigin(fromPull.pull) : null,
        // Blank means "derive it", which is what `hura new` without `--name`
        // does -- by a model, when the server is set up for one. The rule
        // lives on the server so there is one of it. Words rather than a slug
        // are the title, with a slug made from them.
        name: name.trim() || null,
        title: null,
        repo: project.repo,
        task,
        base: base.trim() || null,
        policy,
        providers,
        toolchains,
        start: true,
        interface: agentInterface,
        worktree,
      };
      if (when === "now") {
        onCreated(await api.create(server, session));
      } else if (when === "repeat") {
        if (cadence !== "hourly" && !/^\d{2}:\d{2}$/.test(time)) throw new Error("Pick a time of day for it");
        onQueued(await api.enqueue(server, session, null, repeatOf(cadence, time)));
      } else {
        const instant = when === "at" ? epochOf(at) : null;
        if (when === "at" && instant === null) throw new Error("Pick a time to start it at");
        onQueued(await api.enqueue(server, session, instant));
      }
    } catch (e) {
      setError(messageOf(e));
      setBusy(false);
    }
  };

  // Skills this machine pushes, beside the ones the server already has. One
  // list, because to the agent they are one list.
  const skills = [...new Set([...(options?.skills ?? []), ...mine])];

  const [more, setMore] = useState(false);
  const box = useRef<HTMLTextAreaElement>(null);
  const ready = !busy && options !== null;
  const WhenIcon = WHEN_ICONS[when];
  const whens: { value: When; label: string; hint: string }[] = [
    { value: "now", label: "Now", hint: "As soon as it is made" },
    { value: "queue", label: "When one is free", hint: "Once fewer are running than the server's limit" },
    { value: "at", label: "At a time", hint: "Kept by the server, through a restart" },
    ...(canRepeat
      ? [{ value: "repeat" as When, label: "On a schedule", hint: "A new worktree each time it comes round" }]
      : []),
  ];
  const sendTitle =
    when === "now"
      ? "Start It (Enter)"
      : when === "queue"
        ? "Queue It (Enter)"
        : when === "at"
          ? "Schedule It (Enter)"
          : "Schedule It to Repeat (Enter)";

  return (
    <div className="draft-stage">
      <h1 className="draft-headline">
        <span>What should we build in</span>
        <ProjectPicker project={project} projects={projects} onProject={onProject} locked={fromPull !== null} />
        <span className="draft-q">?</span>
      </h1>

      {fromPull && (
        <p className="draft-origin">
          <PullMark />
          <a href={fromPull.pull.url} target="_blank" rel="noreferrer">
            {pullName(fromPull.pull)}
          </a>
          <span>on</span>
          <code>{fromPull.pull.branch}</code>
          <span>
            {fromPull.mode === "review"
              ? "to review it: the agent is asked not to push"
              : "to work on it: its pushes update the pull request"}
          </span>
        </p>
      )}
      {from && (
        <p className="draft-origin">
          <Tracker />
          <a href={from.url} target="_blank" rel="noreferrer">
            {from.key}
          </a>
          <span>on</span>
          <code>{from.branch}</code>
        </p>
      )}

      <div className="chat-composer draft-composer">
        <div className="chat-box" onMouseDown={(e) => e.target === e.currentTarget && box.current?.focus()}>
          <div className="chat-field">
            <textarea
              ref={box}
              autoFocus
              rows={2}
              value={task}
              // Said, because nothing else here does: this is the agent's
              // first prompt, sent the moment the clone finishes.
              placeholder={`Describe the task. It starts on it once the ${worktree ? "worktree" : "sandbox"} is ready; sent empty, the agent waits.`}
              onChange={(e) => setTask(e.target.value)}
              onKeyDown={(e) => {
                if (e.nativeEvent.isComposing) return;
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  if (ready) void submit();
                } else if (e.key === "Escape" && !task.trim()) {
                  e.preventDefault();
                  onClose();
                }
              }}
            />
          </div>
          <div className="chat-controls">
            <MenuControl
              icon={Agent}
              label={agentInterface === "terminal" ? "Terminal" : "Chat"}
              title="How you talk to the agent"
              disabled={!options}
              items={() =>
                [
                  { value: "chat", label: "Chat", hint: "Drawn by this window" },
                  { value: "terminal", label: "Terminal", hint: "Claude Code's own, in tmux" },
                ].map((o) => ({
                  label: o.label,
                  hint: o.hint,
                  checked: o.value === agentInterface,
                  run: () => setAgentInterface(o.value as Interface),
                }))
              }
            />
            <MenuControl
              icon={WhenIcon}
              label={whens.find((w) => w.value === when)?.label ?? "Now"}
              title="When it starts"
              items={() =>
                whens.map((w) => ({ label: w.label, hint: w.hint, checked: w.value === when, run: () => setWhen(w.value) }))
              }
            />
            {when === "at" && (
              <input
                className="draft-time"
                type="datetime-local"
                title="Starts at, on this machine's clock"
                value={at}
                onChange={(e) => setAt(e.target.value)}
              />
            )}
            {when === "repeat" && (
              <>
                <MenuControl
                  icon={Recurring}
                  label={CADENCES.find((c) => c.value === cadence)?.label ?? ""}
                  title="How often a new one starts"
                  items={() =>
                    CADENCES.map((c) => ({ label: c.label, checked: c.value === cadence, run: () => setCadence(c.value) }))
                  }
                />
                {cadence !== "hourly" && (
                  <input
                    className="draft-time"
                    type="time"
                    title={
                      otherClock()
                        ? "On this machine's clock; the server keeps another, and is told the same moment on its own"
                        : "At this time of day"
                    }
                    value={time}
                    onChange={(e) => setTime(e.target.value)}
                  />
                )}
              </>
            )}
            <span className="flex-1" />
            <button
              className="chat-round chat-send"
              title={sendTitle}
              aria-label={sendTitle}
              disabled={!ready}
              onClick={() => void submit()}
            >
              {busy ? <Busy className="turning" size={14} /> : when === "now" ? <Send size={14} strokeWidth={1.8} /> : <WhenIcon size={14} />}
            </button>
          </div>
          {/* T3 Code's strip at the composer's foot: where the work happens. */}
          <div className="draft-strip">
            {servers.length > 1 && (
              <>
                <MenuControl
                  small
                  icon={ServerGlyph}
                  label={server}
                  title="The server it starts on"
                  items={() => servers.map((s) => ({ label: s, checked: s === server, run: () => onServer(s) }))}
                />
                <span className="draft-sep" aria-hidden />
              </>
            )}
            <MenuControl
              small
              icon={worktree ? Unsandboxed : Sandbox}
              label={worktree ? "Worktree" : "Sandbox"}
              title={
                options && !options.worktrees
                  ? "Where the agent runs: this server's config allows only sandboxes"
                  : "Where the agent runs"
              }
              disabled={!options?.worktrees}
              items={() => [
                { label: "Sandbox", hint: "Its own VM, under a policy", checked: !worktree, run: () => setWorktree(false) },
                { label: "Worktree", hint: "On the server, as you", checked: worktree, run: () => setWorktree(true) },
              ]}
            />
            <span className="draft-sep" aria-hidden />
            <label
              className="draft-field"
              title={fromPull ? "The branch the pull request merges into" : "Base branch (the remote's default when blank)"}
            >
              <Branch />
              <input
                value={base}
                // As wide as what it says, as T3 Code's branch picker is.
                style={{ width: `${Math.max(base.length, base ? 1 : "Default branch".length) + 1}ch` }}
                placeholder="Default branch"
                spellCheck={false}
                readOnly={fromPull !== null}
                onChange={(e) => setBase(e.target.value)}
              />
            </label>
            <span className="draft-sep" aria-hidden />
            <MenuControl
              small
              icon={Policy}
              label={worktree ? "No policy" : policy || "Policy"}
              title={worktree ? "No network policy: a worktree runs on the server as you" : "Network policy"}
              disabled={!options || worktree}
              items={() =>
                (options?.policies ?? []).map((p) => ({
                  label: p.spec,
                  hint: p.summary,
                  checked: p.spec === policy,
                  run: () => setPolicy(p.spec),
                }))
              }
            />
            <span className="draft-sep" aria-hidden />
            <label className="draft-field" title="Name (made from the task when blank)">
              <Edit />
              <input
                value={name}
                style={{ width: `${Math.max(name.length, name ? 1 : "Name from the task".length) + 1}ch` }}
                placeholder="Name from the task"
                spellCheck={false}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            <span className="flex-1" />
            <button
              className="draft-chip"
              aria-expanded={more}
              title="Toolchains, credentials, skills and MCP servers"
              onClick={() => setMore(!more)}
            >
              More
              <Disclosure className={more ? "rotate-180" : undefined} />
            </button>
          </div>
        </div>
        <p className="draft-status">
          {(error ?? loadError) ? (
            <span className="error" title={error ?? loadError ?? undefined}>
              {error ?? loadError}
            </span>
          ) : (
            facts && <Drift facts={facts} />
          )}
        </p>
      </div>

      {/* Said where the choice is made, because nothing afterwards stops it:
          the agent is a process of the server user's, with their files, their
          credentials and the whole network. */}
      {options && worktree && (
        <div className="draft-note">
          <p className="unsandboxed-note">
            <Unsandboxed />
            A git worktree of {project.path} on the server. The agent runs there as the server's user, with its
            files, credentials and network, and no policy. Its skills and MCP servers are that user's own.
          </p>
        </div>
      )}

      {more && options && !worktree && (
        <div className="draft-more pick-rows">
          <PickRow icon={Toolchain} label="Toolchains">
            {options.toolchains.map((t) => (
              <Pill
                key={t.name}
                on={toolchains.includes(t.name)}
                title={t.summary}
                onClick={() => toggle(toolchains, setToolchains, t.name)}
              >
                {t.name}
              </Pill>
            ))}
          </PickRow>

          <PickRow icon={Secret} label="Credentials">
            {options.providers.map((p) => (
              <Pill
                key={p.name}
                on={providers.includes(p.name)}
                title={p.kind}
                onClick={() => toggle(providers, setProviders, p.name)}
              >
                {p.name}
              </Pill>
            ))}
            {options.providers_error && (
              <span className="error" title={options.providers_error}>
                Could not list credentials
              </span>
            )}
            {options.providers.length === 0 && !options.providers_error && <span className="hint">None configured</span>}
          </PickRow>

          {/* Named, not offered: skills and MCP servers are one decision about
              what your agents can reach, made in the server's config file. */}
          <PickRow icon={Skill} label="Skills, set in the server's config">
            {skills.length === 0 && <span className="hint">none</span>}
            {skills.map((s) => (
              <span key={s} className="pill held" title={mine.includes(s) ? "Pushed from this machine first" : undefined}>
                {s}
                {mine.includes(s) && <Publish className="local" />}
              </span>
            ))}
          </PickRow>

          {/* The cost of an MCP server, said where a worktree is about to be
              given one: everything that server can do is now something this
              agent can do with the server's credentials. */}
          <PickRow icon={Integrations} label="MCP servers, which act with the server's credentials">
            {options.mcp.length === 0 && <span className="hint">none</span>}
            {options.mcp.map((m) => (
              <span key={m} className="pill held">
                {m}
              </span>
            ))}
          </PickRow>
        </div>
      )}
      {more && !options && <Waiting />}
    </div>
  );
}

/// The project in the headline, and the menu of the others: T3 Code's.
function ProjectPicker({
  project,
  projects,
  onProject,
  locked,
}: {
  project: Project;
  projects: Project[];
  onProject: (project: Project) => void;
  /// A pull request's draft is its repository's.
  locked: boolean;
}) {
  const menu = useButtonMenu();
  const choosable = !locked && projects.length > 1;
  return (
    <button
      className="draft-project"
      disabled={!choosable}
      title={locked ? "The pull request's repository" : choosable ? "Another Project" : undefined}
      aria-haspopup={choosable ? "menu" : undefined}
      onClick={menu(() =>
        projects.map((p) => ({ label: p.name, checked: p.name === project.name, run: () => onProject(p) })),
      )}
    >
      {project.name}
      {choosable && <Disclosure aria-hidden />}
    </button>
  );
}

/// A choice as the composer's controls draw one: its icon and what is
/// chosen, opening a menu of the rest. `small` is the strip's size.
function MenuControl({
  icon: Icon,
  label,
  title,
  disabled,
  items,
  small = false,
}: {
  icon: React.ComponentType<{ size?: number; className?: string }>;
  label: string;
  title: string;
  disabled?: boolean;
  items: () => MenuItem[];
  small?: boolean;
}) {
  const menu = useButtonMenu();
  return (
    <button
      className={small ? "draft-chip" : "chat-control"}
      title={title}
      aria-haspopup="menu"
      disabled={disabled}
      onClick={menu(items)}
    >
      <Icon size={small ? 13 : 15} />
      <span>{label}</span>
    </button>
  );
}

type When = "now" | "queue" | "at" | "repeat";

/// The mark for when it starts, on the chooser and on the button.
const WHEN_ICONS: Record<When, React.ComponentType<{ size?: number; className?: string }>> = {
  now: Start,
  queue: Queued,
  at: Scheduled,
  repeat: Recurring,
};

/// A time as a `datetime-local` input holds it: this machine's clock, to the
/// minute.
export function localInput(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/// What a `datetime-local` input says, as seconds since the epoch; null for a
/// blank or half-typed one.
export function epochOf(input: string): number | null {
  const ms = new Date(input).getTime();
  return input && Number.isFinite(ms) ? Math.floor(ms / 1000) : null;
}

/// One line of choices: the icon says what they are, the tooltip says it in
/// words.
function PickRow({
  icon: Icon,
  label,
  children,
}: {
  icon: React.ComponentType;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="pick-row">
      <span className="pick-mark" title={label} aria-label={label}>
        <Icon />
      </span>
      <div className="pills">{children}</div>
    </div>
  );
}

/// The agent's first instruction for a session started from a pull request.
///
/// Both say which branch this is and what it merges into, because the agent
/// has no other way to know it is on somebody's pull request rather than a
/// branch of its own. Work leaves the instruction to you, under the link.
/// Review is the instruction: read it, run what covers it, report, and touch
/// nothing that reaches the pull request, since what is said there is yours to
/// decide.
function pullTask(pull: PullRequest, mode: PullStart): string {
  const where =
    `This worktree is on the pull request's own branch, \`${pull.branch}\`, ` +
    `which merges into \`${pull.base}\`.`;
  if (mode === "review") {
    return (
      `Review ${pullName(pull)}: ${pull.title}\n${pull.url}\n\n${where} ` +
      `Read what it changes with \`git diff origin/${pull.base}...HEAD\` and its commits with ` +
      `\`git log origin/${pull.base}..HEAD\`, run the tests that cover it, and report what you ` +
      `find: what is wrong first, then what is risky, then anything smaller, each with its file ` +
      `and line. Do not commit, push, or comment on the pull request.`
    );
  }
  return `${pullName(pull)}: ${pull.title}\n${pull.url}\n\n${where} Pushing updates the pull request.\n\n`;
}

/// The pull request, as the session records it.
function pullOrigin(pull: PullRequest): PullOrigin {
  return {
    tracker: pull.tracker,
    kind: pull.kind,
    id: pull.id,
    key: pull.key,
    url: pull.url,
    repo: pull.repo,
  };
}

/// The ticket, as the session records it.
///
/// Hand-built from the task rather than sent whole: a `Task` carries the row's
/// presentation -- its status, its suggested name -- and a session needs only
/// what a write-back is addressed with.
function ticketOf(task: Task) {
  return {
    tracker: task.tracker,
    kind: task.kind,
    id: task.id,
    key: task.key,
    url: task.url,
    repo: task.repo,
  };
}

/// What stays behind on the server's checkout.
///
/// The sandbox clones `origin`, so uncommitted work and unpushed commits are
/// not coming with it. Worth saying before the session starts rather than after
/// the agent has failed to find them.
function Drift({ facts }: { facts: Facts }) {
  const bits: string[] = [];
  if (facts.uncommitted > 0) bits.push(`${facts.uncommitted} uncommitted`);
  if (facts.unpushed) bits.push(`${facts.unpushed} unpushed`);
  if (!facts.base_on_remote) bits.push("branch not on remote");
  if (bits.length === 0) return null;
  return (
    // The line may be cut short in the footer, so the tooltip says it whole.
    <span className="drift" title={`${bits.join(", ")}: the sandbox clones the remote, so this stays behind on the server's checkout`}>
      <Heads />
      <span>{bits.join(" · ")}</span>
    </span>
  );
}
