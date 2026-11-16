// The hura relay: how hurad reaches a phone. See docs/relay-api.md.

import { DEAD_ACTIVITY_TOKEN_REASONS, DEAD_TOKEN_REASONS, sendApns, type Environment } from "./apns";
import {
  activityPayload,
  apnsPayload,
  BACKGROUND,
  KINDS,
  PHASES,
  STARTING,
  type ActivityEvent,
  type Kind,
  type Notification,
  type Phase,
  type SessionActivity,
} from "./notify";
import {
  badRequest,
  bearer,
  HttpError,
  iso,
  json,
  newId,
  newSecret,
  notFound,
  readJSON,
  sha256hex,
  unauthorized,
  type RelayEnv,
  VERSION,
} from "./util";

// Only handlers may be exported from this module: workerd treats every
// export of the main module as an entrypoint and refuses to start otherwise.

const REGISTRATIONS_PER_HOUR = 10;
const EVENTS_PER_MINUTE = 30;
const LINKS_PER_DEVICE = 50;
const DAY_MS = 24 * 3600 * 1000;
// An activity lasts at most eight hours, and an ended one shows for four more.
const ACTIVITY_MS = 12 * 3600 * 1000;
// A Live Activity push that arrives an hour late says something long since
// replaced.
const ACTIVITY_EXPIRY_SECONDS = 3600;
const SESSION_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
// What an activity's token is while it is being started, until the phone
// gives the real one. Never valid hex, so never mistaken for a token.
const STARTING_TOKEN = "";
// How long a started activity is taken to be waiting for its token. The
// phone gives it when the app next runs, which may be much later; until then
// another start would put the session on the Lock Screen twice. ActivityKit
// ends an activity after eight hours, so one still waiting by then never
// started.
const STARTING_MS = 8 * 3600 * 1000;

interface DeviceRow {
  id: string;
  apns_token: string;
  environment: Environment;
  disabled_at: number | null;
  /** The push-to-start token for Live Activities, when the app gave one. */
  activity_token: string | null;
}

interface LinkRow {
  id: string;
  device_id: string;
  server_id: string;
  label: string;
  created_at: number;
  last_event_at: number | null;
}

type Handler = (req: Request, env: RelayEnv, params: Record<string, string>) => Promise<Response>;

const routes: Array<[string, RegExp, Handler]> = [
  ["GET", /^\/v1\/health$/, async () => json(200, { ok: true, version: VERSION })],
  ["POST", /^\/v1\/devices$/, registerDevice],
  ["PUT", /^\/v1\/devices\/(?<device>d_[a-z2-7]{16})$/, updateDevice],
  ["DELETE", /^\/v1\/devices\/(?<device>d_[a-z2-7]{16})$/, deleteDevice],
  ["POST", /^\/v1\/devices\/(?<device>d_[a-z2-7]{16})\/test$/, testDevice],
  ["PUT", /^\/v1\/devices\/(?<device>d_[a-z2-7]{16})\/activity-token$/, setStartToken],
  ["DELETE", /^\/v1\/devices\/(?<device>d_[a-z2-7]{16})\/activity-token$/, deleteStartToken],
  ["POST", /^\/v1\/servers$/, createLink],
  ["GET", /^\/v1\/servers$/, listLinks],
  ["PATCH", /^\/v1\/servers\/(?<link>l_[a-z2-7]{16})$/, renameLink],
  ["DELETE", /^\/v1\/servers\/(?<link>l_[a-z2-7]{16})$/, deleteLink],
  ["PUT", /^\/v1\/servers\/(?<link>l_[a-z2-7]{16})\/activities\/(?<session>[^/]+)$/, setActivityToken],
  ["DELETE", /^\/v1\/servers\/(?<link>l_[a-z2-7]{16})\/activities\/(?<session>[^/]+)$/, deleteActivityToken],
  ["GET", /^\/v1\/link$/, checkLink],
  ["POST", /^\/v1\/events$/, postEvent],
];

export default {
  async fetch(req: Request, env: RelayEnv): Promise<Response> {
    const path = new URL(req.url).pathname;
    try {
      for (const [method, re, handler] of routes) {
        const m = re.exec(path);
        if (m && req.method === method) return await handler(req, env, m.groups ?? {});
      }
      throw notFound();
    } catch (e) {
      if (e instanceof HttpError) return json(e.status, { error: { code: e.code, message: e.message } }, e.headers);
      console.error(JSON.stringify({ msg: "unhandled error", path, error: String(e) }));
      return json(500, { error: { code: "internal", message: "internal error" } });
    }
  },

  async scheduled(_ctrl: ScheduledController, env: RelayEnv): Promise<void> {
    const now = Date.now();
    await env.DB.batch([
      env.DB.prepare("DELETE FROM events WHERE created_at < ?").bind(now - DAY_MS),
      env.DB.prepare("DELETE FROM registrations WHERE at < ?").bind(now - 3600 * 1000),
      env.DB.prepare("DELETE FROM activities WHERE updated_at < ?").bind(now - ACTIVITY_MS),
    ]);
  },
} satisfies ExportedHandler<RelayEnv>;

// --- credentials --------------------------------------------------------

async function authDevice(req: Request, env: RelayEnv, pathId?: string): Promise<DeviceRow> {
  const secret = bearer(req);
  if (!secret?.startsWith("hud_")) throw unauthorized();
  const row = await env.DB.prepare(
    "SELECT id, apns_token, environment, disabled_at, activity_token FROM devices WHERE secret_hash = ?",
  )
    .bind(await sha256hex(secret))
    .first<DeviceRow>();
  // A valid secret for another device is as good as no secret here.
  if (!row || (pathId !== undefined && row.id !== pathId)) throw unauthorized();
  return row;
}

async function authLink(req: Request, env: RelayEnv): Promise<LinkRow & { device: DeviceRow }> {
  const token = bearer(req);
  if (!token?.startsWith("hup_")) throw unauthorized();
  const row = await env.DB.prepare(
    `SELECT l.id, l.device_id, l.server_id, l.label, l.created_at, l.last_event_at,
            d.apns_token, d.environment, d.disabled_at, d.activity_token
       FROM links l JOIN devices d ON d.id = l.device_id
      WHERE l.token_hash = ?`,
  )
    .bind(await sha256hex(token))
    .first<LinkRow & Omit<DeviceRow, "id">>();
  if (!row) throw unauthorized();
  return {
    ...row,
    device: {
      id: row.device_id,
      apns_token: row.apns_token,
      environment: row.environment,
      disabled_at: row.disabled_at,
      activity_token: row.activity_token,
    },
  };
}

// --- validation ---------------------------------------------------------

function str(body: Record<string, unknown>, key: string, opts: { max: number; re?: RegExp; optional?: boolean }) {
  const v = body[key];
  if (v === undefined || v === null) {
    if (opts.optional) return undefined;
    throw badRequest(`${key} is required`);
  }
  if (typeof v !== "string") throw badRequest(`${key} must be a string`);
  if (opts.re && !opts.re.test(v)) throw badRequest(`${key} is malformed`);
  if (v.length > opts.max) throw badRequest(`${key} is too long`);
  return v;
}

function deviceFields(body: Record<string, unknown>) {
  const token = str(body, "apns_token", { max: 200, re: /^[0-9a-fA-F]{64,200}$/ })!.toLowerCase();
  const environment = str(body, "environment", { max: 20, re: /^(sandbox|production)$/ }) as Environment;
  return { token, environment };
}

/** A Live Activity token: hex, as ActivityKit hands it out. */
function activityToken(body: Record<string, unknown>) {
  return str(body, "token", { max: 200, re: /^[0-9a-fA-F]{64,200}$/ })!.toLowerCase();
}

// --- device routes ------------------------------------------------------

async function registerDevice(req: Request, env: RelayEnv): Promise<Response> {
  const ip = req.headers.get("cf-connecting-ip") ?? "unknown";
  const now = Date.now();
  const recent = await env.DB.prepare("SELECT COUNT(*) AS n FROM registrations WHERE ip = ? AND at > ?")
    .bind(ip, now - 3600 * 1000)
    .first<{ n: number }>();
  if ((recent?.n ?? 0) >= REGISTRATIONS_PER_HOUR) {
    throw new HttpError(429, "rate_limited", "too many registrations", { "retry-after": "3600" });
  }
  const { token, environment } = deviceFields(await readJSON(req));
  const id = newId("d_");
  const secret = newSecret("hud_");
  await env.DB.batch([
    env.DB.prepare(
      "INSERT INTO devices (id, secret_hash, apns_token, environment, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    ).bind(id, await sha256hex(secret), token, environment, now, now),
    env.DB.prepare("INSERT INTO registrations (ip, at) VALUES (?, ?)").bind(ip, now),
  ]);
  return json(201, { device_id: id, device_secret: secret });
}

async function updateDevice(req: Request, env: RelayEnv, p: Record<string, string>): Promise<Response> {
  const dev = await authDevice(req, env, p.device);
  const { token, environment } = deviceFields(await readJSON(req));
  await env.DB.prepare("UPDATE devices SET apns_token = ?, environment = ?, updated_at = ?, disabled_at = NULL WHERE id = ?")
    .bind(token, environment, Date.now(), dev.id)
    .run();
  return json(200, { device_id: dev.id });
}

async function deleteDevice(req: Request, env: RelayEnv, p: Record<string, string>): Promise<Response> {
  const dev = await authDevice(req, env, p.device);
  // D1 enforces foreign keys, so links and their events go with the device.
  await env.DB.prepare("DELETE FROM devices WHERE id = ?").bind(dev.id).run();
  return json(204, null);
}

async function testDevice(req: Request, env: RelayEnv, p: Record<string, string>): Promise<Response> {
  const dev = await authDevice(req, env, p.device);
  const payload = apnsPayload(
    { kind: "test", title: "hura", body: "Test notification from the hura relay" },
    { id: "l_test", serverId: "relay", label: "hura" },
  );
  const r = await sendApns(env, { token: dev.apns_token, environment: dev.environment }, payload);
  if (r.status === 200 && dev.disabled_at !== null) {
    await env.DB.prepare("UPDATE devices SET disabled_at = NULL WHERE id = ?").bind(dev.id).run();
  } else if (r.reason && DEAD_TOKEN_REASONS.has(r.reason)) {
    await env.DB.prepare("UPDATE devices SET disabled_at = ? WHERE id = ?").bind(Date.now(), dev.id).run();
  }
  log("device test", { device: dev.id, apns_status: r.status, apns_reason: r.reason });
  return json(200, { sent: r.status === 200, apns_status: r.status, apns_id: r.apnsId ?? null, apns_reason: r.reason ?? null });
}

/** The device's push-to-start token, which starts a Live Activity. */
async function setStartToken(req: Request, env: RelayEnv, p: Record<string, string>): Promise<Response> {
  const dev = await authDevice(req, env, p.device);
  const token = activityToken(await readJSON(req));
  await env.DB.prepare("UPDATE devices SET activity_token = ? WHERE id = ?").bind(token, dev.id).run();
  return json(204, null);
}

async function deleteStartToken(req: Request, env: RelayEnv, p: Record<string, string>): Promise<Response> {
  const dev = await authDevice(req, env, p.device);
  await env.DB.prepare("UPDATE devices SET activity_token = NULL WHERE id = ?").bind(dev.id).run();
  return json(204, null);
}

// --- link routes --------------------------------------------------------

async function createLink(req: Request, env: RelayEnv): Promise<Response> {
  const dev = await authDevice(req, env);
  const body = await readJSON(req);
  const serverId = str(body, "server_id", { max: 64, re: /^h_[a-z2-7]{16}$/ })!;
  const label = str(body, "label", { max: 100 })!.trim() || serverId;
  const count = await env.DB.prepare("SELECT COUNT(*) AS n FROM links WHERE device_id = ?").bind(dev.id).first<{ n: number }>();
  if ((count?.n ?? 0) >= LINKS_PER_DEVICE) throw badRequest(`a device can have at most ${LINKS_PER_DEVICE} links`);
  const id = newId("l_");
  const token = newSecret("hup_");
  await env.DB.prepare("INSERT INTO links (id, device_id, server_id, label, token_hash, created_at) VALUES (?, ?, ?, ?, ?, ?)")
    .bind(id, dev.id, serverId, label, await sha256hex(token), Date.now())
    .run();
  return json(201, { link_id: id, push_token: token });
}

async function listLinks(req: Request, env: RelayEnv): Promise<Response> {
  const dev = await authDevice(req, env);
  const { results } = await env.DB.prepare(
    "SELECT id, server_id, label, created_at, last_event_at FROM links WHERE device_id = ? ORDER BY created_at",
  )
    .bind(dev.id)
    .all<LinkRow>();
  return json(200, {
    links: results.map((l) => ({
      link_id: l.id,
      server_id: l.server_id,
      label: l.label,
      created_at: iso(l.created_at),
      last_event_at: iso(l.last_event_at),
    })),
  });
}

/// A new label, for when the server is renamed on the phone: it is the title
/// of a notification the phone could not decrypt.
async function renameLink(req: Request, env: RelayEnv, p: Record<string, string>): Promise<Response> {
  const dev = await authDevice(req, env);
  const body = await readJSON(req);
  const label = str(body, "label", { max: 100 })!.trim();
  if (!label) throw badRequest("label must not be empty");
  const r = await env.DB.prepare("UPDATE links SET label = ? WHERE id = ? AND device_id = ?")
    .bind(label, p.link, dev.id)
    .run();
  if (!r.meta.changes) throw notFound();
  return json(204, null);
}

async function deleteLink(req: Request, env: RelayEnv, p: Record<string, string>): Promise<Response> {
  const dev = await authDevice(req, env);
  const r = await env.DB.prepare("DELETE FROM links WHERE id = ? AND device_id = ?").bind(p.link, dev.id).run();
  if (!r.meta.changes) throw notFound();
  return json(204, null);
}

/** The link, when it is the device's own, and the session from the path. */
async function activityTarget(req: Request, env: RelayEnv, p: Record<string, string>) {
  const dev = await authDevice(req, env);
  if (!SESSION_RE.test(p.session)) throw badRequest("session is malformed");
  const owned = await env.DB.prepare("SELECT id, server_id, label FROM links WHERE id = ? AND device_id = ?")
    .bind(p.link, dev.id)
    .first<{ id: string; server_id: string; label: string }>();
  if (!owned) throw notFound();
  return { dev, link: owned, session: p.session };
}

/**
 * The update token of the activity showing one session. An activity the
 * relay started has been waiting for it, perhaps through events that changed
 * what it should show: the latest of those goes to it now.
 */
async function setActivityToken(req: Request, env: RelayEnv, p: Record<string, string>): Promise<Response> {
  const { dev, link, session } = await activityTarget(req, env, p);
  const token = activityToken(await readJSON(req));
  const was = await env.DB.prepare("SELECT token, pending FROM activities WHERE link_id = ? AND session = ?")
    .bind(link.id, session)
    .first<{ token: string; pending: string | null }>();
  await env.DB.prepare(
    `INSERT INTO activities (link_id, session, token, updated_at) VALUES (?, ?, ?, ?)
     ON CONFLICT (link_id, session) DO UPDATE SET token = excluded.token, updated_at = excluded.updated_at,
                                                  pending = NULL`,
  )
    .bind(link.id, session, token, Date.now())
    .run();
  if (was?.token === STARTING_TOKEN && was.pending) {
    const a = JSON.parse(was.pending) as SessionActivity;
    const event: ActivityEvent = STARTING.has(a.phase) ? "update" : "end";
    const r = await sendApns(
      env,
      { token, environment: dev.environment },
      activityPayload(a, event, { id: link.id, serverId: link.server_id, label: link.label }),
      ACTIVITY_EXPIRY_SECONDS,
      "liveactivity",
    );
    // Best effort: the phone has done its part, and the next event will
    // update the activity anyway.
    log("activity caught up", { link: link.id, phase: a.phase, event, apns_status: r.status, apns_reason: r.reason });
    if (r.status === 200 && event === "end") {
      await env.DB.prepare("DELETE FROM activities WHERE link_id = ? AND session = ?").bind(link.id, session).run();
    }
  }
  return json(204, null);
}

async function deleteActivityToken(req: Request, env: RelayEnv, p: Record<string, string>): Promise<Response> {
  const { link, session } = await activityTarget(req, env, p);
  await env.DB.prepare("DELETE FROM activities WHERE link_id = ? AND session = ?").bind(link.id, session).run();
  return json(204, null);
}

// --- server routes -------------------------------------------------------

async function checkLink(req: Request, env: RelayEnv): Promise<Response> {
  const link = await authLink(req, env);
  return json(200, { link_id: link.id, label: link.label, device_enabled: link.device.disabled_at === null });
}

async function postEvent(req: Request, env: RelayEnv): Promise<Response> {
  const link = await authLink(req, env);
  const body = await readJSON(req);
  const eventId = str(body, "event_id", { max: 100, re: /^[A-Za-z0-9_.:-]+$/ })!;
  const kind = str(body, "kind", { max: 40 })!;
  if (!(kind in KINDS)) throw badRequest(`unknown kind ${JSON.stringify(kind)}`);
  // A background kind carries nothing to show, and is refused with text.
  const background = BACKGROUND.has(kind as Kind);
  if (background && ["title", "subtitle", "body", "encrypted"].some((k) => body[k] !== undefined && body[k] !== null)) {
    throw badRequest(`${kind} carries no text`);
  }
  const encrypted = background
    ? ""
    : str(body, "encrypted", { max: 3000, re: /^[A-Za-z0-9_-]+$/, optional: true });
  if (encrypted !== undefined && ["title", "subtitle", "body"].some((k) => body[k] !== undefined && body[k] !== null)) {
    throw badRequest("send either encrypted or title, subtitle and body");
  }
  const title = encrypted === undefined ? str(body, "title", { max: 1000 })! : "";
  const subtitle = encrypted === undefined ? str(body, "subtitle", { max: 1000, optional: true }) : undefined;
  const text = encrypted === undefined ? str(body, "body", { max: 4000 })! : "";
  // A hura session name, which the app opens when the notification is tapped.
  const activity = kind === "session_activity";
  const session = str(body, "session", { max: 64, re: SESSION_RE, optional: !activity });
  const phase = activity ? (str(body, "phase", { max: 20 }) as Phase) : undefined;
  if (phase !== undefined && !PHASES.includes(phase)) throw badRequest(`unknown phase ${JSON.stringify(phase)}`);
  if (!activity && body.phase !== undefined) throw badRequest("phase is only for session_activity");
  // The request a permission notification answers. Required for that kind,
  // whose Allow and Deny would otherwise answer nothing, and refused on any
  // other, so an id never rides along where nothing will use it.
  const askId = str(body, "ask_id", { max: 100, re: /^[A-Za-z0-9_-]{1,100}$/, optional: true });
  // A Live Activity waiting on a permission request carries one too, for its
  // Allow and Deny.
  const asks = kind === "claude_permission" || phase === "permission";
  if (asks && askId === undefined) throw badRequest(`ask_id is required for ${phase ?? kind}`);
  if (!asks && askId !== undefined) throw badRequest("ask_id is only for a permission request");

  const now = Date.now();
  let at = Math.floor(now / 1000);
  if (activity && body.at !== undefined && body.at !== null) {
    if (typeof body.at !== "number" || !Number.isInteger(body.at) || body.at < 0) {
      throw badRequest("at must be a whole number of seconds");
    }
    // A clock ahead of the relay's would hold off every later update.
    at = Math.min(body.at, at);
  } else if (!activity && body.at !== undefined) {
    throw badRequest("at is only for session_activity");
  }
  let since: number | undefined;
  if (activity && body.since !== undefined && body.since !== null) {
    if (typeof body.since !== "number" || !Number.isInteger(body.since) || body.since < 0) {
      throw badRequest("since must be a whole number of seconds");
    }
    since = Math.min(body.since, Math.floor(now / 1000));
  } else if (!activity && body.since !== undefined) {
    throw badRequest("since is only for session_activity");
  }
  const recent = await env.DB.prepare("SELECT COUNT(*) AS n FROM events WHERE link_id = ? AND created_at > ?")
    .bind(link.id, now - 60 * 1000)
    .first<{ n: number }>();
  if ((recent?.n ?? 0) >= EVENTS_PER_MINUTE) {
    throw new HttpError(429, "rate_limited", "too many events", { "retry-after": "60" });
  }
  const ins = await env.DB.prepare("INSERT OR IGNORE INTO events (link_id, event_id, created_at) VALUES (?, ?, ?)")
    .bind(link.id, eventId, now)
    .run();
  if (!ins.meta.changes) return json(200, { sent: false, reason: "duplicate" });
  if (link.device.disabled_at !== null) return json(200, { sent: false, reason: "device_unregistered" });

  if (activity) {
    const a: SessionActivity =
      encrypted !== undefined
        ? { session: session!, phase: phase!, at, since, askId, encrypted }
        : { session: session!, phase: phase!, at, since, askId, title, subtitle, body: text };
    return sendActivity(env, link, a, eventId, now);
  }

  const n: Notification =
    encrypted !== undefined
      ? { kind: kind as Kind, encrypted, session, askId }
      : { kind: kind as Kind, title, subtitle, body: text, session, askId };
  const payload = apnsPayload(n, { id: link.id, serverId: link.server_id, label: link.label });
  const r = await sendApns(
    env,
    { token: link.device.apns_token, environment: link.device.environment },
    payload,
    undefined,
    background ? "background" : "alert",
  );
  log("event", { link: link.id, kind, apns_status: r.status, apns_reason: r.reason });

  if (r.status === 200) {
    await env.DB.prepare("UPDATE links SET last_event_at = ? WHERE id = ?").bind(now, link.id).run();
    return json(200, { sent: true });
  }
  if (r.reason && DEAD_TOKEN_REASONS.has(r.reason)) {
    await env.DB.prepare("UPDATE devices SET disabled_at = ? WHERE id = ?").bind(now, link.device.id).run();
    return json(200, { sent: false, reason: "device_unregistered" });
  }
  // Not delivered: forget the event ID so hurad's retry goes through.
  await env.DB.prepare("DELETE FROM events WHERE link_id = ? AND event_id = ?").bind(link.id, eventId).run();
  throw new HttpError(502, "apns_error", `APNs answered ${r.status || "nothing"}${r.reason ? ` (${r.reason})` : ""}`);
}

/**
 * A session's Live Activity: updated or ended when the phone gave the relay
 * its token, started when it gave a push-to-start token instead, and nothing
 * otherwise (docs/relay-api.md, "Live Activities").
 */
async function sendActivity(
  env: RelayEnv,
  link: LinkRow & { device: DeviceRow },
  a: SessionActivity,
  eventId: string,
  now: number,
): Promise<Response> {
  const linkInfo = { id: link.id, serverId: link.server_id, label: link.label };
  const send = (token: string, event: ActivityEvent) =>
    sendApns(
      env,
      { token, environment: link.device.environment },
      activityPayload(a, event, linkInfo, now),
      ACTIVITY_EXPIRY_SECONDS,
      "liveactivity",
    );
  const forget = () =>
    env.DB.prepare("DELETE FROM activities WHERE link_id = ? AND session = ?").bind(link.id, a.session).run();
  const failed = async (r: { status: number; reason?: string }) => {
    // Not delivered: forget the event ID so hurad's retry goes through.
    await env.DB.prepare("DELETE FROM events WHERE link_id = ? AND event_id = ?").bind(link.id, eventId).run();
    throw new HttpError(502, "apns_error", `APNs answered ${r.status || "nothing"}${r.reason ? ` (${r.reason})` : ""}`);
  };
  const delivered = async (event: ActivityEvent) => {
    log("activity", { link: link.id, phase: a.phase, event });
    await env.DB.prepare("UPDATE links SET last_event_at = ? WHERE id = ?").bind(now, link.id).run();
    return json(200, { sent: true, activity: event });
  };

  let row = await env.DB.prepare("SELECT token, updated_at FROM activities WHERE link_id = ? AND session = ?")
    .bind(link.id, a.session)
    .first<{ token: string; updated_at: number }>();
  if (row?.token === STARTING_TOKEN) {
    // Started, and the phone hasn't yet said where its updates go: it says
    // so when the app next runs. Another start would put the session on the
    // Lock Screen twice, so this is kept for then instead.
    if (now - row.updated_at < STARTING_MS) {
      await env.DB.prepare("UPDATE activities SET pending = ? WHERE link_id = ? AND session = ?")
        .bind(JSON.stringify(a), link.id, a.session)
        .run();
      return json(200, { sent: false, reason: "activity_starting" });
    }
    // Long enough that it never did: the start was lost, or the phone said no.
    await forget();
    row = null;
  }
  if (row) {
    const event: ActivityEvent = STARTING.has(a.phase) ? "update" : "end";
    const r = await send(row.token, event);
    if (r.status === 200) {
      if (event === "end") await forget();
      else {
        await env.DB.prepare("UPDATE activities SET updated_at = ? WHERE link_id = ? AND session = ?")
          .bind(now, link.id, a.session)
          .run();
      }
      return delivered(event);
    }
    if (!r.reason || !DEAD_ACTIVITY_TOKEN_REASONS.has(r.reason)) return failed(r);
    // The activity is gone, ended or dismissed on the phone: start another
    // below if this phase would.
    await forget();
    log("activity token gone", { link: link.id, apns_reason: r.reason });
  }

  if (!STARTING.has(a.phase) || !link.device.activity_token) return json(200, { sent: false, reason: "no_activity" });
  const r = await send(link.device.activity_token, "start");
  if (r.status === 200) {
    await env.DB.prepare(
      `INSERT INTO activities (link_id, session, token, updated_at) VALUES (?, ?, ?, ?)
       ON CONFLICT (link_id, session) DO UPDATE SET token = excluded.token, updated_at = excluded.updated_at`,
    )
      .bind(link.id, a.session, STARTING_TOKEN, now)
      .run();
    return delivered("start");
  }
  if (r.reason && DEAD_ACTIVITY_TOKEN_REASONS.has(r.reason)) {
    await env.DB.prepare("UPDATE devices SET activity_token = NULL WHERE id = ?").bind(link.device.id).run();
    log("activity start token gone", { device: link.device.id, apns_reason: r.reason });
    return json(200, { sent: false, reason: "no_activity" });
  }
  return failed(r);
}

function log(msg: string, fields: Record<string, unknown>) {
  console.log(JSON.stringify({ msg, ...fields }));
}
