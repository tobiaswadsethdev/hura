// Starting a worktree: what this one is for.
//
// The second of the two questions the create flow used to ask on one screen.
// The repository is already answered -- it is the project this is being started
// in -- so what is left is the part that differs between one worktree and the
// next, which is a handful of fields with defaults good enough to submit on
// sight.
//
// Nothing here decides anything `hura new` decides differently. The name is
// derived by the server when this leaves it blank, the policy list and the
// ticked toolchains and credentials arrive from the server, and the skills and
// MCP servers are shown rather than offered, because they are one decision made
// in the config file.

import { useEffect, useState } from "react";

import { api, messageOf } from "./api";
import { Waiting } from "./Empty";
import {
  Agent,
  Branch,
  Busy,
  Close,
  Edit,
  Heads,
  Integrations,
  Policy,
  Publish,
  PullRequest as PullMark,
  Queued,
  Sandbox,
  Scheduled,
  Secret,
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
import { Select } from "./Select";

export function NewWorktreeDialog({
  server,
  project,
  from,
  fromPull,
  onClose,
  onCreated,
  onQueued,
}: {
  server: string;
  project: Project;
  // The ticket this is being started from, when it came from the tickets screen. What it
  // fills in -- the task, the name, the branch -- is the server's answer,
  // derived in `hura_core::tracker`, so the terminal and the window would agree
  // if the terminal had a tickets screen.
  from?: Task | null;
  /// Or the pull request, and whether it is to be worked on or reviewed.
  fromPull?: { pull: PullRequest; mode: PullStart } | null;
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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="scrim" onMouseDown={onClose}>
      <div className="dialog new" onMouseDown={(e) => e.stopPropagation()}>
        {/* The form from the first frame, with the server's half filled in when
            it comes: a spinner in its place was a dialog that changed size the
            moment the options arrived. */}
        <Form
          server={server}
          project={project}
          from={from ?? null}
          fromPull={fromPull ?? null}
          options={options}
          loadError={error}
          onClose={onClose}
          onCreated={onCreated}
          onQueued={onQueued}
        />
      </div>
    </div>
  );
}

/// What kind of worktree.
function Form({
  server,
  project,
  from,
  fromPull,
  options,
  loadError,
  onClose,
  onCreated,
  onQueued,
}: {
  server: string;
  project: Project;
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
  const [error, setError] = useState<string | null>(null);
  // The skills on *this* machine, pushed to the server on submit. Read on the
  // Rust side of the bridge: `~/.claude/skills` is here, and a webview cannot
  // see it.
  const [mine, setMine] = useState<string[]>([]);
  // What the checkout said, which outranks the config file's defaults for the
  // base and the credentials -- and may come back before or after them.
  const [picked, setPicked] = useState<Picked | null>(null);

  useEffect(() => {
    if (!options) return;
    setPolicy(options.default_policy);
    setAgentInterface(options.default_interface);
  }, [options]);

  // The base and the credentials, from whichever of the two answers says the
  // most. The checkout's branch beats the config's default, which is only for
  // a detached HEAD; a branch never pushed is blanked, since a clone of it
  // would fail; a pull request's base is not the checkout's to decide. The
  // checkout's credentials replace the config's defaults when it names any.
  useEffect(() => {
    if (!options) return;
    if (!fromPull) {
      if (picked && !picked.facts.base_on_remote) setBase("");
      else setBase(picked?.branch || options.default_base || "");
    }
    setProviders(picked && picked.providers.length > 0 ? picked.providers : options.default_providers);
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
        setToolchains(picked.facts.toolchains);
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

  return (
    <div
      className="new-session"
      onKeyDown={(e) => {
        // The textarea owns Enter, so the shortcut has to be a chord.
        if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !busy && options) {
          e.preventDefault();
          void submit();
        }
      }}
    >
      <header className="dialog-head">
        <h2>
          <Start className="head-mark" />
          New Session <span className="in">in</span> {project.name}
        </h2>
        <button className="quiet-icon" title="Close" onClick={onClose}>
          <Close aria-label="Close" />
        </button>
      </header>
      <p className="origin" title="Cloned from">
        {project.repo}
      </p>

      {fromPull && (
        <p className="from-ticket">
          <PullMark />
          <a href={fromPull.pull.url} target="_blank" rel="noreferrer">
            {pullName(fromPull.pull)}
          </a>
          <span className="hint">on</span>
          <code>{fromPull.pull.branch}</code>
          <span className="hint">
            {fromPull.mode === "review"
              ? "to review it: the agent is asked not to push"
              : "to work on it: its pushes update the pull request"}
          </span>
        </p>
      )}

      {from && (
        <p className="from-ticket">
          <Tracker />
          <a href={from.url} target="_blank" rel="noreferrer">
            {from.key}
          </a>
          <span className="hint">→</span>
          <code>{from.branch}</code>
        </p>
      )}

      <textarea
        className="task"
        autoFocus
        rows={4}
        value={task}
        // Said, because nothing else on the form does: this is the agent's first
        // prompt, sent the moment the clone finishes, and blank starts it idle.
        placeholder={`What should the agent do? It starts on this as soon as the ${worktree ? "worktree" : "sandbox"} is ready. Leave blank to start it idle.`}
        onChange={(e) => setTask(e.target.value)}
      />

      <div className="chips">
        <label className="chip-field" title="Name (made from the task when blank)">
          <Edit />
          <input
            value={name}
            placeholder="auto name"
            spellCheck={false}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label
          className="chip-field"
          title={
            fromPull
              ? "The branch the pull request merges into"
              : "Base branch (the remote's default when blank)"
          }
        >
          <Branch />
          <input
            value={base}
            placeholder="default branch"
            spellCheck={false}
            readOnly={fromPull !== null}
            onChange={(e) => setBase(e.target.value)}
          />
        </label>
        <label className="chip-field" title="When it starts">
          {when === "now" ? <Start /> : when === "queue" ? <Queued /> : <Scheduled />}
          <Select
            value={when}
            onChange={(v) => setWhen(v as When)}
            aria-label="When"
            options={[
              { value: "now", label: "Now" },
              { value: "queue", label: "When one is free", hint: "Once fewer are running than the server's limit" },
              { value: "at", label: "At a time", hint: "Kept by the server, through a restart" },
            ]}
          />
        </label>
        <label className="chip-field" title="How you talk to the agent">
          <Agent />
          <Select
            value={agentInterface}
            onChange={(v) => setAgentInterface(v as Interface)}
            aria-label="Agent"
            disabled={!options}
            options={[
              { value: "chat", label: "Chat", hint: "Drawn by this window" },
              { value: "terminal", label: "Terminal", hint: "Claude Code's own, in tmux" },
            ]}
          />
        </label>
        {/* Always the six chips, so the form is one shape whatever is chosen
            and whatever the server allows. A server that does not offer
            worktrees shows the sandbox as the only answer, and a worktree
            shows that it has no policy, rather than either chip going. */}
        <label
          className="chip-field"
          title={options && !options.worktrees ? "Where the agent runs: this server's config allows only sandboxes" : "Where the agent runs"}
        >
          {worktree ? <Unsandboxed /> : <Sandbox />}
          <Select
            value={worktree ? "worktree" : "sandbox"}
            onChange={(v) => setWorktree(v === "worktree")}
            aria-label="Where"
            disabled={!options?.worktrees}
            options={[
              { value: "sandbox", label: "Sandbox", hint: "Its own VM, under a policy" },
              { value: "worktree", label: "Worktree", hint: "On the server, as you" },
            ]}
          />
        </label>
        <label
          className="chip-field"
          title={worktree ? "No network policy: a worktree runs on the server as you" : "Network policy"}
        >
          <Policy />
          <Select
            value={policy}
            onChange={setPolicy}
            aria-label="Policy"
            disabled={!options || worktree}
            shown={worktree ? "No policy" : options ? undefined : "Policy"}
            options={(options?.policies ?? []).map((p) => ({ value: p.spec, label: p.spec, hint: p.summary }))}
          />
        </label>
      </div>

      {/* One fixed box for whatever goes under the chips -- the pills, the
          worktree's warning, the wait for the server -- so none of them moves
          the footer. Rows past its height scroll inside it. */}
      <div className="new-details">
      {!options && <Waiting />}

      {/* Said where the choice is made, because nothing afterwards stops it:
          the agent is a process of the server user's, with their files, their
          credentials and the whole network. */}
      {options && worktree && (
        <p className="unsandboxed-note">
          <Unsandboxed />
          A git worktree of {project.path} on the server. The agent runs there as the server's
          user, with its files, credentials and network, and no policy. Its skills and MCP servers
          are that user's own.
        </p>
      )}

      {options && !worktree && (
      <div className="pick-rows">
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
          {options.providers.length === 0 && !options.providers_error && (
            <span className="hint">None configured</span>
          )}
        </PickRow>

        {/* Named, not offered: skills and MCP servers are one decision about
            what your agents can reach, made in the server's config file.
            Shown so a session's tools are not a surprise. */}
        <PickRow icon={Skill} label="Skills, set in the server's config">
          {skills.length === 0 && <span className="hint">none</span>}
          {skills.map((s) => (
            <span
              key={s}
              className="pill fixed"
              title={mine.includes(s) ? "Pushed from this machine first" : undefined}
            >
              {s}
              {mine.includes(s) && <Publish className="local" />}
            </span>
          ))}
        </PickRow>

        {/* The cost of an MCP server, said where a session is about to be
            given one: everything that server can do is now something this
            agent can do with the server's credentials. */}
        <PickRow
          icon={Integrations}
          label="MCP servers, which act with the server's credentials"
        >
          {options.mcp.length === 0 && <span className="hint">none</span>}
          {options.mcp.map((m) => (
            <span key={m} className="pill fixed">
              {m}
            </span>
          ))}
        </PickRow>
      </div>
      )}
      </div>

      <footer className="new-session-foot">
        {/* A line, in the footer's spare room: an error inserted above it
            pushed the footer down, so it says itself here, cut short if need
            be and whole in the tooltip. What it replaces is only a warning. */}
        <span className="foot-status">
          {(error ?? loadError) ? (
            <span className="error" title={error ?? loadError ?? undefined}>
              {error ?? loadError}
            </span>
          ) : (
            facts && <Drift facts={facts} />
          )}
        </span>
        {/* Beside the button it is the time for, in room the footer already
            has. As a chip among the others it added a row to the form, and
            moved everything under it, every time the choice changed. */}
        {when === "at" && (
          <label className="chip-field start-at" title="Starts at, on this machine's clock">
            <Scheduled />
            <input type="datetime-local" value={at} onChange={(e) => setAt(e.target.value)} />
          </label>
        )}
        <span className="keys" title="Start with the keyboard">
          ctrl ↵
        </span>
        <button className="go" disabled={busy || !options} onClick={() => void submit()}>
          {busy ? <Busy className="turning" /> : when === "now" ? <Start /> : when === "queue" ? <Queued /> : <Scheduled />}
          {busy ? (when === "now" ? "Starting…" : "Adding…") : when === "now" ? "Start" : when === "queue" ? "Queue" : "Schedule"}
        </button>
      </footer>
    </div>
  );
}

type When = "now" | "queue" | "at";

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
