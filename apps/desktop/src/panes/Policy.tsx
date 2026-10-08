// The rules on a session's sandbox.
//
// Built from `policy::View`, which is facts rather than text, so the notices
// below are this renderer's own wording, derived from the same things the
// terminal derives its wording from.
//
// Drawn for the question a reader brings to it, which is "can it reach X": a
// card per host, and under it each rule that names the host, either every
// request or a method and a path, with a deny marked as one. Rules are for the
// whole sandbox rather than for one program in it, which is said once at the
// foot rather than on every card.

import { useEffect, useState } from "react";

import { useConfirm } from "../Confirm";
import { copy, useContextMenu } from "../ContextMenu";
import { Waiting } from "../Empty";
import { Copy, Forget, Grant, Heads, Policy as PolicyGlyph, Record, Revoke } from "../icons";
import { useFetch } from "../useFetch";
import { api, messageOf } from "../api";
import type { View } from "../gen/View";
import type { Rule } from "../gen/Rule";

/// What a row can ask for. Each answers with the policy re-read.
type Change = {
  block: (endpoint: string) => void;
  remove: (rule: Rule, host: string) => void;
  unlist: (endpoint: string) => void;
};

export function PolicyPane({ server, name }: { server: string; name: string }) {
  const { data, error } = useFetch(() => api.policy(server, name), [server, name]);
  // What the last change answered with, which outranks the first read.
  const [changed, setChanged] = useState<View | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const { ask, dialog } = useConfirm();

  useEffect(() => setChanged(null), [server, name]);

  const run = async (key: string, change: Promise<View>) => {
    setBusy(key);
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
            <code>{endpoint}</code> is denied to everything in this sandbox, whatever opens it,
            and anything that depends on it stops working.
          </>
        ),
        confirm: "block",
        onConfirm: () => void run(endpoint, api.block(server, name, endpoint, false)),
      }),
    remove: (rule, host) => {
      const others = rule.hosts.filter((h) => h !== host);
      const go = () => void run(rule.id, api.removeRule(server, name, rule.id));
      // A rule naming several hosts goes for all of them, which is worth
      // asking about; one naming only this host is its own undo.
      if (others.length === 0) return go();
      ask({
        title: "Remove this rule?",
        body: (
          <>
            It also names <code>{others.join(", ")}</code>, which lose it too.
          </>
        ),
        confirm: "remove",
        onConfirm: go,
      });
    },
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

  const own = byHost(view.rules.filter((r) => !r.global));
  const global = byHost(view.rules.filter((r) => r.global));
  const lists = view.lists;

  return (
    <div className="policy">
      <header className="policy-head">
        <PolicyGlyph className="policy-glyph" />
        <span className="policy-name">{view.template ?? "no template recorded"}</span>
        <span className="pill fixed" title="rules on this session's own sandbox">
          {view.rules.filter((r) => !r.global).length} rules
        </span>
      </header>

      <section>
        <h4>
          this session
          {own.length > 0 && (
            <span className="count">
              {own.length} {own.length === 1 ? "host" : "hosts"}
            </span>
          )}
        </h4>
        {own.length === 0 ? (
          <Notice>No rules of its own: nothing in this sandbox has egress.</Notice>
        ) : (
          own.map(([host, rules]) => (
            <HostCard key={host} host={host} rules={rules} change={change} busy={busy} menu={menu} />
          ))
        )}
      </section>

      {global.length > 0 && (
        <section>
          <h4 title="From the sandbox runtime's own policy; changed with `sbx policy`, not here.">every sandbox</h4>
          {global.map(([host, rules]) => (
            <HostCard key={host} host={host} rules={rules} change={change} busy={busy} menu={menu} readOnly />
          ))}
        </section>
      )}

      {lists && (
        <section>
          <h4 title="Applied to every new session, so an entry may not be in this one.">every new session</h4>
          <ul className="lists">
            {lists.allow.length + lists.routes.length + lists.block.length === 0 && (
              <li className="none">
                none yet. Allow or block from the events pane with “every new session” ticked.
              </li>
            )}
            {lists.allow.map((a) => (
              <li key={a.endpoint} className="listed" tabIndex={0} {...listed(a.endpoint)}>
                <Grant className="allow" aria-label="allowed" />
                <span className="host">{a.endpoint}</span>
                <State busy={busy === a.endpoint} on={a.in_policy} yes="in this session" no="not in this one" />
              </li>
            ))}
            {lists.routes.map((r) => (
              <li key={r.endpoint} className="listed routed" tabIndex={0} {...listed(r.endpoint)}>
                <Grant className="allow" aria-label="allowed" />
                <span className="host">{r.endpoint}</span>
                <State busy={busy === r.endpoint} on={r.in_policy} yes="in this session" no="not in this one" />
                <ul className="l7">
                  {r.routes.map((route) => (
                    <li key={`${route.method} ${route.path}`}>
                      <span className="method">{route.method}</span>
                      <span className="path">{route.path}</span>
                    </li>
                  ))}
                </ul>
              </li>
            ))}
            {lists.block.map((b) => (
              <li key={b.endpoint} className="listed" tabIndex={0} {...listed(b.endpoint)}>
                <Revoke className="block" aria-label="blocked" />
                <span className="host">{b.endpoint}</span>
                <State busy={busy === b.endpoint} on={b.in_policy} yes="denied here" no="not denied in this one" warnOff />
              </li>
            ))}
          </ul>
        </section>
      )}

      <Notice>
        Rules are for the whole sandbox, not for one program in it. A deny outranks every allow.
      </Notice>
    </div>
  );
}

function HostCard({
  host,
  rules,
  change,
  busy,
  menu,
  readOnly,
}: {
  host: string;
  rules: Rule[];
  change: Change;
  busy: string | null;
  menu: ReturnType<typeof useContextMenu>;
  readOnly?: boolean;
}) {
  const [name, port] = split(host);
  // A host the rules name without a port is any port; blocking it from here
  // means HTTPS, which is what nearly every rule is about.
  const endpoint = port ? host : `${host}:443`;
  const denied = rules.some((r) => !r.allow && r.methods.length === 0);
  return (
    <article className="rule">
      <h3
        title={host}
        {...menu(() => [
          ...(readOnly || denied
            ? []
            : [
                {
                  label: "Block…",
                  icon: Revoke,
                  danger: true,
                  disabled: busy !== null,
                  run: () => change.block(endpoint),
                },
                "separator" as const,
              ]),
          { label: "Copy host", icon: Copy, hint: host, run: () => copy(host) },
        ])}
      >
        <span className="host">
          {name}
          {port && <span className="port">:{port}</span>}
        </span>
      </h3>
      <ul className="l7">
        {rules.map((r) => (
          <li
            key={r.id}
            className={`listed${r.allow ? "" : " deny"}`}
            tabIndex={0}
            title={r.hosts.length > 1 ? `one rule for ${r.hosts.join(", ")}` : undefined}
            {...menu(() => [
              ...(readOnly
                ? []
                : [
                    {
                      label: "Remove this rule",
                      icon: Forget,
                      disabled: busy !== null,
                      run: () => change.remove(r, host),
                    },
                    "separator" as const,
                  ]),
              { label: "Copy rule id", icon: Copy, hint: r.id.slice(0, 8), run: () => copy(r.id) },
            ])}
          >
            <span className="method">{r.methods.length ? r.methods.join(",") : "any"}</span>
            <span className="path">{r.methods.length ? (r.path ?? "/**") : "every request"}</span>
            {busy === r.id ? (
              <span className="spin" />
            ) : (
              !r.allow && <span className="deny-word">deny</span>
            )}
          </li>
        ))}
      </ul>
    </article>
  );
}

function State({
  busy,
  on,
  yes,
  no,
  warnOff,
}: {
  busy: boolean;
  on: boolean;
  yes: string;
  no: string;
  warnOff?: boolean;
}) {
  if (busy) return <span className="spin" />;
  return <span className={`state${on ? "" : warnOff ? " warn" : " off"}`}>{on ? yes : no}</span>;
}

/// The rules grouped under each host they name, in the order the hosts first
/// appear. A rule naming several hosts is under each of them.
function byHost(rules: Rule[]): [string, Rule[]][] {
  const out = new Map<string, Rule[]>();
  for (const r of rules) {
    for (const h of r.hosts) {
      const list = out.get(h) ?? [];
      list.push(r);
      out.set(h, list);
    }
  }
  return [...out.entries()];
}

/// `host:443` as its two halves, and a bare host or glob as itself.
function split(host: string): [string, string | null] {
  const i = host.lastIndexOf(":");
  if (i > 0 && /^\d+$/.test(host.slice(i + 1))) return [host.slice(0, i), host.slice(i + 1)];
  return [host, null];
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
