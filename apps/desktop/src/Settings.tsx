// The settings' pages that are settings rather than screens of their own:
// what a new worktree starts with on this server, what this window tells you
// about, and the trackers. Drawn in T3 Code's rows inside the settings card
// (`settings/SettingsModal.tsx`); the window's look and its keys are
// `AppearanceSettings.tsx`'s.
//
// **Who owns the answer** is what splits them. A branch prefix names the work
// branch of every worktree on the server, including the ones `hura new`
// starts from a terminal, so it is a line in the server's config file and the
// page for it edits that file. Whether this machine pings you when an agent
// waits is nobody's business but this window's, and two people looking at the
// same server from two machines would otherwise fight over one switch. Each
// page says which it is, in the sentence over it.
//
// **The server's are written on save, not on keystroke.** Every other page
// acts immediately, and this is the exception on purpose: a config file
// re-read by every command is not somewhere to land a half-typed branch name,
// and the server refuses a policy that is not a template; an error per
// character while somebody types `feature-work` is not a form, it is a fight.
// Save is in the card's header, where T3 Code puts a page's own action.

import { useEffect, useState } from "react";

import { api, messageOf } from "./api";
import { Waiting } from "./Empty";
import type { ConfiguredTracker } from "./gen/ConfiguredTracker";
import type { NamingKey } from "./gen/NamingKey";
import type { NewOptions } from "./gen/NewOptions";
import type { Settings } from "./gen/Settings";
import type { SettingsView } from "./gen/SettingsView";
import { Pill } from "./Pill";
import type { Prefs } from "./prefs";
import { Select } from "./Select";
import { SettingsPage, SettingsRow, SettingsSection } from "./settings/layout";
import { SettingsActions } from "./settings/SettingsModal";
import { TrackersSection } from "./Trackers";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Switch } from "./ui/switch";

/// What this window tells you about.
export function GeneralPage({
  prefs,
  onPrefs,
  signedIn,
}: {
  prefs: Prefs;
  onPrefs: (change: Partial<Prefs>) => void;
  /// Signed in to hura cloud: without it there are no trackers, so nothing
  /// about them to be told.
  signedIn: boolean | null;
}) {
  return (
    <SettingsPage notice="Kept on this computer, for this window only.">
      <SettingsSection title="Notifications">
        <SettingsRow
          title="When an agent needs me"
          description="The system's notification when a worktree starts waiting on a question or a permission."
          control={<Switch checked={prefs.notify} onCheckedChange={(on) => onPrefs({ notify: on })} />}
        />
        {signedIn && (
          <SettingsRow
            title="When my tickets or pull requests change"
            description="A status moved, somebody else's comment, a ticket newly yours, a review asked of you, and your pull requests' checks and reviews. Checked every few minutes."
            control={
              <Switch checked={prefs.notifyTickets} onCheckedChange={(on) => onPrefs({ notifyTickets: on })} />
            }
          />
        )}
      </SettingsSection>
      {signedIn && (
        <SettingsSection title="Pull requests">
          <SettingsRow
            title="Send review comments to their agents"
            description="What reviewers say on a pull request you opened goes to the worktree on its branch, as one message, while this window is open. Each comment once."
            control={
              <Switch
                checked={prefs.pullCommentsToAgent}
                onCheckedChange={(on) => onPrefs({ pullCommentsToAgent: on })}
              />
            }
          />
        </SettingsSection>
      )}
    </SettingsPage>
  );
}

/// The trackers your tickets and pull requests are read from. Its own
/// section still, drawn as a page.
export function TrackersPage({
  signedIn,
  onSignIn,
  onTrackers,
}: {
  signedIn: boolean | null;
  onSignIn: () => void;
  onTrackers: (trackers: ConfiguredTracker[]) => void;
}) {
  return (
    <section className="screen in-settings" data-legacy>
      <div className="screen-body">
        <div className="screen-inner">
          <TrackersSection key={String(signedIn)} signedIn={signedIn} onSignIn={onSignIn} focus={false} onChanged={onTrackers} />
        </div>
      </div>
    </section>
  );
}

/// What a new worktree starts with on this server: its config file.
export function SessionsPage({ server }: { server: string }) {
  const [view, setView] = useState<SettingsView | null>(null);
  /// What the choosers are filled from. The same request the create form makes,
  /// reused rather than a second one: the policies and the credential providers
  /// this server has are one fact, and a settings page listing a different set
  /// to the form would be the server disagreeing with itself.
  const [options, setOptions] = useState<NewOptions | null>(null);
  /// The edited copy. `null` until the server has answered, so there is no
  /// moment where this shows a blank field that means "not set" and is really
  /// "not loaded".
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
    // that cannot be reached is exactly when somebody wants to look at this.
    // The two choosers say so where they would have been.
    api.newOptions(server).then(
      (o) => live && setOptions(o),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [server]);

  const edit = (change: Partial<Settings>) => {
    setSaved(false);
    setDraft((current) => (current ? { ...current, ...change } : current));
  };

  /// Blank is not a value, it is the absence of one: an empty branch prefix
  /// means "take the key out of the file and go back to `hura`", not "name
  /// branches `/foo`". The server normalises the same way, and this is here so
  /// the placeholder a person sees and the value that gets sent agree.
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

  /// Whether the draft still differs from the file. Computed rather than
  /// tracked: a save answers with the file re-read, `draft` is replaced by
  /// what came back, and this goes false on its own. Compared as JSON because
  /// both sides are the same generated shape with the same key order.
  const dirty = view !== null && draft !== null && JSON.stringify(draft) !== JSON.stringify(view.settings);

  const providers = draft?.providers ?? [];
  const toggleProvider = (name: string) =>
    edit({ providers: providers.includes(name) ? providers.filter((p) => p !== name) : [...providers, name] });
  const fileBased = draft?.policy && !(options?.policies ?? []).some((p) => p.spec === draft.policy);

  return (
    <>
      <SettingsActions>
        {dirty && <span className="px-1 text-xs text-warning-foreground">Unsaved</span>}
        {saved && !dirty && <span className="px-1 text-xs text-success-foreground">Saved</span>}
        <Button size="xs" disabled={saving || !dirty} onClick={() => void save()}>
          {saving ? "Saving…" : "Save"}
        </Button>
      </SettingsActions>
      <SettingsPage
        notice={
          view ? (
            <>
              This server's defaults, shared with <code>hura new</code>. The create form can override each.{" "}
              {view.present ? "Written to " : "Saving creates "}
              <code>{view.path}</code>.
            </>
          ) : undefined
        }
      >
        {error && <p className="px-3 text-sm text-destructive-foreground sm:px-4">{error}</p>}
        {!view && !error && <Waiting />}
        {view && draft && (
          <>
            <SettingsSection title="New worktrees">
              <SettingsRow
                title="Branch prefix"
                description="What every work branch on this server starts with."
                status={
                  <>
                    e.g.{" "}
                    <code>
                      {(draft.branch_prefix?.trim() || view.default_branch_prefix).replace(/\/+$/, "")}/add-auth
                    </code>
                  </>
                }
                control={
                  <Input
                    className="w-56"
                    value={draft.branch_prefix ?? ""}
                    placeholder={view.default_branch_prefix}
                    spellCheck={false}
                    onChange={(e) => edit({ branch_prefix: text(e.target.value) })}
                  />
                }
              />
              <SettingsRow
                title="Agent"
                description="How a new worktree's agent is drawn."
                control={
                  <div data-legacy className="w-56">
                    {/* Chat is what the server does with the key missing, so
                        it is the option a file that does not mention it shows. */}
                    <Select
                      className="w-full"
                      value={draft.interface ?? "chat"}
                      onChange={(v) => edit({ interface: v === "terminal" ? "terminal" : "chat" })}
                      options={[
                        { value: "chat", label: "Chat", hint: "The Agent SDK, drawn by this window" },
                        { value: "terminal", label: "Terminal", hint: "Claude Code's own, in tmux" },
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
                    value={draft.base ?? ""}
                    placeholder="The remote's default"
                    spellCheck={false}
                    onChange={(e) => edit({ base: text(e.target.value) })}
                  />
                }
              />
              <SettingsRow
                title="Policy"
                description="What the sandbox lets the agent reach."
                status={
                  // A policy set to a file path is a perfectly good answer
                  // and one no chooser can offer, so it is said rather than
                  // silently replaced by the first template in the list.
                  fileBased ? (
                    <>
                      <code>{draft.policy}</code> is a file, not a template.
                    </>
                  ) : undefined
                }
                control={
                  <div data-legacy className="w-56">
                    <Select
                      className="w-full"
                      value={draft.policy ?? ""}
                      onChange={(v) => edit({ policy: text(v) })}
                      options={[
                        {
                          value: "",
                          label: options ? options.default_policy : "The default",
                          hint: "The built-in default",
                        },
                        ...(options?.policies ?? []).map((p) => ({ value: p.spec, label: p.spec, hint: p.summary })),
                        ...(fileBased && draft.policy
                          ? [{ value: draft.policy, label: draft.policy, hint: "From the config file" }]
                          : []),
                      ]}
                    />
                  </div>
                }
              />
              <SettingsRow
                title="Credentials"
                description="The providers a new worktree's sandbox is given. None picked: each repository gets its own guess."
              >
                <div data-legacy className="pills mt-2.5">
                  {!options && <Waiting size="row" />}
                  {options?.providers_error && <p className="error">{options.providers_error}</p>}
                  {options && options.providers.length === 0 && !options.providers_error && (
                    <p className="hint">
                      None configured: add a <code>[credentials.NAME]</code> table to the server's config file.
                    </p>
                  )}
                  {(options?.providers ?? []).map((p) => (
                    <Pill key={p.name} on={providers.includes(p.name)} title={p.kind} onClick={() => toggleProvider(p.name)}>
                      {p.name}
                    </Pill>
                  ))}
                </div>
              </SettingsRow>
            </SettingsSection>

            <SettingsSection
              title="Names"
              description="For a worktree started without a name: a title to show it by, and the slug its branch and sandbox are named from."
            >
              <SettingsRow
                title="Named"
                control={
                  <div data-legacy className="w-56">
                    {/* AI is what the server does with the key missing. */}
                    <Select
                      className="w-full"
                      value={draft.names ?? "ai"}
                      onChange={(v) => edit({ names: v === "rules" ? "rules" : "ai" })}
                      options={[
                        { value: "ai", label: "By Claude Haiku", hint: "From the task, or by the rules if it can't be asked" },
                        { value: "rules", label: "By the rules", hint: "The task's first words. Nothing leaves the server" },
                      ]}
                    />
                  </div>
                }
              />
              <NamingKeyRow
                server={server}
                source={view.naming_key}
                // Only the key's status: the draft is the file's, and a key
                // is not in the file.
                onChange={(naming_key) => setView((v) => (v ? { ...v, naming_key } : v))}
              />
            </SettingsSection>

            <SettingsSection title="Queue">
              <SettingsRow
                title="Run at once"
                description="A worktree added to the queue starts once fewer than this many are creating or running. One scheduled for a time starts then, whatever this says."
                control={
                  <Input
                    className="w-20"
                    type="number"
                    min={1}
                    max={50}
                    // Blank is the server's default, which is what the key
                    // missing from the file means.
                    placeholder="2"
                    value={draft.queue_limit ?? ""}
                    onChange={(e) => {
                      const n = Number.parseInt(e.target.value, 10);
                      edit({ queue_limit: Number.isFinite(n) && n > 0 ? n : null });
                    }}
                  />
                }
              />
            </SettingsSection>

            <SettingsSection title="Server">
              <SettingsRow
                title="Auto-update"
                description={
                  <>
                    Verified, then applied the next time <code>hurad</code> starts.
                  </>
                }
                control={
                  // Absent means on, which is what the server does with the
                  // key missing.
                  <Switch checked={draft.auto_update ?? true} onCheckedChange={(on) => edit({ auto_update: on })} />
                }
              />
            </SettingsSection>
          </>
        )}
      </SettingsPage>
    </>
  );
}

/// The Anthropic API key naming calls Claude with, kept in the server's secret
/// store, so per server, and write-only: this says whether there is one and
/// where it comes from, never what it is. Stored or forgotten at once rather
/// than with Save, because it is not part of the config file Save writes.
function NamingKeyRow({
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
    <SettingsRow
      title="API key"
      description={
        <>
          {source === "stored" && "Stored on this server, which names with it. It can't be shown again."}
          {source === "environment" && (
            <>
              From <code>ANTHROPIC_API_KEY</code> in <code>hurad</code>'s environment. One stored here would be
              used instead.
            </>
          )}
          {source === "none" && (
            <>
              None, so naming uses the <code>claude</code> command on the server if it is there and signed in.
              Stored on the server, never sent back here.
            </>
          )}
        </>
      }
      status={error ? <span className="text-destructive-foreground">{error}</span> : undefined}
      control={
        <>
          <Input
            className="w-56"
            type="password"
            value={value}
            placeholder={source === "stored" ? "Paste to replace it" : "sk-ant-…"}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => setValue(e.target.value)}
          />
          <Button variant="outline" size="sm" disabled={busy || value.trim() === ""} onClick={() => void change(value.trim())}>
            Store
          </Button>
          {source === "stored" && (
            <Button variant="ghost" size="sm" disabled={busy} onClick={() => void change(null)}>
              Forget
            </Button>
          )}
        </>
      }
    />
  );
}
