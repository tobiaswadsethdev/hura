// The about screen: which versions are talking to each other, and whether a
// newer one is on offer.
//
// Three versions matter and they move independently. This window updates from
// the release page, `hurad` updates itself on the server, and the agent's
// version is whatever the sandbox image carries -- so "which hura is this" is
// three answers, and a mismatch between the first two is the usual reason a
// new button does nothing.
//
// The update check is here, as a button, so that learning about a release does
// not need a restart. See `Update.tsx` for the state behind it.

import { useEffect, useState } from "react";

import { api, messageOf, type About as AboutView } from "./api";
import {
  About as AboutGlyph,
  Agent,
  Busy,
  Clean,
  Desktop,
  Docs,
  Elsewhere,
  Heads,
  Notes,
  Refresh,
  Report,
  Servers,
  Toolchain,
  Upgrade,
} from "./icons";
import { Screen } from "./Screen";
import { checkForUpdate, installUpdate, percent, useUpdate } from "./Update";

const REPO = "https://github.com/tobiaswadsethdev/hura";

export function AboutScreen({
  server,
  agent,
  onClose,
}: {
  server: string | null;
  /// The selected session's agent, as its status line reports it. Absent
  /// until one has run.
  agent: { version: string | null; model: string | null } | null;
  onClose: () => void;
}) {
  const [about, setAbout] = useState<AboutView | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api
      .about(server)
      .then((a) => live && setAbout(a))
      .catch((e) => live && setError(messageOf(e)));
    return () => {
      live = false;
    };
  }, [server]);

  const differs =
    about?.server_version != null && about.server_version !== about.desktop;

  return (
    <Screen icon={AboutGlyph} title="about" narrow onClose={onClose}>
      <div className="about-hero">
        <span className="about-mark">hura</span>
        {about && <span className="version-chip">{about.desktop}</span>}
      </div>
      {error && <p className="error">{error}</p>}

      <ul className="about-rows">
        <Row icon={Desktop} label="desktop" value={about?.desktop} />
        {server && (
          <Row
            icon={Servers}
            label={`hurad · ${server}`}
            value={about?.server_version}
            note={
              about?.server_error ? (
                <span className="error" title={about.server_error}>
                  unreachable
                </span>
              ) : differs ? (
                <span className="about-warn" title="the window and the server are on different releases">
                  <Heads /> differs
                </span>
              ) : null
            }
          />
        )}
        {agent?.version && (
          <Row icon={Agent} label="claude code" value={agent.version} note={agent.model} />
        )}
      </ul>

      <Updates supported={about?.updater ?? null} />

      <nav className="about-links">
        <a href={`${REPO}/releases`} target="_blank" rel="noreferrer">
          <Notes /> releases
        </a>
        <a href={`${REPO}/tree/main/docs`} target="_blank" rel="noreferrer">
          <Docs /> docs
        </a>
        <a href={`${REPO}/issues/new/choose`} target="_blank" rel="noreferrer">
          <Report /> report a bug
        </a>
      </nav>
    </Screen>
  );
}

function Row({
  icon: Icon,
  label,
  value,
  note,
}: {
  icon: React.ComponentType;
  label: string;
  value: string | null | undefined;
  note?: React.ReactNode;
}) {
  return (
    <li>
      <Icon />
      <span className="about-label">{label}</span>
      <code className="about-value">{value ?? "—"}</code>
      {note && <span className="about-note">{note}</span>}
    </li>
  );
}

/// The update card. `supported` is the Rust side's answer, `null` while it is
/// still coming.
function Updates({ supported }: { supported: boolean | null }) {
  const p = useUpdate();
  // Re-rendered now and then so "checked 2m ago" does not stay "just now".
  const [, tick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(id);
  }, []);

  if (supported === false || p.at === "unsupported") {
    return (
      <section className="update-card">
        <Toolchain />
        <div className="update-text">
          <b>built from source</b>
          <span className="hint">this platform has no installer — pull and rebuild to update</span>
        </div>
        <a className="quiet" href={`${REPO}/blob/main/docs/install.md`} target="_blank" rel="noreferrer">
          <Elsewhere /> how
        </a>
      </section>
    );
  }

  const checkedAt = "checkedAt" in p ? p.checkedAt : null;
  const checkButton = (
    <button className="quiet" disabled={p.at === "checking"} onClick={() => void checkForUpdate()}>
      <Refresh className={p.at === "checking" ? "turning" : undefined} /> check
    </button>
  );

  if (p.at === "found") {
    const { found } = p;
    return (
      <section className="update-card found">
        <Upgrade />
        <div className="update-text">
          <b>{found.version} is available</b>
          <span className="hint">
            {found.date ? `released ${new Date(found.date).toLocaleDateString()} · ` : ""}
            the window restarts to install
          </span>
          {found.notes && (
            <details className="update-notes">
              <summary>what's new</summary>
              <pre>{found.notes}</pre>
            </details>
          )}
        </div>
        <button className="go" onClick={() => void installUpdate()}>
          <Upgrade /> install &amp; restart
        </button>
      </section>
    );
  }

  if (p.at === "installing") {
    const pct = percent(p.got, p.total);
    return (
      <section className="update-card found">
        <Busy className="turning" />
        <div className="update-text">
          <b>installing {p.version}</b>
          <span className="hint">{pct ? `downloaded ${pct}` : "downloading…"} · restarts on its own</span>
          <span className="update-progress">
            <span style={{ width: pct ?? "0%" }} />
          </span>
        </div>
      </section>
    );
  }

  if (p.at === "failed") {
    return (
      <section className="update-card bad">
        <Heads />
        <div className="update-text">
          <b>{p.version ? `could not install ${p.version}` : "could not check for updates"}</b>
          <span className="hint" title={p.why}>
            {p.why}
          </span>
        </div>
        {p.version && (
          <a
            className="quiet"
            href={`${REPO}/releases/tag/v${p.version}`}
            target="_blank"
            rel="noreferrer"
          >
            <Elsewhere /> download
          </a>
        )}
        {checkButton}
      </section>
    );
  }

  return (
    <section className="update-card">
      {p.at === "checking" ? <Busy className="turning" /> : <Clean />}
      <div className="update-text">
        <b>{p.at === "checking" ? "checking…" : p.at === "current" ? "up to date" : "not checked yet"}</b>
        {checkedAt && <span className="hint">checked {ago(checkedAt)}</span>}
      </div>
      {checkButton}
    </section>
  );
}

function ago(at: number): string {
  const s = Math.round((Date.now() - at) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return new Date(at).toLocaleDateString();
}
