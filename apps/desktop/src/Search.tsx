// Every conversation this machine has seen, searched.
//
// What is searched is kept by the Rust side as it passes on the way to the
// panes -- see `hura_client::history` -- so it includes conversations that
// have ended and sessions that have been removed, whose transcripts are
// nowhere else. Words are looked for in what was said; `file:` in the files a
// tool call touched and `tool:` in the tool's name.
//
// A result in a conversation that still exists opens it, at the message, in
// the workspace. One that only this machine still has is read here instead,
// from what was kept: the same rows a pane draws, with nothing to send.

import { useEffect, useMemo, useRef, useState } from "react";

import { api, type HistoryFound, type HistoryHit, messageOf } from "./api";
import { Empty } from "./Empty";
import type { ChatEntry } from "./gen/ChatEntry";
import type { Session } from "./gen/Session";
import { Back, Chat, Find, Working } from "./icons";
import { Found, Items, relative } from "./chat/Items";
import { build, group, locate } from "./chat/transcript";
import { Screen } from "./Screen";

/// What was last searched for, and what it found, for the screen to come back
/// to: leaving to read a result and returning is the normal way to use it.
let last: { query: string; found: HistoryFound | null } = { query: "", found: null };

/// The words a query looks for, without its `file:` and `tool:`, for marking
/// them in a snippet. `parse` in `history.rs` is the one that searches.
function words(query: string): string[] {
  const out: string[] = [];
  for (const m of query.matchAll(/"([^"]*)"?|(\S+)/g)) {
    const w = (m[1] ?? m[2] ?? "").toLowerCase();
    if (w && !/^(file|tool):./.test(w)) out.push(w);
  }
  return out;
}

/// A snippet with the words looked for marked.
function Marked({ text, words }: { text: string; words: string[] }) {
  if (words.length === 0) return <>{text}</>;
  const escaped = words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const parts = text.split(new RegExp(`(${escaped.join("|")})`, "gi"));
  return <>{parts.map((p, i) => (i % 2 === 1 ? <mark key={i}>{p}</mark> : p))}</>;
}

const ROLES: Record<HistoryHit["role"], string> = {
  you: "You",
  claude: "Claude",
  agent: "Subagent",
  shell: "!",
  error: "Error",
};

function when(at: number): string {
  const d = new Date(at);
  const today = new Date();
  return d.toDateString() === today.toDateString()
    ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString([], {
        day: "numeric",
        month: "short",
        year: d.getFullYear() === today.getFullYear() ? undefined : "numeric",
      });
}

function convLabel(conv: string): string {
  return conv === "agent" ? "Agent" : conv;
}

export function SearchScreen({
  server,
  sessions,
  onOpen,
  onClose,
}: {
  /// The server the window is on, whose session list is `sessions`.
  server: string | null;
  sessions: Session[];
  /// Open a result in the workspace, in a conversation that still exists.
  onOpen: (hit: HistoryHit) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState(last.query);
  const [found, setFound] = useState<HistoryFound | null>(last.found);
  const [error, setError] = useState<string | null>(null);
  const [reading, setReading] = useState<HistoryHit | null>(null);
  const field = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!reading) field.current?.focus();
  }, [reading]);

  // As it is typed, a moment after the last key: every search reads whatever
  // was appended since the one before, which is nothing to do per keystroke.
  useEffect(() => {
    if (!query.trim()) {
      setFound(null);
      last = { query, found: null };
      return;
    }
    let live = true;
    const timer = setTimeout(() => {
      api.historySearch(query).then(
        (f) => {
          if (!live) return;
          setFound(f);
          setError(null);
          last = { query, found: f };
        },
        (e) => live && setError(messageOf(e)),
      );
    }, 150);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [query]);

  const marked = useMemo(() => words(query), [query]);

  /// Whether a result can be opened where it happened. A session on another
  /// server is taken at its word; one on this server has to be in its list,
  /// since it may have been removed from another window or the command line.
  const live = (hit: HistoryHit) =>
    !hit.removed && !hit.ended && (hit.server !== server || sessions.some((s) => s.name === hit.session));

  if (reading) {
    return (
      <Screen icon={Find} title="Search" onClose={onClose}>
        <Kept hit={reading} onBack={() => setReading(null)} />
      </Screen>
    );
  }

  return (
    <Screen icon={Find} title="Search" onClose={onClose}>
      <label className="history-field">
        <Find aria-hidden />
        <input
          ref={field}
          className="prose"
          value={query}
          placeholder="Search every conversation"
          aria-label="Search every conversation"
          onChange={(e) => setQuery(e.target.value)}
        />
      </label>
      <p className="hint history-help">
        Words are looked for in what was said. <code>file:auth.rs</code> finds the tool calls that touched a file,{" "}
        <code>tool:Bash</code> those of one tool, and <code>"two words"</code> a phrase.
      </p>
      {error && <p className="error">{error}</p>}
      {found && found.hits.length === 0 && <Empty icon={Find} note="Nothing found" />}
      {found && found.hits.length > 0 && (
        <ul className="history-hits">
          {found.hits.map((hit) => (
            <li key={`${hit.server}\n${hit.session_key}\n${hit.conv_key}\n${hit.seq}`}>
              <button
                className="history-hit"
                onClick={() => (live(hit) ? onOpen(hit) : setReading(hit))}
                title={live(hit) ? "Open the conversation here" : "Read what was kept of it"}
              >
                <span className="history-where">
                  <b>{hit.title ?? hit.session}</b>
                  <span>{convLabel(hit.conv)}</span>
                  {hit.project && <span>{hit.project}</span>}
                  {hit.server !== server && <span>{hit.server}</span>}
                  {hit.removed && <span className="history-gone">removed</span>}
                  {!hit.removed && hit.ended && <span className="history-gone">ended</span>}
                  <time title={new Date(hit.at).toLocaleString()}>{when(hit.at)}</time>
                </span>
                <span className="history-said">
                  <span className={`history-role ${hit.role}`}>{ROLES[hit.role]}</span>
                  <Marked text={hit.snippet} words={marked} />
                </span>
                {(hit.tools.length > 0 || hit.files.length > 0) && (
                  <span className="history-used">
                    {hit.tools.map((t) => (
                      <span key={`t${t}`} className="history-tool">
                        {t}
                      </span>
                    ))}
                    {hit.files.map((f) => (
                      <span key={`f${f}`} className="history-file" title={f}>
                        {relative(f)}
                      </span>
                    ))}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
      {found?.more && <p className="hint">Only the newest {found.hits.length} are shown; say more to find fewer.</p>}
    </Screen>
  );
}

/// A conversation that exists only here, read from what was kept, at the
/// message found. Its pictures were not kept, so a picture it sent says so
/// rather than drawing.
function Kept({ hit, onBack }: { hit: HistoryHit; onBack: () => void }) {
  const [entries, setEntries] = useState<ChatEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    api.historyTranscript(hit.server, hit.session_key, hit.conv_key).then(
      (e) => live && setEntries(e),
      (e) => live && setError(messageOf(e)),
    );
    return () => {
      live = false;
    };
  }, [hit]);
  const items = useMemo(() => (entries ? group(build(entries).items, false) : []), [entries]);
  const located = useMemo(() => {
    const at = locate(items, hit.seq);
    return at && { ...at, nonce: hit.seq };
  }, [items, hit.seq]);

  return (
    <div className="history-kept">
      <div className="history-kept-head">
        <button className="quiet" onClick={onBack}>
          <Back />
          Results
        </button>
        <span className="history-where">
          <b>{hit.title ?? hit.session}</b>
          <span>{convLabel(hit.conv)}</span>
          <span className="history-gone">
            {hit.removed ? "The session has been removed" : "The conversation has ended"}; this is what this
            computer kept of it
          </span>
        </span>
      </div>
      {error && <p className="error">{error}</p>}
      {!entries && !error && <Empty icon={Working} />}
      {entries && entries.length === 0 && <Empty icon={Chat} note="Nothing was kept of it" />}
      <Found.Provider value={located}>
        <div className="chat-log history-log">
          <Items items={items} />
        </div>
      </Found.Provider>
    </div>
  );
}
