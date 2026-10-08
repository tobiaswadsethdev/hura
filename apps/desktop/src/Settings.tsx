// The settings screen: what a new session starts with, and how this window
// looks.
//
// Two sections, and the split between them is the only structure this screen
// has -- because it is the only structure that matters. **Who owns the answer.**
// A branch prefix names the work branch of every session on the server,
// including the ones `hura new` starts from a terminal, so it is a line in the
// server's config file and this screen edits that file. How wide the sidebars
// are is nobody's business but this window's, and two people looking at the
// same server from two machines would otherwise fight over one number.
//
// So the top half says which file it writes and the bottom half says the values
// never leave the machine. A settings screen that mixed the two silently would
// be one where changing a width asked permission from a server, and where
// changing a branch prefix appeared to work and then didn't for the CLI.
//
// **Written on save, not on keystroke.** Every other screen in this window acts
// immediately, and this is the exception on purpose: a config file re-read by
// every command is not somewhere to land a half-typed branch name, and the
// server refuses a policy that is not a template -- an error per character
// while somebody types `feature-work` is not a form, it is a fight. The window
// half is immediate, because it has nothing to validate against and the point
// of a width is seeing it.

import { useEffect, useState } from "react";

import { api, messageOf } from "./api";
import { Waiting } from "./Empty";
import type { NewOptions } from "./gen/NewOptions";
import type { Settings } from "./gen/Settings";
import type { SettingsView } from "./gen/SettingsView";
import { Settings as SettingsGlyph } from "./icons";
import { DEFAULTS, LIMITS, type Prefs } from "./prefs";
import { Screen } from "./Screen";
import { Pill } from "./Pill";
import { Select } from "./Select";

export function SettingsScreen({
  server,
  prefs,
  onPrefs,
  onClose,
}: {
  server: string;
  prefs: Prefs;
  onPrefs: (change: Partial<Prefs>) => void;
  onClose: () => void;
}) {
  const [view, setView] = useState<SettingsView | null>(null);
  /// What the choosers are filled from. The same request the create form makes,
  /// reused rather than a second one: the policies and the credential providers
  /// this server has are one fact, and a settings screen listing a different
  /// set to the form would be the server disagreeing with itself.
  const [options, setOptions] = useState<NewOptions | null>(null);
  /// The edited copy. `null` until the server has answered, so there is no
  /// moment where this screen shows a blank field that means "not set" and is
  /// really "not loaded".
  const [draft, setDraft] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  /// Cleared on the next edit, so it reads as "that save worked" rather than
  /// as a permanent claim about the current state of the form.
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let live = true;
    api
      .settings(server)
      .then((v) => {
        if (!live) return;
        setView(v);
        setDraft(v.settings);
      })
      .catch((e) => live && setError(messageOf(e)));
    // Not fatal on its own: the text fields work without it, and a gateway
    // that cannot be reached is exactly when somebody wants to look at this
    // screen. The two choosers say so where they would have been.
    api.newOptions(server).then(
      (o) => live && setOptions(o),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [server]);

  /// One field of the draft.
  const edit = (change: Partial<Settings>) => {
    setSaved(false);
    setDraft((current) => (current ? { ...current, ...change } : current));
  };

  /// Blank is not a value, it is the absence of one.
  ///
  /// The whole screen turns on this: an empty branch prefix field means "take
  /// the key out of the file and go back to `hura`", not "name branches
  /// `/foo`". The server normalises the same way, and this is here so the
  /// placeholder a person sees and the value that gets sent agree.
  const text = (value: string): string | null => (value.trim() === "" ? null : value);

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    setError(null);
    try {
      const written = await api.setSettings(server, draft);
      // What the file now says, not what was sent: a cleared field came back
      // as an absent key, and the fields have to show that rather than the
      // blank that produced it.
      setView(written);
      setDraft(written.settings);
      setSaved(true);
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setSaving(false);
    }
  };

  /// Whether the draft still differs from the file.
  ///
  /// Worth computing rather than tracking a boolean, because a save answers
  /// with the file re-read: a field cleared to blank comes back as an absent
  /// key, `draft` is replaced by what came back, and this goes false on its own
  /// without anything having to remember to reset it. Compared as JSON because
  /// both sides are the same generated shape with the same key order -- `draft`
  /// is a spread of `view.settings` -- so there is nothing to walk.
  ///
  /// This screen is the one in the window with a save button, so it is the one
  /// that owes an answer to "did that take?". Not a prompt on close: a settings
  /// screen that will not let go of the window is worse than a discarded edit
  /// to a branch prefix.
  const dirty = view !== null && draft !== null && JSON.stringify(draft) !== JSON.stringify(view.settings);

  const providers = draft?.providers ?? [];
  const toggleProvider = (name: string) =>
    edit({
      providers: providers.includes(name)
        ? providers.filter((p) => p !== name)
        : [...providers, name],
    });

  return (
    <Screen icon={SettingsGlyph} title="settings" narrow onClose={onClose}>
      {error && <p className="error">{error}</p>}
      {!view && !error && <Waiting />}

      {view && draft && (
        <>
          <section className="setting-group">
            <h3>new sessions</h3>
            <p className="hint">
              Server defaults, shared with <code>hura new</code>. The create form can override them.
            </p>

            <label>
              <span>branch prefix</span>
              <input
                value={draft.branch_prefix ?? ""}
                placeholder={view.default_branch_prefix}
                spellCheck={false}
                onChange={(e) => edit({ branch_prefix: text(e.target.value) })}
              />
            </label>
            {/* The example, computed rather than described. `<prefix>/<name>`
                is a sentence somebody has to decode; `tobias/add-auth` is
                the branch they are about to have. */}
            <p className="hint indent">
              e.g.{" "}
              <code>
                {(draft.branch_prefix?.trim() || view.default_branch_prefix).replace(/\/+$/, "")}
                /add-auth
              </code>
            </p>

            <label>
              <span>agent</span>
              {/* Chat is what the server does with the key missing, so it is
                  the option a file that does not mention it shows. */}
              <Select
                value={draft.interface ?? "chat"}
                onChange={(v) => edit({ interface: v === "terminal" ? "terminal" : "chat" })}
                options={[
                  { value: "chat", label: "chat", hint: "the Agent SDK, drawn by this window" },
                  { value: "terminal", label: "terminal", hint: "Claude Code's own, in tmux" },
                ]}
              />
            </label>

            <label>
              <span>base branch</span>
              <input
                value={draft.base ?? ""}
                placeholder="the remote's default branch"
                spellCheck={false}
                onChange={(e) => edit({ base: text(e.target.value) })}
              />
            </label>

            <label>
              <span>policy</span>
              {/* The built-in default is an option rather than a blank row:
                  "whatever hura chooses" is a real answer here, and the one
                  that survives the default changing. A policy set to a path
                  is shown as itself, since no template is it. */}
              <Select
                value={draft.policy ?? ""}
                onChange={(v) => edit({ policy: text(v) })}
                options={[
                  {
                    value: "",
                    label: options ? options.default_policy : "the default",
                    hint: "the built-in default",
                  },
                  ...(options?.policies ?? []).map((p) => ({
                    value: p.spec,
                    label: p.spec,
                    hint: p.summary,
                  })),
                  ...(draft.policy && !(options?.policies ?? []).some((p) => p.spec === draft.policy)
                    ? [{ value: draft.policy, label: draft.policy, hint: "from the config file" }]
                    : []),
                ]}
              />
            </label>
            {/* A policy set to a YAML path is a perfectly good answer and one
                no chooser can offer, so it is shown rather than silently
                replaced by the first template in the list. */}
            {draft.policy && !(options?.policies ?? []).some((p) => p.spec === draft.policy) && (
              <p className="hint indent">
                <code>{draft.policy}</code> is a file, not a template.
              </p>
            )}

            <fieldset>
              <legend>credentials</legend>
              {!options && <p className="hint">reading the credentials…</p>}
              {options?.providers_error && <p className="error">{options.providers_error}</p>}
              {options && options.providers.length === 0 && !options.providers_error && (
                <p className="hint">
                  none configured: add a <code>[credentials.NAME]</code> table to the server's config file
                </p>
              )}
              <div className="pills">
                {(options?.providers ?? []).map((p) => (
                  <Pill
                    key={p.name}
                    on={providers.includes(p.name)}
                    title={p.kind}
                    onClick={() => toggleProvider(p.name)}
                  >
                    {p.name}
                  </Pill>
                ))}
              </div>
              <p className="hint">None picked: each repository gets its own guess.</p>
            </fieldset>
          </section>

          <section className="setting-group">
            <h3>this server</h3>
            <label className="tick">
              <input
                type="checkbox"
                // Absent means on, which is what the server does with the
                // key missing -- so the box has to be ticked for a file that
                // does not mention it.
                checked={draft.auto_update ?? true}
                onChange={(e) => edit({ auto_update: e.target.checked })}
              />
              <span>auto-update the server</span>
            </label>
            <p className="hint indent">
              Verified, then applied the next time <code>hurad</code> starts.
            </p>
          </section>

          <div className="setting-actions">
            <span className="hint path">
              {view.present ? "writes " : "creates "}
              <code>{view.path}</code>
            </span>
            {dirty && <span className="unsaved">unsaved</span>}
            {saved && !dirty && <span className="ok">saved</span>}
            <button className="go" disabled={saving || !dirty} onClick={() => void save()}>
              {saving ? "saving…" : "save"}
            </button>
          </div>
        </>
      )}

      <section className="setting-group">
        <h3>this window</h3>
        <p className="hint">Stored on this machine only.</p>

        <label>
          <span>projects width</span>
          <Pixels
            value={prefs.treeWidth}
            limit={LIMITS.treeWidth}
            onChange={(treeWidth) => onPrefs({ treeWidth })}
          />
        </label>

        <label>
          <span>dock width</span>
          <Pixels
            value={prefs.dockWidth}
            limit={LIMITS.dockWidth}
            onChange={(dockWidth) => onPrefs({ dockWidth })}
          />
        </label>
        <p className="hint indent">Or drag a sidebar's edge; double-click to reset.</p>

        <label>
          <span>refresh</span>
          <Pixels
            value={prefs.refreshMs}
            limit={LIMITS.refreshMs}
            unit="ms"
            step={250}
            onChange={(refreshMs) => onPrefs({ refreshMs })}
          />
        </label>
        <p className="hint indent">How often the worktree list is re-read. Raise it over a VPN.</p>

        <label className="tick">
          <input
            type="checkbox"
            checked={prefs.notify}
            onChange={(e) => onPrefs({ notify: e.target.checked })}
          />
          <span>notify when an agent needs me</span>
        </label>

        <label className="tick">
          <input
            type="checkbox"
            checked={prefs.notifyTickets}
            onChange={(e) => onPrefs({ notifyTickets: e.target.checked })}
          />
          <span>notify when my tickets change</span>
        </label>
        <p className="hint indent">Status changes and other people's comments, checked every few minutes.</p>

        <div className="setting-actions">
          <button className="quiet" onClick={() => onPrefs(DEFAULTS)}>
            back to defaults
          </button>
        </div>
      </section>
    </Screen>
  );
}

/// A number with a unit and a range, held as text while it is being typed.
///
/// `<input type="number">` was the first attempt and is the wrong control here:
/// clearing it to retype produces `""`, which is not a number, and a parent
/// holding the real value re-renders the field back to what it was mid-edit --
/// so `260` cannot be turned into `320` without selecting it all first. The text
/// stays local and only a value inside the range is committed upward.
function Pixels({
  value,
  limit,
  unit = "px",
  step = 10,
  onChange,
}: {
  value: number;
  limit: { min: number; max: number };
  unit?: string;
  step?: number;
  onChange: (value: number) => void;
}) {
  const [text, setText] = useState(String(value));

  // The value can also change from a drag, and the field has to follow it --
  // this and the sidebar's edge are two ways to set one number.
  //
  // Safe against what it looks like, which is an effect that overwrites what
  // somebody is typing: `value` only moves when `commit` below runs or when
  // the handle is dragged, so `18` on the way to `180` never reaches the
  // parent and never comes back.
  useEffect(() => {
    setText(String(value));
  }, [value]);

  const commit = (raw: string) => {
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) return;
    onChange(Math.min(limit.max, Math.max(limit.min, Math.round(parsed))));
  };

  return (
    <span className="pixels">
      <input
        value={text}
        inputMode="numeric"
        onChange={(e) => setText(e.target.value)}
        // On blur and on Enter, not on change: `18` on the way to `180` is
        // below the floor and would be clamped to it under the cursor.
        onBlur={() => commit(text)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit(text);
            return;
          }
          const by = e.key === "ArrowUp" ? step : e.key === "ArrowDown" ? -step : 0;
          if (by !== 0) {
            e.preventDefault();
            onChange(Math.min(limit.max, Math.max(limit.min, value + by)));
          }
        }}
      />
      <span className="unit">{unit}</span>
      <span className="range">
        {limit.min}–{limit.max}
      </span>
    </span>
  );
}
