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
  Secret,
  Skill,
  Start,
  Toolchain,
  Tracker,
} from "./icons";
import type { Facts } from "./gen/Facts";
import type { Interface } from "./gen/Interface";
import type { NewOptions } from "./gen/NewOptions";
import type { Picked } from "./gen/Picked";
import type { Project } from "./gen/Project";
import type { Task } from "./gen/Task";
import { Pill } from "./Pill";
import { Select } from "./Select";

export function NewWorktreeDialog({
  server,
  project,
  from,
  onClose,
  onCreated,
}: {
  server: string;
  project: Project;
  // The ticket this is being started from, when it came from the tickets screen. What it
  // fills in -- the task, the name, the branch -- is the server's answer,
  // derived in `hura_core::tracker`, so the terminal and the window would agree
  // if the terminal had a tickets screen.
  from?: Task | null;
  onClose: () => void;
  onCreated: (name: string) => void;
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
        {error && <p className="error">{error}</p>}
        {!options && !error && <Waiting />}
        {options && (
          <Form
            server={server}
            project={project}
            from={from ?? null}
            options={options}
            onClose={onClose}
            onCreated={onCreated}
          />
        )}
      </div>
    </div>
  );
}

/// What kind of worktree.
function Form({
  server,
  project,
  from,
  options,
  onClose,
  onCreated,
}: {
  server: string;
  project: Project;
  from: Task | null;
  options: NewOptions;
  onClose: () => void;
  onCreated: (name: string) => void;
}) {
  // A ticket fills three fields and leaves the rest alone. The prompt carries
  // the key and the link as well as the title: the agent's first instruction is
  // the whole of what it knows about why it exists, and `PROJ-123` in it is
  // what makes its commits and its pull request say so too.
  const [task, setTask] = useState(from ? `${from.key}: ${from.title}\n\n${from.url}` : "");
  const [name, setName] = useState(from?.session_name ?? "");
  // Only sent when it came from a ticket. Left empty, the server names the
  // branch by the convention in its own config file -- which is the one place
  // that decides it.
  const [branch] = useState(from?.branch ?? "");
  const [base, setBase] = useState(options.default_base ?? "");
  const [policy, setPolicy] = useState(options.default_policy);
  const [toolchains, setToolchains] = useState<string[]>([]);
  const [providers, setProviders] = useState<string[]>(options.default_providers);
  const [agentInterface, setAgentInterface] = useState<Interface>(options.default_interface);
  const [facts, setFacts] = useState<Facts | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The skills on *this* machine, pushed to the server on submit. Read on the
  // Rust side of the bridge: `~/.claude/skills` is here, and a webview cannot
  // see it.
  const [mine, setMine] = useState<string[]>([]);

  useEffect(() => {
    let live = true;
    api.mySkills().then((names) => live && setMine(names));
    return () => {
      live = false;
    };
  }, []);

  // The repository has already answered two of these questions, so the form
  // arrives with them answered rather than asking. It costs subprocesses and a
  // gateway call on the server, which is why it happens once, here, and not for
  // every row of the picker.
  useEffect(() => {
    let live = true;
    api
      // The project stores a path; which branch that checkout is on is a fact
      // only the server can read, so it comes back with the rest.
      .inspect(server, project.path, null)
      .then((picked: Picked) => {
        if (!live) return;
        setFacts(picked.facts);
        setToolchains(picked.facts.toolchains);
        if (picked.branch) setBase(picked.branch);
        // Empty when the config file names providers, since an explicit list
        // replaces the rule rather than adding to it -- so the defaults already
        // in state stand.
        if (picked.providers.length > 0) setProviders(picked.providers);
        // A branch that has never been pushed cannot be cloned from, so the
        // remote's default is used instead of handing the gateway a clone that
        // is going to fail.
        if (!picked.facts.base_on_remote) setBase("");
      })
      .catch((e) => live && setError(messageOf(e)));
    return () => {
      live = false;
    };
  }, [server, project.path]);

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

      const created = await api.create(server, {
        project: project.name,
        branch: branch || null,
        // A note on the session of where the work came from.
        ticket: from ? ticketOf(from) : null,
        // Blank means "derive it", which is what `hura new` without `--name`
        // does. The rule lives on the server so there is one of it.
        name: name.trim() || null,
        repo: project.repo,
        task,
        base: base.trim() || null,
        policy,
        providers,
        toolchains,
        start: true,
        interface: agentInterface,
      });
      onCreated(created);
    } catch (e) {
      setError(messageOf(e));
      setBusy(false);
    }
  };

  // Skills this machine pushes, beside the ones the server already has. One
  // list, because to the agent they are one list.
  const skills = [...new Set([...options.skills, ...mine])];

  return (
    <div
      className="new-session"
      onKeyDown={(e) => {
        // The textarea owns Enter, so the shortcut has to be a chord.
        if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !busy) {
          e.preventDefault();
          void submit();
        }
      }}
    >
      <header className="dialog-head">
        <h2>
          <Start className="head-mark" />
          new session <span className="in">in</span> {project.name}
        </h2>
        <button className="quiet-icon" title="close" onClick={onClose}>
          <Close aria-label="close" />
        </button>
      </header>
      <p className="origin" title="cloned from">
        {project.repo}
      </p>

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
        placeholder="What should the agent do? It starts on this as soon as the sandbox is ready. Leave blank to start it idle."
        onChange={(e) => setTask(e.target.value)}
      />

      <div className="chips">
        <label className="chip-field" title="session name (derived from the task when blank)">
          <Edit />
          <input
            value={name}
            placeholder="auto name"
            spellCheck={false}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label className="chip-field" title="base branch (the remote's default when blank)">
          <Branch />
          <input
            value={base}
            placeholder="default branch"
            spellCheck={false}
            onChange={(e) => setBase(e.target.value)}
          />
        </label>
        <label className="chip-field" title="how you talk to the agent">
          <Agent />
          <Select
            value={agentInterface}
            onChange={(v) => setAgentInterface(v as Interface)}
            aria-label="agent"
            options={[
              { value: "chat", label: "chat", hint: "drawn by this window" },
              { value: "terminal", label: "terminal", hint: "Claude Code's own, in tmux" },
            ]}
          />
        </label>
        <label className="chip-field" title="network policy">
          <Policy />
          <Select
            value={policy}
            onChange={setPolicy}
            aria-label="policy"
            options={options.policies.map((p) => ({ value: p.spec, label: p.spec, hint: p.summary }))}
          />
        </label>
      </div>

      <div className="pick-rows">
        <PickRow icon={Toolchain} label="toolchains">
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

        <PickRow icon={Secret} label="credentials">
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
              gateway unreachable
            </span>
          )}
          {options.providers.length === 0 && !options.providers_error && (
            <span className="hint">none on the gateway</span>
          )}
        </PickRow>

        {/* Named, not offered: skills and MCP servers are one decision about
            what your agents can reach, made in the server's config file.
            Shown so a session's tools are not a surprise. */}
        <PickRow icon={Skill} label="skills, set in the server's config">
          {skills.length === 0 && <span className="hint">none</span>}
          {skills.map((s) => (
            <span
              key={s}
              className="pill fixed"
              title={mine.includes(s) ? "pushed from this machine first" : undefined}
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

      {error && <p className="error">{error}</p>}

      <footer className="new-session-foot">
        {facts && <Drift facts={facts} />}
        <span className="keys" title="start with the keyboard">
          ctrl ↵
        </span>
        <button className="go" disabled={busy} onClick={() => void submit()}>
          {busy ? <Busy className="turning" /> : <Start />}
          {busy ? "starting…" : "start"}
        </button>
      </footer>
    </div>
  );
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
    <span className="drift" title="the sandbox clones the remote, so this stays behind on the server's checkout">
      <Heads />
      {bits.join(" · ")}
    </span>
  );
}
