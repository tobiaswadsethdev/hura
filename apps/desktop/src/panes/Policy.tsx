// The rules the gateway is enforcing.
//
// Built from `policy::View`, which is facts rather than text -- so the notices
// below are this renderer's own wording, derived from the same things the
// terminal derives its wording from. The two say the same thing and neither is
// parsing the other's output.
//
// Drawn for the question a reader brings to it, which is "can it reach X":
// each rule a card of hosts with what they grant on the right, the programs
// it lets out at its foot by name, and the attributes that are the same on
// every endpoint said once in a tooltip instead of three pills a row. The
// caveats are one line each, with a glyph for which are warnings.

import { useEffect, useState } from "react";

import { useConfirm } from "../Confirm";
import { copy, useContextMenu } from "../ContextMenu";
import { Waiting } from "../Empty";
import { Copy, Forget, Grant, Heads, Policy as PolicyGlyph, Program, Record, Revoke } from "../icons";
import { useFetch } from "../useFetch";
import { api, messageOf } from "../api";
import type { View } from "../gen/View";
import type { Endpoint } from "../gen/Endpoint";

/// What a row can ask for. Each answers with the policy re-read.
type Change = { block: (endpoint: string) => void; unlist: (endpoint: string) => void };

export function PolicyPane({ server, name }: { server: string; name: string }) {
  const { data, error } = useFetch(() => api.policy(server, name), [server, name]);
  // What the last change answered with, which outranks the first read.
  const [changed, setChanged] = useState<View | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const { ask, dialog } = useConfirm();

  useEffect(() => setChanged(null), [server, name]);

  const run = async (endpoint: string, change: Promise<View>) => {
    setBusy(endpoint);
    setFailed(null);
    try {
      setChanged(await change);
    } catch (e) {
      setFailed(messageOf(e));
    } finally {
      setBusy(null);
    }
  };

  const change: Change = {
    block: (endpoint) =>
      ask({
        title: "Block this endpoint?",
        body: (
          <>
            <code>{endpoint}</code> is removed from this sandbox for every binary, and anything
            that depends on it stops working. Every rule naming it loses it.
          </>
        ),
        confirm: "block",
        onConfirm: () => void run(endpoint, api.block(server, name, endpoint, false)),
      }),
    unlist: (endpoint) => void run(endpoint, api.unlist(server, name, endpoint)),
  };

  const view = changed ?? data;
  if (error && !view) return <p className="error">{error}</p>;
  if (!view) return <Waiting />;
  return (
    <>
      {failed && <p className="error">{failed}</p>}
      <Policy view={view} change={change} busy={busy} />
      {dialog}
    </>
  );
}

function Policy({ view, change, busy }: { view: View; change: Change; busy: string | null }) {
  const menu = useContextMenu();
  /// A list entry's menu. Taking it off only changes what the *next* session
  /// starts with; sessions already running keep what it gave them.
  const listed = (endpoint: string) =>
    menu(() => [
      {
        label: "Remove from the list",
        icon: Forget,
        hint: "new sessions only",
        disabled: busy !== null,
        run: () => change.unlist(endpoint),
      },
      "separator",
      { label: "Copy endpoint", icon: Copy, hint: endpoint, run: () => copy(endpoint) },
    ]);

  const r = view.revision;
  const changedSinceCreation = r.version > 1 && view.template !== null;
  const endpoints = view.network?.reduce((n, rule) => n + rule.endpoints.length, 0) ?? 0;

  return (
    <div className="policy">
      {/* Which policy, and which revision of it, as one line: the template
          the session started from and the revision the gateway has loaded.
          The hash and where it came from are the tooltip -- worth having,
          not worth a row each. */}
      <header className="policy-head">
        <PolicyGlyph className="policy-glyph" />
        <span className="policy-name">{view.template ?? "no template recorded"}</span>
        <span
          className="pill fixed"
          title={[r.source && `source: ${r.source}`, r.hash && `hash: ${r.hash.slice(0, 12)}`]
            .filter(Boolean)
            .join("\n")}
        >
          rev {r.active_version}
        </span>
      </header>

      {!r.settled && (
        <Notice warn>
          Revision {r.version} is submitted but not loaded yet; below is {r.active_version}.
        </Notice>
      )}
      {changedSinceCreation && (
        <Notice>The rules have changed since creation, so the template is only where they started.</Notice>
      )}
      {r.source === "global" && (
        <Notice warn>A gateway-global policy lock is in force and outranks this sandbox's own.</Notice>
      )}

      <section>
        <h4>
          network
          {view.network && view.network.length > 0 && (
            <span className="count">
              {view.network.length} {view.network.length === 1 ? "rule" : "rules"} · {endpoints}{" "}
              {endpoints === 1 ? "endpoint" : "endpoints"}
            </span>
          )}
        </h4>
        {view.network === null ? (
          <Notice warn>The gateway returned no policy payload.</Notice>
        ) : view.network.length === 0 ? (
          <Notice>No network rules: nothing in this sandbox has egress.</Notice>
        ) : (
          view.network.map((rule) => (
            // A card per rule: what it lets out, and who it lets out. The
            // programs are at the foot rather than the head, because what a
            // reader is looking for in a policy is a host -- and they are
            // names, not paths: `/usr/lib/git-core/git-remote-https` is the
            // tooltip, `git-remote-https` is what is read.
            <article key={rule.key} className="rule">
              <h3
                title={rule.key}
                {...menu(() => [
                  { label: "Copy rule name", icon: Copy, hint: rule.key, run: () => copy(rule.key) },
                ])}
              >
                {rule.name ?? rule.key}
              </h3>
              <ul className="endpoints">
                {rule.endpoints.map((e) => (
                  <EndpointRow
                    key={e.host_port}
                    endpoint={e}
                    busy={busy === e.host_port}
                    {...menu(() => [
                      {
                        label: "Block…",
                        icon: Revoke,
                        danger: true,
                        disabled: busy !== null,
                        run: () => change.block(e.host_port),
                      },
                      "separator",
                      {
                        label: "Copy endpoint",
                        icon: Copy,
                        hint: e.host_port,
                        run: () => copy(e.host_port),
                      },
                    ])}
                  />
                ))}
              </ul>
              <div className="programs">
                <Program />
                {rule.binaries.length === 0 ? (
                  <span className="none">no programs, so this rule grants nothing</span>
                ) : (
                  rule.binaries.map((b) => (
                    <span key={b} className="program" title={b}>
                      {b.slice(b.lastIndexOf("/") + 1)}
                    </span>
                  ))
                )}
              </div>
            </article>
          ))
        )}
      </section>

      {view.lists && (
        <section>
          <h4 title="Applied to every new session, so an entry may not be in this one.">every new session</h4>
          <ul className="lists">
            {view.lists.allow.length + view.lists.block.length === 0 && (
              <li className="none">
                none yet. Allow or block from the events pane with “every new session” ticked.
              </li>
            )}
            {view.lists.allow.map((a) => (
              <li key={a.endpoint} className="listed" tabIndex={0} {...listed(a.endpoint)}>
                <Grant className="allow" aria-label="allowed" />
                <span className="host">{a.endpoint}</span>
                {busy === a.endpoint ? (
                  <span className="spin" />
                ) : (
                  <span className={`state${a.in_policy ? "" : " off"}`}>
                    {a.in_policy ? "in this policy" : "not in this one"}
                  </span>
                )}
              </li>
            ))}
            {view.lists.block.map((b) => (
              <li key={b.endpoint} className="listed" tabIndex={0} {...listed(b.endpoint)}>
                <Revoke className="block" aria-label="blocked" />
                <span className="host">{b.endpoint}</span>
                {busy === b.endpoint ? (
                  <span className="spin" />
                ) : (
                  <span className={`state${b.still_in_policy ? " warn" : " off"}`}>
                    {b.still_in_policy ? "still in this policy" : "gone from this one"}
                  </span>
                )}
              </li>
            ))}
          </ul>
          <Notice>
            A block removes an endpoint; it is not a deny that outranks an allow, so blocking one
            nothing grants changes nothing.
          </Notice>
        </section>
      )}

      {view.locked && (
        <section>
          <h4>filesystem and process</h4>
          <div className="fact">
            <span className="fact-label">read-write</span>
            <Paths paths={view.locked.read_write} />
          </div>
          <div className="fact">
            <span className="fact-label">read-only</span>
            <Paths paths={view.locked.read_only} />
          </div>
          <div className="fact">
            <span className="fact-label">workdir</span>
            <span className="fact-value mono">{view.locked.include_workdir ? "included" : "excluded"}</span>
          </div>
          {view.locked.run_as && (
            <div className="fact">
              <span className="fact-label">run as</span>
              <span className="fact-value mono">{view.locked.run_as}</span>
            </div>
          )}
          {/* Landlock is applied at creation, and a later change is accepted
              and reported but never takes effect -- so these are as
              submitted, not necessarily as enforced. */}
          <Notice>Fixed when the sandbox was created. Recreate the session to change them.</Notice>
        </section>
      )}
    </div>
  );
}

function Paths({ paths }: { paths: string[] }) {
  return (
    <span className="fact-value fact-chips">
      {paths.length === 0 ? (
        <span className="none">none</span>
      ) : (
        paths.map((p) => (
          <span key={p} className="pill fixed">
            {p}
          </span>
        ))
      )}
    </span>
  );
}

/// What an endpoint grants, as one word. `rules` is an endpoint with no class
/// of its own, only the method-and-path rules under it.
function accessOf(e: Endpoint): string {
  if (typeof e.access === "object") return e.access.class;
  return e.access === "rules-only" ? "rules" : "none";
}

function EndpointRow({
  endpoint: e,
  busy,
  ...menu
}: {
  endpoint: Endpoint;
  busy: boolean;
} & ReturnType<ReturnType<typeof useContextMenu>>) {
  const colon = e.host_port.lastIndexOf(":");
  const host = colon > 0 ? e.host_port.slice(0, colon) : e.host_port;
  const port = colon > 0 ? e.host_port.slice(colon) : "";
  const access = accessOf(e);

  return (
    <li
      className="listed"
      tabIndex={0}
      // How it is inspected, which is the same for nearly every endpoint --
      // `rest`, `enforce` -- and was a pill each on every row. Said here, and
      // on the row only when it is the exception.
      title={[e.protocol && `protocol: ${e.protocol}`, e.enforcement && `enforcement: ${e.enforcement}`]
        .filter(Boolean)
        .join("\n") || "layer 4: inspected by host and port only"}
      {...menu}
    >
      <span className="endpoint-line">
        <span className="host">
          {host}
          <span className="port">{port}</span>
        </span>
        {e.enforcement && e.enforcement !== "enforce" && <span className="tag warn">{e.enforcement}</span>}
        {e.tls === "skip" && (
          <span className="tag warn" title="TLS is not inspected: the rules below cannot see paths">
            tls skip
          </span>
        )}
        {busy ? <span className="spin" /> : <span className={`access${access === "none" ? " no" : ""}`}>{access}</span>}
      </span>
      {e.l7.length > 0 && (
        <ul className="l7">
          {e.l7.map((r, i) => (
            <li key={i} className={r.allow ? "" : "deny"}>
              <span className="method">{r.method}</span>
              <span className="path">{r.path}</span>
              {!r.allow && <span className="deny-word">deny</span>}
            </li>
          ))}
        </ul>
      )}
      {typeof e.access === "object" && e.l7.length > 0 && (
        <Notice>Access and rules together grant the union, not the intersection.</Notice>
      )}
      {e.tls === "terminate" && <Notice>`tls: terminate` is deprecated; termination is automatic now.</Notice>}
    </li>
  );
}

/// A caveat about what is above it. One line in the dock's quiet voice, with
/// the glyph saying which kind: something to know, or something to watch.
function Notice({ children, warn }: { children: React.ReactNode; warn?: boolean }) {
  const Glyph = warn ? Heads : Record;
  return (
    <p className={`notice${warn ? " warn" : ""}`}>
      <Glyph />
      <span>{children}</span>
    </p>
  );
}
