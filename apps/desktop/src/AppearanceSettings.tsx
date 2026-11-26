// How the window looks and which keys do what: T3 Code's Appearance and
// Keybindings pages, in its rows. A VS Code theme taken in, the faces and the
// sizes, and every shortcut, changed by pressing the new one. What each does
// once chosen is `look.ts`'s and `keys.ts`'s.

import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { useEffect, useRef, useState } from "react";

import { type Command, COMMANDS, comboFrom, comboOf, keysLabel, useKeys } from "./keys";
import { CODE_SIZE, readVsCodeTheme } from "./look";
import { cn } from "./lib/cn";
import type { Prefs } from "./prefs";
import { Segmented, SettingsPage, SettingsRow, SettingsSection } from "./settings/layout";
import { SettingsActions } from "./settings/SettingsModal";
import { APPEARANCES } from "./theme";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Switch } from "./ui/switch";

type Props = { prefs: Prefs; onPrefs: (change: Partial<Prefs>) => void };

const SIZES = [0.9, 1, 1.1, 1.25];

export function AppearancePage({ prefs, onPrefs }: Props) {
  const [error, setError] = useState<string | null>(null);
  const file = useRef<HTMLInputElement>(null);
  const take = (text: string, name?: string) => {
    try {
      onPrefs({ theme: readVsCodeTheme(text, name) });
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <SettingsPage notice="Kept on this computer, for this window only.">
      <SettingsSection title="Theme">
        <SettingsRow
          title="Mode"
          description={
            prefs.theme
              ? "The VS Code theme in use decides, while it is."
              : "System follows the computer's own."
          }
          control={
            <Segmented
              label="Mode"
              value={prefs.appearance}
              options={APPEARANCES}
              onChange={(appearance) => onPrefs({ appearance })}
            />
          }
        />
        <SettingsRow
          title="VS Code theme"
          description={
            prefs.theme ? (
              <>
                <span className="font-medium text-foreground">{prefs.theme.name}</span>, {prefs.theme.scheme}, in place
                of the window's colours, the terminals' included.
              </>
            ) : (
              <>
                A colour theme's JSON, from an extension's <code>themes</code> folder. Its colours take the window's
                place, in its own light or dark.
              </>
            )
          }
          status={error ? <span className="text-destructive-foreground">{error}</span> : undefined}
          control={
            <>
              <Button variant="outline" size="sm" onClick={() => file.current?.click()}>
                Choose a File…
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() =>
                  void readText().then(
                    (text) => take(text),
                    () => setError("Nothing to paste: copy a theme's JSON first."),
                  )
                }
              >
                Paste
              </Button>
              {prefs.theme && (
                <Button variant="ghost" size="sm" onClick={() => onPrefs({ theme: null })}>
                  Stop Using It
                </Button>
              )}
              <input
                ref={file}
                type="file"
                accept=".json,.jsonc,application/json"
                hidden
                onChange={(e) => {
                  const chosen = e.target.files?.[0];
                  e.target.value = "";
                  if (chosen) void chosen.text().then((text) => take(text, chosen.name));
                }}
              />
            </>
          }
        />
        <SettingsRow
          title="Keep terminals dark"
          description="For programs drawn for a dark terminal, when the window is light."
          control={<Switch checked={prefs.terminalsDark} onCheckedChange={(on) => onPrefs({ terminalsDark: on })} />}
        />
      </SettingsSection>

      <SettingsSection title="Typography">
        <SettingsRow
          title="Interface font"
          description="A face installed on this computer, by name."
          control={
            <Input
              className="w-56"
              value={prefs.uiFont}
              placeholder="Inter"
              spellCheck={false}
              onChange={(e) => onPrefs({ uiFont: e.target.value })}
            />
          }
        />
        <SettingsRow
          title="Code font"
          description="The terminals, the files and the diffs."
          control={
            <Input
              className="w-56"
              value={prefs.monoFont}
              placeholder="The system's monospace"
              spellCheck={false}
              onChange={(e) => onPrefs({ monoFont: e.target.value })}
            />
          }
        />
        <SettingsRow
          title="Code size"
          description={`In pixels, ${CODE_SIZE.min} to ${CODE_SIZE.max}.`}
          control={
            <Input
              className="w-20"
              type="number"
              min={CODE_SIZE.min}
              max={CODE_SIZE.max}
              value={prefs.codeSize}
              onChange={(e) => {
                const n = Number(e.target.value);
                if (Number.isFinite(n) && n >= CODE_SIZE.min && n <= CODE_SIZE.max) onPrefs({ codeSize: n });
              }}
            />
          }
        />
        <SettingsRow
          title="Interface size"
          description="The whole window, larger or smaller."
          control={
            <Segmented
              label="Interface size"
              value={prefs.zoom}
              options={SIZES.map((size) => ({ value: size, label: `${Math.round(size * 100)}%` }))}
              onChange={(zoom) => onPrefs({ zoom })}
            />
          }
        />
      </SettingsSection>
    </SettingsPage>
  );
}

/// Every shortcut, and changing one by pressing it. Escape stops listening,
/// Backspace unbinds; a binding another command already has is said, not
/// refused, since which one wins is for whoever is binding.
export function KeybindingsPage({ prefs, onPrefs }: Props) {
  useKeys();
  const [recording, setRecording] = useState<Command | null>(null);

  useEffect(() => {
    if (!recording) return;
    const key = (e: KeyboardEvent) => {
      // Taken before anything else sees it: this keystroke is a binding, not a
      // shortcut to act on, and its Escape is not the card's.
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") return setRecording(null);
      const combo = e.key === "Backspace" && !e.ctrlKey && !e.metaKey && !e.altKey ? "" : comboFrom(e);
      if (combo === null) return;
      set(recording, combo);
      setRecording(null);
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
    // `set` reads `prefs.keys`, which only changes after this effect ends.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recording]);

  const set = (command: Command, combo: string) => {
    const keys = { ...prefs.keys };
    if (combo === COMMANDS[command].combo) delete keys[command];
    else keys[command] = combo;
    onPrefs({ keys });
  };
  const ids = Object.keys(COMMANDS) as Command[];
  const changed = ids.some((id) => id in prefs.keys);

  return (
    <>
      {changed && (
        <SettingsActions>
          <Button size="xs" variant="ghost" onClick={() => onPrefs({ keys: {} })}>
            Restore Defaults
          </Button>
        </SettingsActions>
      )}
      <SettingsPage notice="Kept on this computer. Click a shortcut and press the keys for it; Backspace leaves a command without one, Escape stops.">
        <SettingsSection title="Shortcuts">
          {ids.map((id) => {
            const combo = comboOf(id);
            const clash = combo ? ids.find((other) => other !== id && comboOf(other) === combo) : undefined;
            return (
              <SettingsRow
                key={id}
                title={COMMANDS[id].label}
                status={
                  clash ? <span className="text-warning-foreground">Also {COMMANDS[clash].label}</span> : undefined
                }
                control={
                  <>
                    {id in prefs.keys && (
                      <Button
                        size="xs"
                        variant="ghost-muted"
                        title={`Back to ${keysLabel(COMMANDS[id].combo) || "None"}`}
                        onClick={() => set(id, COMMANDS[id].combo)}
                      >
                        Reset
                      </Button>
                    )}
                    <Button
                      size="xs"
                      variant="outline"
                      className={cn(
                        "min-w-28 font-sans",
                        !combo && recording !== id && "text-muted-foreground",
                        recording === id && "border-ring text-foreground",
                      )}
                      aria-label={`Shortcut for ${COMMANDS[id].label}`}
                      onClick={() => setRecording(recording === id ? null : id)}
                    >
                      {recording === id ? "Press the keys…" : keysLabel(combo) || "None"}
                    </Button>
                  </>
                }
              />
            );
          })}
        </SettingsSection>
      </SettingsPage>
    </>
  );
}

/// How diffs are drawn, T3 Code's Diffs page: every diff in the window, the
/// changes beside a file and a pull request's alike, and the ones open now.
export function DiffsPage({ prefs, onPrefs }: Props) {
  return (
    <SettingsPage notice="Kept on this computer, for this window only.">
      <SettingsSection title="Diffs">
        <SettingsRow
          title="Layout"
          description="The old and the new side by side, or one above the other. A pull request too narrow for two columns stacks them whatever this says."
          control={
            <Segmented
              label="Diff layout"
              value={prefs.diffStacked ? "stacked" : "split"}
              options={[
                { value: "split", label: "Side by Side" },
                { value: "stacked", label: "Stacked" },
              ]}
              onChange={(v) => onPrefs({ diffStacked: v === "stacked" })}
            />
          }
        />
        <SettingsRow
          title="Hide whitespace-only changes"
          description="Lines that differ only in their spaces and tabs are left out of the diff."
          control={
            <Switch
              checked={prefs.diffHideWhitespace}
              onCheckedChange={(on) => onPrefs({ diffHideWhitespace: on })}
            />
          }
        />
      </SettingsSection>
    </SettingsPage>
  );
}
