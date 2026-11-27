// What each project's new worktrees start with, T3 Code's project defaults:
// a section per project, each choice either the server's default or this
// project's own. Kept with the project on the server (`ProjectDefaults`), so
// every window starting a worktree there agrees, and saved as it is changed,
// text when its field is left. The new-worktree draft opens with them.

import { useEffect, useState } from "react";

import { api, messageOf } from "./api";
import { Waiting } from "./Empty";
import type { NewOptions } from "./gen/NewOptions";
import type { Project } from "./gen/Project";
import type { ProjectDefaults } from "./gen/ProjectDefaults";
import { Pill } from "./Pill";
import { Select } from "./Select";
import { Segmented, SettingsPage, SettingsRow, SettingsSection } from "./settings/layout";
import { Input } from "./ui/input";

export function ProjectsPage({
  server,
  projects,
  onProjects,
}: {
  server: string;
  projects: Project[];
  /// The list as the server has it after a change.
  onProjects: (projects: Project[]) => void;
}) {
  const [options, setOptions] = useState<NewOptions | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    api.newOptions(server).then(
      (o) => live && setOptions(o),
      (e) => live && setError(messageOf(e)),
    );
    return () => {
      live = false;
    };
  }, [server]);

  const save = (name: string, defaults: ProjectDefaults) =>
    api.setProjectDefaults(server, name, defaults).then(onProjects, (e) => setError(messageOf(e)));

  return (
    <SettingsPage notice="Kept with each project on this server. What is left as the server's default follows the server's config.">
      {error && <p className="px-3 text-sm text-destructive-foreground sm:px-4">{error}</p>}
      {!options && !error && <Waiting />}
      {options && projects.length === 0 && (
        <p className="px-3 text-sm text-muted-foreground sm:px-4">No projects yet.</p>
      )}
      {options &&
        projects.map((p) => (
          <ProjectDefaultsSection
            key={p.name}
            project={p}
            options={options}
            onSave={(defaults) => void save(p.name, defaults)}
          />
        ))}
    </SettingsPage>
  );
}

function ProjectDefaultsSection({
  project,
  options,
  onSave,
}: {
  project: Project;
  options: NewOptions;
  onSave: (defaults: ProjectDefaults) => void;
}) {
  const d = project.defaults ?? {};
  const change = (to: Partial<ProjectDefaults>) => onSave({ ...d, ...to });
  // Typed, and saved when the field is left: a branch name half typed is not
  // somewhere to start a worktree from.
  const [base, setBase] = useState(d.base ?? "");
  useEffect(() => setBase(d.base ?? ""), [d.base]);
  const toggle = (list: string[] | null | undefined, value: string) => {
    const now = list ?? [];
    const next = now.includes(value) ? now.filter((v) => v !== value) : [...now, value];
    // None picked is the server's default, not "none at all".
    return next.length ? next : null;
  };

  return (
    <SettingsSection title={project.name} description={project.repo}>
      <SettingsRow
        title="Agent"
        description="How you talk to a new worktree's agent."
        control={
          <Segmented
            label="Agent"
            value={d.interface ?? "default"}
            options={[
              { value: "default", label: "Server's" },
              { value: "chat", label: "Chat" },
              { value: "terminal", label: "Terminal" },
            ]}
            onChange={(v) => change({ interface: v === "default" ? null : (v as "chat" | "terminal") })}
          />
        }
      />
      {options.worktrees && (
        <SettingsRow
          title="Runs in"
          description="A sandbox of its own, or a git worktree on the server as its user, with no policy."
          control={
            <Segmented
              label="Runs in"
              value={d.worktree === true ? "worktree" : d.worktree === false ? "sandbox" : "default"}
              options={[
                { value: "default", label: "Server's" },
                { value: "sandbox", label: "Sandbox" },
                { value: "worktree", label: "Worktree" },
              ]}
              onChange={(v) => change({ worktree: v === "default" ? null : v === "worktree" })}
            />
          }
        />
      )}
      <SettingsRow
        title="Policy"
        description="What a sandbox lets the agent reach."
        control={
          <div data-legacy className="w-56">
            <Select
              className="w-full"
              value={d.policy ?? ""}
              onChange={(v) => change({ policy: v || null })}
              options={[
                { value: "", label: `Server's (${options.default_policy})`, hint: "The config file's" },
                ...options.policies.map((p) => ({ value: p.spec, label: p.spec, hint: p.summary })),
              ]}
            />
          </div>
        }
      />
      <SettingsRow
        title="Base branch"
        description="What a new work branch starts from."
        control={
          <Input
            className="w-56"
            value={base}
            placeholder={options.default_base ?? "The checkout's branch"}
            spellCheck={false}
            onChange={(e) => setBase(e.target.value)}
            onBlur={() => base !== (d.base ?? "") && change({ base: base.trim() || null })}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
        }
      />
      <SettingsRow
        title="Toolchains"
        description="None picked: the ones the checkout's files call for."
      >
        <div data-legacy className="pills mt-2.5">
          {options.toolchains.map((t) => (
            <Pill
              key={t.name}
              on={(d.toolchains ?? []).includes(t.name)}
              title={t.summary}
              onClick={() => change({ toolchains: toggle(d.toolchains, t.name) })}
            >
              {t.name}
            </Pill>
          ))}
        </div>
      </SettingsRow>
      <SettingsRow
        title="Credentials"
        description="None picked: the server's defaults, or what the checkout names."
      >
        <div data-legacy className="pills mt-2.5">
          {options.providers.map((c) => (
            <Pill
              key={c.name}
              on={(d.providers ?? []).includes(c.name)}
              title={c.kind}
              onClick={() => change({ providers: toggle(d.providers, c.name) })}
            >
              {c.name}
            </Pill>
          ))}
          {options.providers.length === 0 && <span className="hint">None configured</span>}
        </div>
      </SettingsRow>
    </SettingsSection>
  );
}
