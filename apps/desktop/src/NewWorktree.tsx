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
import { Close } from "./icons";
import type { Facts } from "./gen/Facts";
import type { NewOptions } from "./gen/NewOptions";
import type { Picked } from "./gen/Picked";
import type { Project } from "./gen/Project";
import type { Task } from "./gen/Task";
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
      <div className="dialog" onMouseDown={(e) => e.stopPropagation()}>
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
      });
      onCreated(created);
    } catch (e) {
      setError(messageOf(e));
      setBusy(false);
    }
  };

  return (
    <>
      <header className="dialog-head">
        <h2>{project.name}</h2>
        <button className="quiet-icon" title="close" onClick={onClose}>
          <Close aria-label="close" />
        </button>
      </header>

      <p className="origin">{project.repo}</p>
      {from && (
        <p className="hint">
          <a href={from.url} target="_blank" rel="noreferrer">
            {from.key}
          </a>{" "}
          → <code>{from.branch}</code>
        </p>
      )}
      {facts && <Drift facts={facts} />}

      <label>
        <span>task</span>
        <textarea
          autoFocus
          rows={3}
          value={task}
          placeholder="what the agent should do"
          onChange={(e) => setTask(e.target.value)}
        />
      </label>

      <label>
        <span>name</span>
        <input
          value={name}
          placeholder="derived from the task"
          onChange={(e) => setName(e.target.value)}
        />
      </label>

      <label>
        <span>base</span>
        <input
          value={base}
          placeholder="the remote's default branch"
          onChange={(e) => setBase(e.target.value)}
        />
      </label>

      <label>
        <span>policy</span>
        <Select
          value={policy}
          onChange={setPolicy}
          options={options.policies.map((p) => ({ value: p.spec, label: p.spec, hint: p.summary }))}
        />
      </label>

      <fieldset>
        <legend>toolchains</legend>
        {options.toolchains.map((t) => (
          <label key={t.name} className="tick">
            <input
              type="checkbox"
              checked={toolchains.includes(t.name)}
              onChange={() => toggle(toolchains, setToolchains, t.name)}
            />
            <span>{t.name}</span>
            <span className="hint">{t.summary}</span>
          </label>
        ))}
      </fieldset>

      <fieldset>
        <legend>providers</legend>
        {options.providers_error && <p className="error">{options.providers_error}</p>}
        {options.providers.length === 0 && !options.providers_error && (
          <p className="hint">the gateway has no credential providers</p>
        )}
        {options.providers.map((p) => (
          <label key={p.name} className="tick">
            <input
              type="checkbox"
              checked={providers.includes(p.name)}
              onChange={() => toggle(providers, setProviders, p.name)}
            />
            <span>{p.name}</span>
            <span className="hint">{p.kind}</span>
          </label>
        ))}
      </fieldset>

      {/* Named, not offered: skills and MCP servers are one decision about
          what your agents can reach, made in the server's config file.
          Shown so a session's tools are not a surprise. */}
      <dl className="carried">
        <dt>skills</dt>
        <dd>{options.skills.join(", ") || <span className="hint">none</span>}</dd>
        <dt>mcp</dt>
        <dd>{options.mcp.join(", ") || <span className="hint">none</span>}</dd>
      </dl>
      {options.mcp.length > 0 && (
        // The cost of an MCP server, said where a session is about to be
        // given one rather than only in a document. Everything that server
        // can do is now something this agent can do with your credentials,
        // and the gateway sees every call as the same `POST /mcp`.
        <p className="hint">
          Each of those is something the agent can do with the server's credentials — see{" "}
          <b>integrations</b>.
        </p>
      )}

      {mine.length > 0 && (
        <p className="hint">
          pushed from this machine first: {mine.join(", ")}
        </p>
      )}

      {error && <p className="error">{error}</p>}

      <div className="actions">
        <button className="go" disabled={busy} onClick={() => void submit()}>
          {busy ? "starting…" : "start session"}
        </button>
      </div>
    </>
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
  if (!facts.base_on_remote) bits.push("this branch is not on the remote");
  if (bits.length === 0) return null;
  return (
    <p className="notice">
      {bits.join(", ")} — the sandbox clones the remote
    </p>
  );
}
