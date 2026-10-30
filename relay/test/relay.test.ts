import { env } from "cloudflare:test";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { apnsOrigin, providerToken, resetApnsCache } from "../src/apns";
import * as mainModule from "../src/index";
import worker from "../src/index";

// --- helpers --------------------------------------------------------------

const APNS_TOKEN = "a".repeat(64);
const SERVER = "h_abcdefghijklmnop";

interface ApnsCall {
  url: string;
  headers: Headers;
  body: any;
}

let apnsCalls: ApnsCall[] = [];
let apnsReplies: Array<Response | Error> = [];

beforeEach(() => {
  apnsCalls = [];
  apnsReplies = [];
  resetApnsCache();
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    const req = new Request(input as RequestInfo, init);
    apnsCalls.push({ url: req.url, headers: req.headers, body: await req.json() });
    const next = apnsReplies.shift() ?? new Response(null, { status: 200, headers: { "apns-id": "id-1" } });
    if (next instanceof Error) throw next;
    return next;
  });
});

afterEach(() => vi.restoreAllMocks());

function apnsError(status: number, reason: string) {
  return new Response(JSON.stringify({ reason }), { status });
}

async function call(method: string, path: string, opts: { auth?: string; body?: unknown; ip?: string } = {}) {
  const headers: Record<string, string> = { "cf-connecting-ip": opts.ip ?? "203.0.113.1" };
  if (opts.auth) headers.authorization = `Bearer ${opts.auth}`;
  if (opts.body !== undefined) headers["content-type"] = "application/json";
  const res = await worker.fetch(
    new Request(`https://relay.test${path}`, {
      method,
      headers,
      body: opts.body === undefined ? undefined : typeof opts.body === "string" ? opts.body : JSON.stringify(opts.body),
    }),
    env,
  );
  const text = await res.text();
  return { status: res.status, headers: res.headers, json: text ? JSON.parse(text) : null };
}

let ipCounter = 0;
async function device(environment = "sandbox") {
  const r = await call("POST", "/v1/devices", {
    body: { apns_token: APNS_TOKEN, environment },
    ip: `198.51.100.${++ipCounter % 250}`,
  });
  expect(r.status).toBe(201);
  return r.json as { device_id: string; device_secret: string };
}

async function link(secret: string, serverId = SERVER, label = "studio") {
  const r = await call("POST", "/v1/servers", { auth: secret, body: { server_id: serverId, label } });
  expect(r.status).toBe(201);
  return r.json as { link_id: string; push_token: string };
}

let eventCounter = 0;
function event(extra: Record<string, unknown> = {}) {
  return {
    event_id: `h_testboot01-${++eventCounter}`,
    kind: "claude_needs_input",
    title: "fix-tests",
    subtitle: "studio",
    body: "Claude is asking to use Bash",
    session: "fix-tests",
    ...extra,
  };
}

function b64urlDecode(s: string): Uint8Array {
  const b64 = s.replace(/-/g, "+").replace(/_/g, "/");
  return Uint8Array.from(atob(b64 + "=".repeat((4 - (b64.length % 4)) % 4)), (c) => c.charCodeAt(0));
}

async function verifyJWT(jwt: string) {
  const [h, c, s] = jwt.split(".");
  const key = await crypto.subtle.importKey(
    "jwk",
    JSON.parse(env.TEST_APNS_PUBLIC_JWK),
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["verify"],
  );
  const ok = await crypto.subtle.verify(
    { name: "ECDSA", hash: "SHA-256" },
    key,
    b64urlDecode(s),
    new TextEncoder().encode(`${h}.${c}`),
  );
  const dec = (x: string) => JSON.parse(new TextDecoder().decode(b64urlDecode(x)));
  return { ok, header: dec(h), claims: dec(c) };
}

// --- tests ----------------------------------------------------------------

describe("basics", () => {
  it("exports only the default handler from the main module", () => {
    // workerd refuses to start a Worker whose main module exports anything else.
    expect(Object.keys(mainModule)).toEqual(["default"]);
  });

  it("reports health", async () => {
    const r = await call("GET", "/v1/health");
    expect(r).toMatchObject({ status: 200, json: { ok: true } });
  });

  it("404s unknown routes and wrong methods", async () => {
    expect((await call("GET", "/v1/nope")).status).toBe(404);
    expect((await call("GET", "/v1/events")).status).toBe(404);
  });

  it("rejects bodies that are not JSON objects", async () => {
    for (const body of ["{", "[]", "1"]) {
      const r = await call("POST", "/v1/devices", { body, ip: "192.0.2.77" });
      expect(r.status).toBe(400);
      expect(r.json.error.code).toBe("bad_request");
    }
  });
});

describe("devices", () => {
  it("registers and returns credentials", async () => {
    const d = await device();
    expect(d.device_id).toMatch(/^d_[a-z2-7]{16}$/);
    expect(d.device_secret).toMatch(/^hud_[A-Za-z0-9_-]{43}$/);
    // Only the hash is stored.
    const row = await env.DB.prepare("SELECT * FROM devices WHERE id = ?").bind(d.device_id).first<Record<string, string>>();
    expect(JSON.stringify(row)).not.toContain(d.device_secret);
  });

  it("validates device fields", async () => {
    for (const body of [
      { apns_token: "zz", environment: "sandbox" },
      { apns_token: APNS_TOKEN, environment: "staging" },
      { environment: "sandbox" },
    ]) {
      expect((await call("POST", "/v1/devices", { body, ip: "192.0.2.10" })).status).toBe(400);
    }
  });

  it("rate limits registrations per address", async () => {
    const body = { apns_token: APNS_TOKEN, environment: "sandbox" };
    for (let i = 0; i < 10; i++) expect((await call("POST", "/v1/devices", { body, ip: "192.0.2.50" })).status).toBe(201);
    const limited = await call("POST", "/v1/devices", { body, ip: "192.0.2.50" });
    expect(limited.status).toBe(429);
    expect(limited.headers.get("retry-after")).toBe("3600");
    expect((await call("POST", "/v1/devices", { body, ip: "192.0.2.51" })).status).toBe(201);
  });

  it("only lets a device manage itself", async () => {
    const a = await device();
    const b = await device();
    const body = { apns_token: "b".repeat(64), environment: "production" };
    expect((await call("PUT", `/v1/devices/${a.device_id}`, { auth: b.device_secret, body })).status).toBe(401);
    expect((await call("PUT", `/v1/devices/${a.device_id}`, { auth: "hud_nope", body })).status).toBe(401);
    expect((await call("PUT", `/v1/devices/${a.device_id}`, { body })).status).toBe(401);
    expect((await call("PUT", `/v1/devices/${a.device_id}`, { auth: a.device_secret, body })).status).toBe(200);
  });

  it("deleting a device removes its links", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    expect((await call("DELETE", `/v1/devices/${d.device_id}`, { auth: d.device_secret })).status).toBe(204);
    expect((await call("GET", "/v1/link", { auth: l.push_token })).status).toBe(401);
  });
});

describe("links", () => {
  it("creates, lists and deletes links", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    expect(l.link_id).toMatch(/^l_[a-z2-7]{16}$/);
    expect(l.push_token).toMatch(/^hup_[A-Za-z0-9_-]{43}$/);

    const list = await call("GET", "/v1/servers", { auth: d.device_secret });
    expect(list.json.links).toEqual([
      { link_id: l.link_id, server_id: SERVER, label: "studio", created_at: expect.any(String), last_event_at: null },
    ]);

    const check = await call("GET", "/v1/link", { auth: l.push_token });
    expect(check.json).toEqual({ link_id: l.link_id, label: "studio", device_enabled: true });

    // Another device can't delete it.
    const other = await device();
    expect((await call("DELETE", `/v1/servers/${l.link_id}`, { auth: other.device_secret })).status).toBe(404);
    expect((await call("DELETE", `/v1/servers/${l.link_id}`, { auth: d.device_secret })).status).toBe(204);
    expect((await call("GET", "/v1/link", { auth: l.push_token })).status).toBe(401);
  });

  it("validates link fields and credentials", async () => {
    const d = await device();
    for (const serverId of ["studio", "m_abcdefghijklmnop", "h_ABCDEFGHIJKLMNOP", "h_abc"]) {
      expect((await call("POST", "/v1/servers", { auth: d.device_secret, body: { server_id: serverId, label: "x" } })).status).toBe(400);
    }
    // A push token is not a device secret, and vice versa.
    const l = await link(d.device_secret);
    expect((await call("GET", "/v1/servers", { auth: l.push_token })).status).toBe(401);
    expect((await call("GET", "/v1/link", { auth: d.device_secret })).status).toBe(401);
  });
});

describe("events", () => {
  it("sends a notification with the payload the relay decides", async () => {
    const d = await device("production");
    const l = await link(d.device_secret);
    const r = await call("POST", "/v1/events", {
      auth: l.push_token,
      body: event({ server_id: "h_spoofedspoofedsp" }),
    });
    expect(r).toMatchObject({ status: 200, json: { sent: true } });

    expect(apnsCalls).toHaveLength(1);
    const c = apnsCalls[0];
    expect(c.url).toBe(`https://api.push.apple.com/3/device/${APNS_TOKEN}`);
    expect(c.headers.get("apns-topic")).toBe("dev.hura.test");
    expect(c.headers.get("apns-push-type")).toBe("alert");
    expect(c.headers.get("apns-priority")).toBe("10");
    expect(Number(c.headers.get("apns-expiration"))).toBeGreaterThan(Date.now() / 1000 + 5 * 3600);
    expect(c.body).toEqual({
      aps: {
        alert: { title: "fix-tests", subtitle: "studio", body: "Claude is asking to use Bash" },
        sound: "default",
        category: "CLAUDE_INPUT",
        "thread-id": `${SERVER}:fix-tests`,
        "interruption-level": "active",
      },
      // server_id comes from the link, never from the request.
      hura: { kind: "claude_needs_input", server_id: SERVER, link_id: l.link_id, session: "fix-tests" },
    });

    const auth = c.headers.get("authorization")!;
    expect(auth).toMatch(/^bearer /);
    const jwt = await verifyJWT(auth.slice(7));
    expect(jwt.ok).toBe(true);
    expect(jwt.header).toEqual({ alg: "ES256", kid: "TESTKEY123" });
    expect(jwt.claims.iss).toBe("TESTTEAM12");

    const list = await call("GET", "/v1/servers", { auth: d.device_secret });
    expect(list.json.links[0].last_event_at).not.toBeNull();
  });

  it("maps kinds to categories and interruption levels", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    const expected: Record<string, [string, string]> = {
      claude_needs_input: ["CLAUDE_INPUT", "active"],
      claude_permission: ["CLAUDE_PERMISSION", "active"],
      claude_turn_finished: ["CLAUDE_TURN", "active"],
      claude_failed: ["CLAUDE_FAILED", "time-sensitive"],
      test: ["TEST", "active"],
    };
    for (const [kind, [category, level]] of Object.entries(expected)) {
      apnsCalls = [];
      const ask = kind === "claude_permission" ? { ask_id: "toolu_01" } : {};
      expect((await call("POST", "/v1/events", { auth: l.push_token, body: event({ kind, ...ask }) })).status).toBe(200);
      expect(apnsCalls[0].body.aps.category).toBe(category);
      expect(apnsCalls[0].body.aps["interruption-level"]).toBe(level);
      // Sandbox devices go to the sandbox host.
      expect(apnsCalls[0].url).toMatch(/^https:\/\/api\.sandbox\.push\.apple\.com\//);
    }
  });

  it("sends encrypted events with generic text for the extension to replace", async () => {
    const d = await device();
    const l = await link(d.device_secret, SERVER, "narcissus");
    const enc = "oKGio6Slpqeoqaqr" + "A".repeat(200);
    const { title, subtitle, body, ...rest } = event();
    void title, subtitle, body;
    const r = await call("POST", "/v1/events", { auth: l.push_token, body: { ...rest, encrypted: enc } });
    expect(r).toMatchObject({ status: 200, json: { sent: true } });
    expect(apnsCalls[0].body).toEqual({
      aps: {
        alert: { title: "narcissus", body: "Claude needs your input" },
        sound: "default",
        category: "CLAUDE_INPUT",
        "thread-id": `${SERVER}:fix-tests`,
        "interruption-level": "active",
        "mutable-content": 1,
      },
      hura: { kind: "claude_needs_input", server_id: SERVER, link_id: l.link_id, session: "fix-tests", enc },
    });
    // The whole payload stays within APNs' 4 KB at the largest ciphertext.
    apnsCalls = [];
    await call("POST", "/v1/events", { auth: l.push_token, body: { ...rest, event_id: "h_big-1", encrypted: "A".repeat(3000) } });
    expect(new TextEncoder().encode(JSON.stringify(apnsCalls[0].body)).length).toBeLessThan(4096);
  });

  it("passes a permission request's id through, encrypted or not", async () => {
    const d = await device();
    const l = await link(d.device_secret, SERVER, "narcissus");
    const r = await call("POST", "/v1/events", {
      auth: l.push_token,
      body: event({ kind: "claude_permission", body: "Bash: npm test", ask_id: "toolu_01AbC-9" }),
    });
    expect(r).toMatchObject({ status: 200, json: { sent: true } });
    expect(apnsCalls[0].body.aps.category).toBe("CLAUDE_PERMISSION");
    expect(apnsCalls[0].body.aps.alert.body).toBe("Bash: npm test");
    expect(apnsCalls[0].body.hura).toEqual({
      kind: "claude_permission",
      server_id: SERVER,
      link_id: l.link_id,
      session: "fix-tests",
      ask_id: "toolu_01AbC-9",
    });

    // Encrypted, with the largest id, session and ciphertext, a long label,
    // still inside APNs' 4 KB.
    apnsCalls = [];
    const big = await link(d.device_secret, "h_qrstuvwxyzabcdef", "x".repeat(100));
    const { title, subtitle, body, ...rest } = event({ kind: "claude_permission" });
    void title, subtitle, body;
    const largest = {
      ...rest,
      event_id: "h_big-2",
      session: "s" + "a".repeat(63),
      encrypted: "A".repeat(3000),
      ask_id: "a".repeat(100),
    };
    expect((await call("POST", "/v1/events", { auth: big.push_token, body: largest })).status).toBe(200);
    expect(apnsCalls[0].body.aps.alert.body).toBe("Claude needs your permission");
    expect(apnsCalls[0].body.hura.ask_id).toBe("a".repeat(100));
    expect(new TextEncoder().encode(JSON.stringify(apnsCalls[0].body)).length).toBeLessThan(4096);
  });

  it("wants an ask_id on a permission request and nowhere else", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    for (const bad of [
      event({ kind: "claude_permission" }),
      event({ kind: "claude_permission", ask_id: "has space" }),
      event({ kind: "claude_permission", ask_id: "a".repeat(101) }),
      event({ kind: "claude_permission", ask_id: "" }),
      event({ kind: "claude_needs_input", ask_id: "toolu_01" }),
      event({ kind: "test", ask_id: "toolu_01" }),
    ]) {
      expect((await call("POST", "/v1/events", { auth: l.push_token, body: bad })).status).toBe(400);
    }
    expect(apnsCalls).toHaveLength(0);
  });

  it("refuses encrypted events that also carry text, or bad ciphertext", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    const { title, subtitle, body, ...rest } = event();
    void subtitle, body;
    for (const bad of [
      { ...rest, encrypted: "abc", title },
      { ...rest, encrypted: "not base64url!" },
      { ...rest, encrypted: "A".repeat(3001) },
      { ...rest, encrypted: 42 },
      rest,
    ]) {
      expect((await call("POST", "/v1/events", { auth: l.push_token, body: bad })).status).toBe(400);
    }
    expect(apnsCalls).toHaveLength(0);
  });

  it("validates events", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    for (const bad of [
      { kind: "made_up" },
      // Kinds hura does not send, which Radix knows.
      { kind: "shell_exited" },
      { kind: "claude_ended" },
      { session: "Fix-Tests" },
      { session: "-leading-dash" },
      { session: "has_underscore" },
      { session: "a".repeat(65) },
      { session: 7 },
      { title: 42 },
      { event_id: "has spaces" },
    ]) {
      expect((await call("POST", "/v1/events", { auth: l.push_token, body: event(bad) })).status).toBe(400);
    }
    expect((await call("POST", "/v1/events", { auth: "hup_unknown", body: event() })).status).toBe(401);
    // Radix's push token prefix is not this relay's.
    expect((await call("POST", "/v1/events", { auth: "hrp_" + l.push_token.slice(4), body: event() })).status).toBe(401);
    expect(apnsCalls).toHaveLength(0);
  });

  it("clips long text and omits optional fields", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    const long = "ä".repeat(900);
    await call("POST", "/v1/events", {
      auth: l.push_token,
      body: { event_id: "e1", kind: "test", title: long, body: long },
    });
    const { alert } = apnsCalls[0].body.aps;
    expect(Array.from(alert.title)).toHaveLength(100);
    expect(Array.from(alert.body)).toHaveLength(500);
    expect(alert.body.endsWith("…")).toBe(true);
    expect(alert.subtitle).toBeUndefined();
    expect(apnsCalls[0].body.hura).toEqual({ kind: "test", server_id: SERVER, link_id: l.link_id });
    expect(apnsCalls[0].body.aps["thread-id"]).toBe(SERVER);
  });

  it("acknowledges duplicates without sending again", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    const e = event();
    expect((await call("POST", "/v1/events", { auth: l.push_token, body: e })).json).toEqual({ sent: true });
    expect((await call("POST", "/v1/events", { auth: l.push_token, body: e })).json).toEqual({ sent: false, reason: "duplicate" });
    expect(apnsCalls).toHaveLength(1);
  });

  it("rate limits events per link", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    for (let i = 0; i < 30; i++) expect((await call("POST", "/v1/events", { auth: l.push_token, body: event() })).status).toBe(200);
    const r = await call("POST", "/v1/events", { auth: l.push_token, body: event() });
    expect(r.status).toBe(429);
    expect(r.headers.get("retry-after")).toBe("60");
    // Other links are unaffected.
    const l2 = await link(d.device_secret, "h_qrstuvwxyzabcdef");
    expect((await call("POST", "/v1/events", { auth: l2.push_token, body: event() })).status).toBe(200);
  });

  it("disables a device APNs no longer knows, until the app updates it", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    apnsReplies.push(apnsError(410, "Unregistered"));
    expect((await call("POST", "/v1/events", { auth: l.push_token, body: event() })).json).toEqual({
      sent: false,
      reason: "device_unregistered",
    });
    expect((await call("POST", "/v1/events", { auth: l.push_token, body: event() })).json.reason).toBe("device_unregistered");
    expect(apnsCalls).toHaveLength(1);
    expect((await call("GET", "/v1/link", { auth: l.push_token })).json.device_enabled).toBe(false);

    await call("PUT", `/v1/devices/${d.device_id}`, {
      auth: d.device_secret,
      body: { apns_token: "c".repeat(64), environment: "sandbox" },
    });
    expect((await call("POST", "/v1/events", { auth: l.push_token, body: event() })).json).toEqual({ sent: true });
    expect(apnsCalls[1].url).toContain("c".repeat(64));
  });

  it("asks hurad to retry when APNs fails, and the retry goes through", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    const e = event();
    apnsReplies.push(apnsError(503, "ServiceUnavailable"));
    const r = await call("POST", "/v1/events", { auth: l.push_token, body: e });
    expect(r.status).toBe(502);
    expect(r.json.error.code).toBe("apns_error");
    expect(r.json.error.message).toContain("ServiceUnavailable");

    apnsReplies.push(new Error("network down"));
    expect((await call("POST", "/v1/events", { auth: l.push_token, body: e })).status).toBe(502);

    expect((await call("POST", "/v1/events", { auth: l.push_token, body: e })).json).toEqual({ sent: true });
    expect(apnsCalls).toHaveLength(3);
  });

  it("refreshes an expired provider token once", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    // Two sends with the same cached token...
    await call("POST", "/v1/events", { auth: l.push_token, body: event() });
    await call("POST", "/v1/events", { auth: l.push_token, body: event() });
    expect(apnsCalls[0].headers.get("authorization")).toBe(apnsCalls[1].headers.get("authorization"));
    // ...then APNs rejects it: a fresh token is minted and the send retried.
    apnsReplies.push(apnsError(403, "ExpiredProviderToken"));
    expect((await call("POST", "/v1/events", { auth: l.push_token, body: event() })).json).toEqual({ sent: true });
    expect(apnsCalls).toHaveLength(4);
  });
});

describe("device test route", () => {
  it("reports what APNs said, which doubles as the connectivity check", async () => {
    const d = await device();
    apnsReplies.push(apnsError(400, "BadDeviceToken"));
    const r = await call("POST", `/v1/devices/${d.device_id}/test`, { auth: d.device_secret });
    expect(r.json).toEqual({ sent: false, apns_status: 400, apns_id: null, apns_reason: "BadDeviceToken" });
    expect(apnsCalls[0].body.aps.category).toBe("TEST");

    const ok = await call("POST", `/v1/devices/${d.device_id}/test`, { auth: d.device_secret });
    expect(ok.json).toMatchObject({ sent: true, apns_status: 200, apns_id: "id-1" });
  });
});

describe("apns", () => {
  it("reuses provider tokens for 50 minutes", async () => {
    const t0 = Date.UTC(2026, 9, 27, 12);
    const a = await providerToken(env, t0);
    expect(await providerToken(env, t0 + 49 * 60 * 1000)).toBe(a);
    const b = await providerToken(env, t0 + 51 * 60 * 1000);
    expect(b).not.toBe(a);
    expect((await verifyJWT(b)).claims.iat).toBe(Math.floor((t0 + 51 * 60 * 1000) / 1000));
  });

  it("only accepts a loopback origin override", () => {
    const withOverride = (o: string) => apnsOrigin({ ...env, APNS_ORIGIN_OVERRIDE: o }, "production");
    expect(withOverride("http://127.0.0.1:9999")).toBe("http://127.0.0.1:9999");
    expect(withOverride("http://localhost:8080")).toBe("http://localhost:8080");
    expect(withOverride("https://evil.example")).toBe("https://api.push.apple.com");
    expect(withOverride("http://127.0.0.1.evil.example")).toBe("https://api.push.apple.com");
    expect(withOverride("")).toBe("https://api.push.apple.com");
  });
});

describe("cleanup", () => {
  it("prunes old idempotency and registration rows", async () => {
    const d = await device();
    const l = await link(d.device_secret);
    const old = Date.now() - 25 * 3600 * 1000;
    await env.DB.prepare("INSERT INTO events (link_id, event_id, created_at) VALUES (?, 'old', ?)").bind(l.link_id, old).run();
    await env.DB.prepare("INSERT INTO registrations (ip, at) VALUES ('x', ?)").bind(old).run();
    await worker.scheduled({} as ScheduledController, env);
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM events WHERE event_id = 'old'").first("n")).toBe(0);
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM registrations WHERE ip = 'x'").first("n")).toBe(0);
  });
});
