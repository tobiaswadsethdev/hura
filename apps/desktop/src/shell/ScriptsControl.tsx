// The project's scripts in the header over the conversation, as T3 Code puts
// its project actions there: the first script as a button that runs it, a
// menu beside it with the rest, and a dialog to add, change and remove them.
// See `scripts.ts` for where they are kept, and `App` for running one.

import { type ComponentType, useEffect, useRef, useState } from "react";

import { type MenuItem, useButtonMenu } from "../ContextMenu";
import {
  Close,
  Disclosure,
  Plus,
  ScriptBuild,
  ScriptConfigure,
  ScriptDebug,
  ScriptLint,
  ScriptRun,
  ScriptTest,
  Settings,
} from "../icons";
import { cn } from "../lib/cn";
import { newScriptId, SCRIPT_ICONS, type Script, type ScriptIcon, useScripts } from "../scripts";
import { Button } from "../ui/button";

export const SCRIPT_GLYPHS: Record<ScriptIcon, ComponentType<{ className?: string; "aria-hidden"?: boolean }>> = {
  play: ScriptRun,
  test: ScriptTest,
  build: ScriptBuild,
  lint: ScriptLint,
  debug: ScriptDebug,
  configure: ScriptConfigure,
};

const ICON_NAMES: Record<ScriptIcon, string> = {
  play: "Run",
  test: "Test",
  build: "Build",
  lint: "Lint",
  debug: "Debug",
  configure: "Configure",
};

export function ScriptsControl({
  server,
  project,
  onRun,
}: {
  server: string;
  project: string;
  onRun: (script: Script) => void;
}) {
  const [scripts, save] = useScripts(server, project);
  const [editing, setEditing] = useState(false);
  const menu = useButtonMenu();
  const first = scripts[0];

  const items = (): MenuItem[] => [
    ...scripts.map((s) => ({ label: s.name, icon: SCRIPT_GLYPHS[s.icon], run: () => onRun(s) })),
    "separator",
    { label: "Edit Scripts…", icon: Settings, run: () => setEditing(true) },
  ];

  return (
    <>
      {first ? (
        <div className="flex shrink-0 items-center">
          <Button
            variant="outline"
            size="xs"
            className="rounded-r-none"
            title={`Run ${first.name} in a New Shell: ${first.command}`}
            onClick={() => onRun(first)}
          >
            {(() => {
              const Glyph = SCRIPT_GLYPHS[first.icon];
              return <Glyph aria-hidden />;
            })()}
            <span className="max-w-32 truncate">{first.name}</span>
          </Button>
          <Button
            variant="outline"
            size="xs"
            className="-ml-px rounded-l-none px-1"
            title="The Project's Scripts"
            aria-label="More scripts"
            aria-haspopup="menu"
            onClick={menu(items)}
          >
            <Disclosure aria-hidden />
          </Button>
        </div>
      ) : (
        <Button
          variant="ghost-muted"
          size="xs"
          className="shrink-0"
          title="Commands to Run in This Project's Worktrees, a Click Away"
          onClick={() => setEditing(true)}
        >
          <Plus aria-hidden />
          Add Script
        </Button>
      )}
      {editing && (
        <ScriptsDialog
          project={project}
          scripts={scripts}
          onSave={(next) => {
            save(next);
            setEditing(false);
          }}
          onClose={() => setEditing(false)}
        />
      )}
    </>
  );
}

function ScriptsDialog({
  project,
  scripts,
  onSave,
  onClose,
}: {
  project: string;
  scripts: Script[];
  onSave: (next: Script[]) => void;
  onClose: () => void;
}) {
  const blank = (): Script => ({ id: newScriptId(), name: "", command: "", icon: "play" });
  const [rows, setRows] = useState<Script[]>(() => (scripts.length ? scripts : [blank()]));
  const first = useRef<HTMLInputElement>(null);
  useEffect(() => first.current?.focus(), []);
  useEffect(() => {
    // Not an Escape a menu over the dialog (the icon's) has already taken.
    const key = (e: KeyboardEvent) => e.key === "Escape" && !e.defaultPrevented && onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);
  const change = (id: string, to: Partial<Script>) => setRows((all) => all.map((r) => (r.id === id ? { ...r, ...to } : r)));
  const half = rows.some((r) => !r.name.trim() !== !r.command.trim());
  const field =
    "h-8 min-w-0 rounded-md bg-foreground/5 px-2.5 text-sm text-foreground outline-none ring-1 ring-border placeholder:text-muted-foreground focus:ring-ring";

  return (
    <div
      data-tw
      className="fixed inset-0 z-[100] flex items-start justify-center bg-black/40 pt-[14vh]"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <form
        role="dialog"
        aria-label="Project scripts"
        className="flex max-h-[70vh] w-[min(680px,calc(100vw-48px))] flex-col overflow-hidden rounded-2xl bg-popover text-sm shadow-[0_24px_72px_-20px_rgb(0_0_0/0.6)] ring-1 ring-border"
        onSubmit={(e) => {
          e.preventDefault();
          if (!half) onSave(rows);
        }}
      >
        <header className="px-5 pt-5 pb-3">
          <h2 className="text-base font-semibold text-foreground">Scripts for {project}</h2>
          <p className="mt-1 text-muted-foreground">
            Each runs in a new shell in the worktree's sandbox, from the repository's root. The first is the
            header's button.
          </p>
        </header>
        <ul className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-5 pb-2">
          {rows.map((r, i) => (
            <li key={r.id} className="flex items-center gap-2">
              <IconPicker value={r.icon} onChange={(icon) => change(r.id, { icon })} />
              <input
                ref={i === 0 ? first : undefined}
                className={cn(field, "w-36 shrink-0")}
                placeholder="Name"
                aria-label="Name"
                value={r.name}
                onChange={(e) => change(r.id, { name: e.target.value })}
              />
              <input
                className={cn(field, "flex-1 font-mono text-xs")}
                placeholder="npm run dev"
                aria-label="Command"
                spellCheck={false}
                value={r.command}
                onChange={(e) => change(r.id, { command: e.target.value })}
              />
              <Button
                type="button"
                variant="ghost-destructive"
                size="icon-xs"
                title="Remove This Script"
                aria-label="Remove"
                onClick={() => setRows((all) => all.filter((x) => x.id !== r.id))}
              >
                <Close aria-hidden />
              </Button>
            </li>
          ))}
          <li>
            <Button type="button" variant="ghost-muted" size="xs" onClick={() => setRows((all) => [...all, blank()])}>
              <Plus aria-hidden />
              Add a Script
            </Button>
          </li>
        </ul>
        <footer className="flex items-center justify-end gap-2 border-t border-border px-5 py-3">
          {half && <span className="mr-auto text-xs text-muted-foreground">Each script needs a name and a command.</span>}
          <Button type="button" variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" size="sm" disabled={half}>
            Save
          </Button>
        </footer>
      </form>
    </div>
  );
}

/// The icon a script wears, picked from T3 Code's six with a menu.
function IconPicker({ value, onChange }: { value: ScriptIcon; onChange: (icon: ScriptIcon) => void }) {
  const menu = useButtonMenu();
  const Glyph = SCRIPT_GLYPHS[value];
  return (
    <Button
      type="button"
      variant="outline"
      size="icon-sm"
      className="shrink-0"
      title={`Icon: ${ICON_NAMES[value]}`}
      aria-label="Icon"
      aria-haspopup="menu"
      onClick={menu(() =>
        SCRIPT_ICONS.map((icon) => ({
          label: ICON_NAMES[icon],
          icon: SCRIPT_GLYPHS[icon],
          run: () => onChange(icon),
        })),
      )}
    >
      <Glyph aria-hidden />
    </Button>
  );
}
