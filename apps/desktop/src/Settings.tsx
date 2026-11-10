// The settings screen: what a new session starts with, when this window
// notifies you, and the trackers the sidebar of tickets and pull requests reads.
// Signing in to hura cloud, which the trackers need, is the account screen's.
//
// Two halves, and the split between them is the only structure this screen
// has -- because it is the only structure that matters. **Who owns the answer.**
// A branch prefix names the work branch of every session on the server,
// including the ones `hura new` starts from a terminal, so it is a line in the
// server's config file and this screen edits that file. Whether this machine
// pings you when an agent waits is nobody's business but this window's, and two
// people looking at the same server from two machines would otherwise fight
// over one switch.
//
// So the top half says which file it writes and the bottom half says the values
// never leave the machine. A settings screen that mixed the two silently would
// be one where muting a notification asked permission from a server, and where
// changing a branch prefix appeared to work and then didn't for the CLI.
//
// **Written on save, not on keystroke.** Every other screen in this window acts
// immediately, and this is the exception on purpose: a config file re-read by
// every command is not somewhere to land a half-typed branch name, and the
// server refuses a policy that is not a template -- an error per character
// while somebody types `feature-work` is not a form, it is a fight. The window
// half is immediate, because a checkbox has nothing to validate against.
//
// The sidebars' widths and the refresh interval used to be fields here too.
// The widths are set by dragging a sidebar's edge (double-click resets it),
// and the interval is fixed: four number boxes for things nobody changed.

import { useEffect, useState } from "react";

import { api, messageOf } from "./api";
import { Waiting } from "./Empty";
import type { ConfiguredTracker } from "./gen/ConfiguredTracker";
import type { NamingKey } from "./gen/NamingKey";
import type { NewOptions } from "./gen/NewOptions";
import type { Settings } from "./gen/Settings";
import type { SettingsView } from "./gen/SettingsView";
import { Settings as SettingsGlyph } from "./icons";
import type { Prefs } from "./prefs";
import { Screen } from "./Screen";
import { Pill } from "./Pill";
import { Select } from "./Select";
import { TrackersSection } from "./Trackers";

export function SettingsScreen({
  server,
  prefs,
  onPrefs,
  at,
  onTrackers,
  signedIn,
  onSignIn,
  onClose,
}: {
  server: string;
  prefs: Prefs;
  onPrefs: (change: Partial<Prefs>) => void;
  /// Where to open it: at the trackers, when the sidebar sent you here to add
  /// one.
  at: "trackers" | null;
  /// The trackers after a change, so the sidebar reads them again.
  onTrackers: (trackers: ConfiguredTracker[]) => void;
  /// Signed in to hura cloud or out, which is whether there are trackers.
  /// `null` until the window has read the session.
  signedIn: boolean | null;
  /// To the account screen, from the trackers section signed out.
  onSignIn: () => void;
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
    // Not fatal on its own: the text fields work without it, and a runtime
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
    <Screen icon={SettingsGlyph} title="Settings" narrow onClose={onClose}>
      {error && <p className="error">{error}</p>}
      {!view && !error && <Waiting />}

      {view && draft && (
        <>
          <section className="setting-group">
            <h3>New sessions</h3>
            <p className="hint">
              Server defaults, shared with <code>hura new</code>. The create form can override them.
            </p>

            <label>
              <span>Branch prefix</span>
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
              <span>Agent</span>
              {/* Chat is what the server does with the key missing, so it is
                  the option a file that does not mention it shows. */}
              <Select
                value={draft.interface ?? "chat"}
                onChange={(v) => edit({ interface: v === "terminal" ? "terminal" : "chat" })}
                options={[
                  { value: "chat", label: "Chat", hint: "The Agent SDK, drawn by this window" },
                  { value: "terminal", label: "Terminal", hint: "Claude Code's own, in tmux" },
                ]}
              />
            </label>

            <label>
              <span>Base branch</span>
              <input
                value={draft.base ?? ""}
                placeholder="the remote's default branch"
                spellCheck={false}
                onChange={(e) => edit({ base: text(e.target.value) })}
              />
            </label>

            <label>
              <span>Policy</span>
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
                    label: options ? options.default_policy : "The default",
                    hint: "The built-in default",
                  },
                  ...(options?.policies ?? []).map((p) => ({
                    value: p.spec,
                    label: p.spec,
                    hint: p.summary,
                  })),
                  ...(draft.policy && !(options?.policies ?? []).some((p) => p.spec === draft.policy)
                    ? [{ value: draft.policy, label: draft.policy, hint: "From the config file" }]
                    : []),
                ]}
              />
            </label>
            {/* A policy set to a file path is a perfectly good answer and one
                no chooser can offer, so it is shown rather than silently
                replaced by the first template in the list. */}
            {draft.policy && !(options?.policies ?? []).some((p) => p.spec === draft.policy) && (
              <p className="hint indent">
                <code>{draft.policy}</code> is a file, not a template.
              </p>
            )}

            <fieldset>
              <legend>Credentials</legend>
              {!options && <p className="hint">Reading the credentials…</p>}
              {options?.providers_error && <p className="error">{options.providers_error}</p>}
              {options && options.providers.length === 0 && !options.providers_error && (
                <p className="hint">
                  None configured: add a <code>[credentials.NAME]</code> table to the server's config file
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
            <h3>Session names</h3>
            <p className="hint">
              For a session started without a name: a title to show it by, and the slug its branch and
              sandbox are named from.
            </p>
            <label>
              <span>Names</span>
              {/* AI is what the server does with the key missing. */}
              <Select
                value={draft.names ?? "ai"}
                onChange={(v) => edit({ names: v === "rules" ? "rules" : "ai" })}
                options={[
                  { value: "ai", label: "By Claude Haiku", hint: "From the task, or by the rules if it can't be asked" },
                  { value: "rules", label: "By the rules", hint: "The task's first words. Nothing leaves the server" },
                ]}
              />
            </label>
            <NamingKeyField
              server={server}
              source={view.naming_key}
              // Only the key's status: the draft is the file's, and a key is
              // not in the file.
              onChange={(naming_key) => setView((v) => (v ? { ...v, naming_key } : v))}
            />
          </section>

          <section className="setting-group">
            <h3>This server</h3>
            <label className="tick">
              <input
                type="checkbox"
                // Absent means on, which is what the server does with the
                // key missing -- so the box has to be ticked for a file that
                // does not mention it.
                checked={draft.auto_update ?? true}
                onChange={(e) => edit({ auto_update: e.target.checked })}
              />
              <span>Auto-update the server</span>
            </label>
            <p className="hint indent">
              Verified, then applied the next time <code>hurad</code> starts.
            </p>
          </section>

          <div className="setting-actions">
            <span className="hint path">
              {view.present ? "Writes " : "Creates "}
              <code>{view.path}</code>
            </span>
            {dirty && <span className="unsaved">unsaved</span>}
            {saved && !dirty && <span className="ok">saved</span>}
            <button className="go" disabled={saving || !dirty} onClick={() => void save()}>
              {saving ? "Saving…" : "Save"}
            </button>
          </div>
        </>
      )}

      <section className="setting-group">
        <h3>This window</h3>
        <p className="hint">Stored on this machine only.</p>

        <label className="tick">
          <input
            type="checkbox"
            checked={prefs.notify}
            onChange={(e) => onPrefs({ notify: e.target.checked })}
          />
          <span>Notify when an agent needs me</span>
        </label>

        {/* Signed in only: signed out there are no trackers, so nothing that
            could change, and a switch for it is a question about nothing. */}
        {signedIn && (
          <>
            <label className="tick">
              <input
                type="checkbox"
                checked={prefs.notifyTickets}
                onChange={(e) => onPrefs({ notifyTickets: e.target.checked })}
              />
              <span>Notify when my tickets or pull requests change</span>
            </label>
            <p className="hint indent">
              A status moved, somebody else's comment, a ticket newly yours, a review asked of you,
              and your pull requests' checks and reviews. Checked every few minutes.
            </p>
          </>
        )}
      </section>

      {/* Scrolled to once the server's half has drawn above it, or it lands
          where the section was before that half pushed it down. */}
      <TrackersSection
        key={String(signedIn)}
        signedIn={signedIn}
        onSignIn={onSignIn}
        focus={at === "trackers" && (view !== null || error !== null)}
        onChanged={onTrackers}
      />
    </Screen>
  );
}

/// The Anthropic API key naming calls Claude with, kept in the server's secret
/// store -- so per server, and write-only: this says whether there is one and
/// where it comes from, never what it is. Stored or forgotten at once rather
/// than with Save, because it is not part of the config file Save writes.
function NamingKeyField({
  server,
  source,
  onChange,
}: {
  server: string;
  source: NamingKey;
  onChange: (source: NamingKey) => void;
}) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const change = async (next: string | null) => {
    setBusy(true);
    setError(null);
    try {
      await api.secret(server, "ANTHROPIC_API_KEY", next);
      setValue("");
      onChange((await api.settings(server)).naming_key);
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <label>
        <span>API key</span>
        <span className="key-field">
          <input
            type="password"
            value={value}
            placeholder={source === "stored" ? "Stored. Paste to replace it" : "sk-ant-…"}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => setValue(e.target.value)}
          />
          <button disabled={busy || value.trim() === ""} onClick={() => void change(value.trim())}>
            Store
          </button>
          {source === "stored" && (
            <button disabled={busy} onClick={() => void change(null)}>
              Forget
            </button>
          )}
        </span>
      </label>
      {error && <p className="error indent">{error}</p>}
      <p className="hint indent">
        {source === "stored" && "Stored on this server, which names with it. It can't be shown again."}
        {source === "environment" && (
          <>
            From <code>ANTHROPIC_API_KEY</code> in <code>hurad</code>'s environment. One stored here
            would be used instead.
          </>
        )}
        {source === "none" && (
          <>
            None, so naming uses the <code>claude</code> command on the server if it is there and signed
            in. Stored on the server, never sent back here.
          </>
        )}
      </p>
    </>
  );
}
