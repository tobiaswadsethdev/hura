// The rules the gateway is enforcing.
//
// Built from `policy::View`, which is facts rather than text -- so the notices
// below are this renderer's own wording, derived from the same things the
// terminal derives its wording from. The two say the same thing and neither is
// parsing the other's output.

import { useEffect, useState } from "react";

import { useConfirm } from "../Confirm";
import { copy, useContextMenu } from "../ContextMenu";
import { Waiting } from "../Empty";
import { Copy, Forget, Revoke } from "../icons";
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

  return (
    <div className="policy">
      <dl className="facts">
        <dt>template</dt>
        <dd>{view.template ?? "(none recorded)"}</dd>
        <dt>revision</dt>
        <dd>
          {r.settled ? `${r.version} (loaded)` : `${r.version} submitted, ${r.active_version} loaded`}
        </dd>
        {r.source && (
          <>
            <dt>source</dt>
            <dd>{r.source}</dd>
          </>
        )}
        {r.hash && (
          <>
            <dt>hash</dt>
            <dd>{r.hash.slice(0, 12)}</dd>
          </>
        )}
      </dl>

      {!r.settled && (
        <Notice>A newer revision has been submitted. The rules below are the loaded ones.</Notice>
      )}
      {changedSinceCreation && (
        <Notice>
          The network rules have changed since creation, so the template above names what this
          session started from, not what it has now.
        </Notice>
      )}
      {r.source === "global" && (
        <Notice>A gateway-global policy lock is in force and outranks this sandbox's own.</Notice>
      )}

      {view.network === null ? (
        <Notice>The gateway returned no policy payload.</Notice>
      ) : view.network.length === 0 ? (
        <Notice>No network rules: nothing in this sandbox has egress.</Notice>
      ) : (
        view.network.map((rule) => (
          <section key={rule.key} className="rule">
            <h3
              {...menu(() => [
                { label: "Copy rule name", icon: Copy, hint: rule.key, run: () => copy(rule.key) },
              ])}
            >
              {rule.key}
              {rule.name && <span className="alias"> ({rule.name})</span>}
            </h3>
            {rule.binaries.length === 0 ? (
              <Notice>No binaries: this rule grants nothing.</Notice>
            ) : (
              <ul className="binaries">
                {rule.binaries.map((b) => (
                  <li key={b}>{b}</li>
                ))}
              </ul>
            )}
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
          </section>
        ))
      )}

      {view.lists && (
        <section className="rule">
          <h3>global lists</h3>
          <p className="hint">Applied to every new session, so a row may not be in this one.</p>
          <ul className="endpoints">
            {view.lists.allow.length + view.lists.block.length === 0 && (
              <li className="none">
                nothing yet — allow or block from the events pane, and tick “every new session”
              </li>
            )}
            {view.lists.allow.map((a) => (
              <li key={a.endpoint} className="listed" tabIndex={0} {...listed(a.endpoint)}>
                <span className="verb allow">allow</span>
                <span className="host">{a.endpoint}</span>
                <span className={`tag ${a.in_policy ? "" : "no"}`}>
                  {a.in_policy ? "in this policy" : "not in this policy"}
                </span>
                {busy === a.endpoint && <span className="spin row-act" />}
              </li>
            ))}
            {view.lists.block.map((b) => (
              <li key={b.endpoint} className="listed" tabIndex={0} {...listed(b.endpoint)}>
                <span className="verb block">block</span>
                <span className="host">{b.endpoint}</span>
                <span className={`tag ${b.still_in_policy ? "warn" : ""}`}>
                  {b.still_in_policy ? "still in this policy" : "gone from this policy"}
                </span>
                {busy === b.endpoint && <span className="spin row-act" />}
              </li>
            ))}
          </ul>
          <Notice>
            A block removes an endpoint. It is not a deny that outranks an allow, so blocking
            something no policy grants was already the case and changes nothing.
          </Notice>
        </section>
      )}

      {view.locked && (
        <section className="rule">
          <h3>filesystem and process</h3>
          <dl className="facts">
            <dt>workdir</dt>
            <dd>{view.locked.include_workdir ? "included" : "excluded"}</dd>
            {view.locked.read_write.length > 0 && (
              <>
                <dt>read-write</dt>
                <dd>{view.locked.read_write.join(", ")}</dd>
              </>
            )}
            {view.locked.read_only.length > 0 && (
              <>
                <dt>read-only</dt>
                <dd>{view.locked.read_only.join(", ")}</dd>
              </>
            )}
            {view.locked.run_as && (
              <>
                <dt>run as</dt>
                <dd>{view.locked.run_as}</dd>
              </>
            )}
          </dl>
          <Notice>
            These are as submitted, not necessarily as enforced. Landlock is applied at creation,
            and a later change is accepted and reported but never takes effect. Recreate the
            session to change them.
          </Notice>
        </section>
      )}
    </div>
  );
}

function EndpointRow({
  endpoint: e,
  busy,
  ...menu
}: {
  endpoint: Endpoint;
  busy: boolean;
} & ReturnType<ReturnType<typeof useContextMenu>>) {
  const access =
    typeof e.access === "object"
      ? e.access.class
      : e.access === "rules-only"
        ? "(rules only)"
        : "no access granted";

  return (
    <li className="listed" tabIndex={0} {...menu}>
      <span className="host">{e.host_port}</span>
      {e.protocol && <span className="tag">{e.protocol}</span>}
      {e.enforcement && <span className="tag">{e.enforcement}</span>}
      <span className={`tag ${e.access === "none" ? "no" : ""}`}>{access}</span>
      {e.tls === "skip" && <span className="tag warn">tls:skip</span>}
      {busy && <span className="spin row-act" />}
      {e.l7.length > 0 && (
        <ul className="l7">
          {e.l7.map((r, i) => (
            <li key={i}>
              <span className={`verb ${r.allow ? "allow" : "block"}`}>
                {r.allow ? "allow" : "deny"}
              </span>{" "}
              {r.method} {r.path}
            </li>
          ))}
        </ul>
      )}
      {typeof e.access === "object" && e.l7.length > 0 && (
        <Notice>Access and rules together grant the union, not the intersection.</Notice>
      )}
      {e.tls === "terminate" && (
        <Notice>`tls: terminate` is deprecated; termination is automatic now.</Notice>
      )}
    </li>
  );
}

function Notice({ children }: { children: React.ReactNode }) {
  return <p className="notice">{children}</p>;
}
