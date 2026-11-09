// Where your tickets and pull requests come from: each tracker, its token, and
// the form that adds one.
//
// A section of the settings screen, under the account section, because a
// tracker is kept in the hura cloud account, token included, and read there,
// the same on every device. A hura server never sees either. Signed out there
// are none, and this says to sign in. The sidebar on the right is what it
// feeds, and the sidebar's empty state is a button that brings you here.
//
// Every tracker answers the same question, what is assigned to you or created
// by you. What can change once one exists is whether the half you created is
// asked at all, and on Jira, the named JQL filters read beside it, each a
// group of its own in the sidebar.

import { useEffect, useRef, useState } from "react";

import { api, messageOf } from "./api";
import { Empty, Waiting } from "./Empty";
import type { ConfiguredTracker } from "./gen/ConfiguredTracker";
import type { JqlFilter } from "./gen/JqlFilter";
import type { Tracker } from "./gen/Tracker";
import type { TrackerKind } from "./gen/TrackerKind";
import { Forget, Plus, Store, Tracker as TrackerGlyph } from "./icons";
import { openExternal } from "./open";
import { Select } from "./Select";

export function TrackersSection({
  signedIn,
  focus,
  onChanged,
}: {
  /// Whether this computer is signed in to hura cloud, which keeps the
  /// trackers. `null` until the account section has read the session.
  signedIn: boolean | null;
  /// Scrolled to when the screen opens, because the sidebar's "add a tracker"
  /// is what opened it.
  focus: boolean;
  /// The list after a change, so the sidebar can read them again.
  onChanged: (trackers: ConfiguredTracker[]) => void;
}) {
  const [trackers, setTrackers] = useState<ConfiguredTracker[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  // Signed in, what hura cloud can connect through its own sign-in.
  const [connectable, setConnectable] = useState<string[]>([]);
  // The provider being connected in the browser, while the list is watched
  // for it to arrive.
  const [connecting, setConnecting] = useState<string | null>(null);
  const section = useRef<HTMLElement>(null);

  useEffect(() => {
    if (!signedIn) return;
    api.cloudConnectable().then(setConnectable, () => setConnectable([]));
  }, [signedIn]);

  /// Opens the provider's sign-in in the browser, then reads the list every
  /// few seconds until it changes (the tracker connected) or a few minutes
  /// have gone by.
  const connect = async (provider: string) => {
    setError(null);
    try {
      openExternal(await api.cloudConnect(provider));
    } catch (e) {
      setError(messageOf(e));
      return;
    }
    setConnecting(provider);
    const before = JSON.stringify(trackers);
    for (let i = 0; i < 60; i++) {
      await new Promise((r) => setTimeout(r, 3000));
      const list = await api.trackers().catch(() => null);
      if (list && JSON.stringify(list) !== before) {
        setTrackers(list);
        onChanged(list);
        break;
      }
    }
    setConnecting(null);
  };

  useEffect(() => {
    let live = true;
    api
      .trackers()
      .then((v) => live && setTrackers(v))
      .catch((e) => live && setError(messageOf(e)));
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    if (focus) section.current?.scrollIntoView({ block: "start" });
  }, [focus]);

  /// One change to the trackers, and the list it answers with.
  const act = async (what: string, run: () => Promise<ConfiguredTracker[]>): Promise<boolean> => {
    setBusy(what);
    setError(null);
    try {
      const list = await run();
      setTrackers(list);
      onChanged(list);
      return true;
    } catch (e) {
      setError(messageOf(e));
      return false;
    } finally {
      setBusy(null);
    }
  };

  if (!signedIn) {
    return (
      <section ref={section} className="setting-group panel trackers">
        <h3>trackers</h3>
        {signedIn === null ? (
          <Waiting />
        ) : (
          <p className="hint">
            Where the sidebar's tickets and pull requests come from. They are kept in your hura cloud
            account: sign in above to add one.
          </p>
        )}
      </section>
    );
  }

  return (
    <section ref={section} className="setting-group panel trackers">
      <h3>trackers</h3>
      <p className="hint">
        Where the sidebar's tickets and pull requests come from. Kept in your hura account, the same
        on every device signed in to it; tokens are kept there encrypted and never sent back.
      </p>
      {error && <p className="error">{error}</p>}
      {trackers === null && !error && <Waiting />}
      {trackers?.length === 0 && <Empty icon={TrackerGlyph} note="no trackers yet" />}
      {trackers?.map((t) => (
        <TrackerRow
          key={t.source.name}
          tracker={t}
          busy={busy}
          onToken={(value) =>
            act(`tracker:${t.source.name}`, () => api.setTrackerToken(t.source.name, value))
          }
          onForget={() =>
            void act(`tracker:${t.source.name}`, () => api.forgetTracker(t.source.name))
          }
          onEdit={(source) =>
            act(`tracker:${t.source.name}`, () => api.editTracker(t.source.name, source))
          }
        />
      ))}
      {connectable.length > 0 && (
        <div className="setting-actions connect">
          <span className="hint">
            {connecting
              ? `Finish signing in to ${connecting === "github" ? "GitHub" : "Jira"} in the browser…`
              : "Or connect one through its own sign-in, with no token to paste:"}
          </span>
          {connectable.map((p) => (
            <button key={p} className="go" disabled={connecting !== null} onClick={() => void connect(p)}>
              {p === "github" ? "connect GitHub" : "connect Jira"}
            </button>
          ))}
        </div>
      )}
      <NewTracker
        busy={busy !== null}
        onAdd={(tracker, token) => act(`tracker:${tracker.name}`, () => api.addTracker(tracker, token))}
      />
    </section>
  );
}

/// What a tracker is pointed at, in one line.
///
/// Each kind is addressed differently (a GitHub repository, an Azure DevOps
/// organisation and project, a Jira site) and the row says which rather than
/// making three columns that are empty two times out of three.
function target(t: Tracker): string {
  switch (t.kind) {
    case "git-hub":
      return t.repo ?? "every repository";
    case "azure-dev-ops":
      return [t.org, t.project].filter(Boolean).join("/");
    case "jira":
      return [t.site, t.email].filter(Boolean).join(" as ");
  }
}

/// What a kind is called out here.
///
/// The wire value is serde's kebab-case of the Rust variant, `git-hub` and
/// `azure-dev-ops`, and those are not names anybody uses. The config file
/// spells them the way this does, so a row and the file agree.
const KINDS: { value: TrackerKind; label: string; name: string }[] = [
  { value: "jira", label: "jira", name: "Jira" },
  { value: "azure-dev-ops", label: "azure-devops", name: "Azure DevOps" },
  { value: "git-hub", label: "github", name: "GitHub" },
];

function kindLabel(kind: TrackerKind): string {
  return KINDS.find((k) => k.value === kind)?.label ?? kind;
}

/// One tracker, and whether it has a token.
///
/// A tracker with no token is the whole of why its part of the sidebar is
/// empty with a warning on it, so it is said here, with the field to fix it.
function TrackerRow({
  tracker,
  busy,
  onToken,
  onForget,
  onEdit,
}: {
  tracker: ConfiguredTracker;
  busy: string | null;
  /// Store or replace the token. Kept where the tracker is, and never shown
  /// again.
  onToken: (value: string) => Promise<boolean>;
  onForget: () => void;
  /// Save what it shows. The token stays where it is.
  onEdit: (source: Tracker) => Promise<boolean>;
}) {
  const t = tracker.source;
  const working = busy === `tracker:${t.name}`;
  const [token, setToken] = useState("");
  return (
    <div className="row tracker">
      <span className="row-name">{t.name}</span>
      <span className="tracker-kind">{kindLabel(t.kind)}</span>
      <span className="hint" title={target(t)}>
        {target(t)}
      </span>
      {tracker.connected ? (
        <span className={tracker.token_set ? "yes" : "no"}>
          {tracker.token_set ? `connected as ${tracker.connected}` : "connect it again"}
        </span>
      ) : (
        <span className={tracker.token_set ? "yes" : "no"}>
          {tracker.token_set ? "token stored" : "no token"}
        </span>
      )}
      <span className="row-actions">
        {!tracker.connected && (
          <>
            <input
              type="password"
              value={token}
              placeholder={tracker.token_set ? "replace the token" : "paste the token"}
              onChange={(e) => setToken(e.target.value)}
            />
            <button
              className="quiet-icon"
              disabled={working || token.trim().length === 0}
              title={tracker.token_set ? "replace the token" : "store the token"}
              onClick={() => {
                // Cleared either way: a password field holding a token is worth
                // nothing once it is stored, and less if it was refused.
                void onToken(token.trim()).finally(() => setToken(""));
              }}
            >
              <Store aria-label="store the token" />
            </button>
          </>
        )}
        <button
          className="quiet-icon danger"
          disabled={working}
          title="remove this tracker and its token from your account"
          onClick={onForget}
        >
          <Forget aria-label={`forget ${t.name}`} />
        </button>
      </span>
      {!tracker.token_set && !tracker.connected && (
        <p className="problem">No token yet; paste one above and this tracker starts answering.</p>
      )}
      {!tracker.token_set && tracker.connected && (
        <p className="problem">Its sign-in ran out or was taken back; connect it again below.</p>
      )}
      <Shows source={t} working={working} onSave={onEdit} />
    </div>
  );
}

/// What a tracker puts in the sidebar beyond what is assigned to you: the
/// tickets you created, and on Jira, its JQL filters.
///
/// Edited here and saved as one, because a filter half typed is not one to
/// read on the next timer.
function Shows({
  source,
  working,
  onSave,
}: {
  source: Tracker;
  working: boolean;
  onSave: (source: Tracker) => Promise<boolean>;
}) {
  const [created, setCreated] = useState(!source.hide_created);
  const [filters, setFilters] = useState<JqlFilter[]>(source.jql_filters);

  // A save comes back as a new list: start again from what was kept.
  useEffect(() => {
    setCreated(!source.hide_created);
    setFilters(source.jql_filters);
  }, [source]);

  const jira = source.kind === "jira";
  const changed =
    created === source.hide_created ||
    JSON.stringify(filters) !== JSON.stringify(source.jql_filters);
  const ready = filters.every((f) => f.name.trim() && f.jql.trim());
  const change = (i: number, edit: Partial<JqlFilter>) =>
    setFilters(filters.map((f, j) => (j === i ? { ...f, ...edit } : f)));

  return (
    <div className="tracker-shows">
      <label className="tick">
        <input type="checkbox" checked={created} onChange={(e) => setCreated(e.target.checked)} />
        <span>show the tickets I created</span>
      </label>
      {jira && (
        <>
          <p className="hint">
            Filters: each is a group of its own in the sidebar, read beside your tickets and
            notified like them, with a ticket new to it said too. The JQL is sent as written, so
            say <code>statusCategory != Done</code> if done tickets should stay out.
          </p>
          {filters.map((f, i) => (
            <div key={i} className="tracker-filter">
              <input
                value={f.name}
                placeholder="name"
                aria-label="the filter's name"
                onChange={(e) => change(i, { name: e.target.value })}
              />
              <input
                className="jql"
                value={f.jql}
                placeholder="project = PROJ AND status = Ready ORDER BY updated DESC"
                aria-label="its JQL"
                spellCheck={false}
                onChange={(e) => change(i, { jql: e.target.value })}
              />
              <button
                className="quiet-icon danger"
                title="remove this filter"
                onClick={() => setFilters(filters.filter((_, j) => j !== i))}
              >
                <Forget aria-label={`remove ${f.name || "this filter"}`} />
              </button>
            </div>
          ))}
          <button
            className="quiet add-filter"
            onClick={() => setFilters([...filters, { name: "", jql: "" }])}
          >
            <Plus /> add a filter
          </button>
        </>
      )}
      {changed && (
        <div className="setting-actions">
          <span className="unsaved">{ready ? "not saved" : "every filter needs a name and JQL"}</span>
          <button
            className="go"
            disabled={working || !ready}
            onClick={() =>
              void onSave({
                ...source,
                hide_created: !created,
                jql_filters: filters.map((f) => ({ name: f.name.trim(), jql: f.jql.trim() })),
              })
            }
          >
            save
          </button>
        </div>
      )}
    </div>
  );
}

/// Add a tracker: where it points and the token, as labelled fields.
///
/// Only what each kind needs, the required ones marked.
function NewTracker({
  busy,
  onAdd,
}: {
  busy: boolean;
  /// Answers whether it was taken. A rejected entry keeps what was typed:
  /// the reason is usually one field, and re-typing the rest would be the
  /// punishment for a typo.
  onAdd: (tracker: Tracker, token: string) => Promise<boolean>;
}) {
  const [kind, setKind] = useState<TrackerKind>("jira");
  const [name, setName] = useState("");
  const [token, setToken] = useState("");
  const [repo, setRepo] = useState("");
  const [org, setOrg] = useState("");
  const [project, setProject] = useState("");
  const [site, setSite] = useState("");
  const [email, setEmail] = useState("");

  const filled = (v: string) => v.trim().length > 0;
  const blank = (v: string) => (filled(v) ? v.trim() : null);
  // What each kind cannot work without: the same rule the store applies, so
  // the button says so before the store has to.
  const ready =
    filled(token) &&
    (kind !== "jira" || (filled(site) && filled(email))) &&
    (kind !== "azure-dev-ops" || (filled(org) && filled(project)));

  const tracker = (): Tracker => ({
    kind,
    // Blank means "call it after its kind".
    name: name.trim(),
    repo: kind === "git-hub" ? blank(repo) : null,
    org: kind === "azure-dev-ops" ? blank(org) : null,
    project: kind === "azure-dev-ops" ? blank(project) : null,
    site: kind === "jira" ? blank(site) : null,
    email: kind === "jira" ? blank(email) : null,
    hide_created: false,
    jql_filters: [],
  });

  const reset = () => {
    setName("");
    setRepo("");
    setOrg("");
    setProject("");
    setSite("");
    setEmail("");
  };

  return (
    <div className="setting-group new-tracker">
      <h4>add a tracker</h4>
      <label>
        <span>tracker</span>
        <Select
          value={kind}
          onChange={(v) => setKind(v as TrackerKind)}
          options={KINDS.map((k) => ({ value: k.value, label: k.name }))}
        />
      </label>

      {kind === "jira" && (
        <>
          <Field label="site" required>
            <input
              value={site}
              placeholder="https://your-org.atlassian.net"
              onChange={(e) => setSite(e.target.value)}
            />
          </Field>
          <Field label="email" required>
            <input
              value={email}
              placeholder="you@example.com"
              onChange={(e) => setEmail(e.target.value)}
            />
          </Field>
          <p className="hint indent">The Atlassian account the token belongs to.</p>
        </>
      )}
      {kind === "azure-dev-ops" && (
        <>
          <Field label="organisation" required>
            <input value={org} placeholder="your-org" onChange={(e) => setOrg(e.target.value)} />
          </Field>
          <Field label="project" required>
            <input
              value={project}
              placeholder="YourProject"
              onChange={(e) => setProject(e.target.value)}
            />
          </Field>
        </>
      )}
      {kind === "git-hub" && (
        <>
          <Field label="repository">
            <input value={repo} placeholder="owner/name" onChange={(e) => setRepo(e.target.value)} />
          </Field>
          <p className="hint indent">Leave it empty for every repository the token can see.</p>
        </>
      )}

      <Field label={TOKEN_LABEL[kind]} required>
        <input type="password" value={token} onChange={(e) => setToken(e.target.value)} />
      </Field>
      <p className="hint indent">
        {TOKEN_HINT[kind]} Kept in your hura account, encrypted.
      </p>

      <Field label="name">
        <input value={name} placeholder={kindLabel(kind)} onChange={(e) => setName(e.target.value)} />
      </Field>
      <p className="hint indent">What this tracker is called here. Only needed for a second one.</p>

      <div className="setting-actions">
        <span className="hint">* required</span>
        <button
          className="go"
          disabled={busy || !ready}
          onClick={() => {
            void onAdd(tracker(), token.trim()).then((added) => {
              // The token goes either way: a password field holding one is
              // worth nothing once it is stored, and less if it was refused.
              setToken("");
              if (added) reset();
            });
          }}
        >
          add tracker
        </button>
      </div>
    </div>
  );
}

/// A labelled field in the settings grid, with a mark when it is required.
function Field({
  label,
  required = false,
  children,
}: {
  label: string;
  required?: boolean;
  children: React.ReactNode;
}) {
  return (
    <label>
      <span>
        {label}
        {required && <span className="required"> *</span>}
      </span>
      {children}
    </label>
  );
}

/// What each kind calls its token, and where to get one.
const TOKEN_LABEL: Record<TrackerKind, string> = {
  jira: "API token",
  "azure-dev-ops": "access token",
  "git-hub": "token",
};

const TOKEN_HINT: Record<TrackerKind, string> = {
  jira: "Created at id.atlassian.com → Security → API tokens.",
  "azure-dev-ops":
    "A personal access token with Work Items (read) for the tickets and Code (read) for the pull requests, or Code (read & write) to comment and vote on them.",
  "git-hub":
    "A token that can read issues and pull requests, and write pull requests to comment and review.",
};
