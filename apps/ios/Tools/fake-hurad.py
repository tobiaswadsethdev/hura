#!/usr/bin/env python3
"""A stand-in hurad, for looking at the iPhone app in a simulator without a
server: a few projects and sessions in every state, a poll for each, and a
conversation to scroll through; and on the next port, a stand-in for hura
cloud, signed in, with two trackers and an inbox. Not the protocol in full -- an op it does not
know answers with a failure, which the app shows as it would any other.

    python3 apps/ios/Tools/fake-hurad.py [port]

It makes a certificate of its own and prints the command that launches a Debug
build paired with it (see `pairFromLaunchArguments` in HuraApp.swift). Python's
standard library and openssl only.
"""
import hashlib, json, os, ssl, subprocess, sys, tempfile, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18555
TOKEN = "fake-token"
now = int(time.time())

projects = [
    {"name": "hura", "repo": "git@github.com:tobiaswadsethdev/hura.git", "path": "/srv/hura", "created_at": now - 86400 * 40},
    {"name": "storefront", "repo": "git@github.com:acme/storefront.git", "path": "/srv/storefront", "created_at": now - 86400 * 20},
]
def session(name, project, branch, state, hours, iface="chat", **extra):
    return {"name": name, "state": state, "work_branch": branch, "task": "Make the thing", "interface": iface,
            "project": project, "repo": "git@github.com:tobiaswadsethdev/hura.git", "base_branch": "main",
            "created_at": now - int(hours * 3600), "sandbox": name, "backend": "sandbox", "agent": "claude",
            "toolchains": ["node"], "providers": ["anthropic"], "policy": "default", **extra}
sessions = [
    session("desktop-overhaul", "hura", "hura/desktop-overhaul", "ready", 5),
    session("ios-sign-out", "hura", "hura/ios-sign-out-alert", "ready", 26, iface="terminal"),
    session("relay-tls", "hura", "hura/relay-tls-rotation", "ready", 14),
    session("checkout-flow", "storefront", "tobias/checkout-flow", "ready", 30, backend="worktree"),
    session("price-rounding", "storefront", "tobias/fix-price-rounding", "creating", 0.01),
    session("old-spike", "storefront", "tobias/spike", "failed", 72, failure="sandbox not found"),
]
reports = {
    "desktop-overhaul": {"state": "running", "detail": None},
    "ios-sign-out": {"state": "waiting", "detail": "Bash", "prompt": "Bash: npm test"},
    "relay-tls": {"state": "idle"},
    "checkout-flow": {"state": "idle"},
}
usage = {"model": "opus", "version": "2.1", "cost_usd": 3.21, "duration_ms": 5400000, "lines_added": 412,
         "lines_removed": 233, "context_used_percentage": 37, "context_size": 200000,
         "windows": [{"label": "5h", "used_percentage": 42, "resets_at": now + 7200},
                     {"label": "7d", "used_percentage": 83, "resets_at": now + 86400 * 3}]}

def answer(req):
    op = req.get("op")
    if op == "ls": return {"sessions": sessions}
    if op == "projects": return {"projects": projects}
    if op == "poll":
        n = req.get("name")
        return {"status": reports.get(n), "usage": usage if n in reports else None,
                "stat": {"added": 412, "removed": 233, "untracked": 3} if n in reports else None, "ports": []}
    if op == "shells": return {"shells": []}
    if op == "chats": return {"chats": ["agent"], "opened": None}
    return None

import base64, struct

def ws_frames(entries_at):
    at = entries_at
    e = lambda seq, ev: {"seq": seq, "at": at + seq * 20000, "ev": ev}
    sdk = lambda msg: {"kind": "sdk", "msg": msg}
    return [
        e(1, {"kind": "prompt", "text": "The desktop app has drifted from the iOS app. Give it a sleek, minimal look, like something Apple made.", "uuid": "u1", "files": [], "receipt": False}),
        e(2, {"kind": "read", "uuid": "u1"}),
        e(3, sdk({"type": "assistant", "message": {"id": "m1", "content": [{"type": "text", "text": "I'll start with the **design tokens** in `style.css`, then the window chrome:\n\n- an overlay title bar with the traffic lights inset\n- a translucent sidebar\n- the system accent for focus and primary actions"}]}})),
        e(4, sdk({"type": "assistant", "message": {"id": "m2", "content": [{"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "/sandbox/repo/apps/desktop/src/style.css"}}]}})),
        e(5, sdk({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": "3772 lines"}]}})),
        e(6, sdk({"type": "assistant", "message": {"id": "m3", "content": [{"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "npx tsc --noEmit", "description": "Typecheck"}}]}})),
        e(7, sdk({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t2", "content": ""}]}})),
        e(8, sdk({"type": "assistant", "message": {"id": "m4", "content": [{"type": "text", "text": "Typecheck passes. The sidebar now uses the window's material on macOS, and every popover shares one elevation."}]}})),
        e(9, {"kind": "prompt", "text": "Now the iPhone app too.", "uuid": "u2", "files": [], "receipt": False}),
    ]

def ws_send(w, obj):
    data = json.dumps(obj).encode()
    n = len(data)
    head = bytes([0x81]) + (bytes([n]) if n < 126 else bytes([126]) + struct.pack(">H", n) if n < 65536 else bytes([127]) + struct.pack(">Q", n))
    w.write(head + data); w.flush()

def ws_recv(r):
    def read(k):
        b = r.read(k)
        if len(b) < k: raise ConnectionError
        return b
    h = read(2); op = h[0] & 0x0F; n = h[1] & 0x7F
    if n == 126: n = struct.unpack(">H", read(2))[0]
    elif n == 127: n = struct.unpack(">Q", read(8))[0]
    mask = read(4) if h[1] & 0x80 else b"\0\0\0\0"
    data = bytes(b ^ mask[i % 4] for i, b in enumerate(read(n)))
    return op, data

class H(BaseHTTPRequestHandler):
    # A WebSocket upgrade is HTTP/1.1's; the default 1.0 status line is refused.
    protocol_version = "HTTP/1.1"
    def ws(self):
        key = self.headers.get("Sec-WebSocket-Key", "")
        accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        self.send_response(101); self.send_header("Upgrade", "websocket"); self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept); self.end_headers(); self.wfile.flush()
        try:
            while True:
                op, data = ws_recv(self.rfile)
                if op == 8: return
                if op != 1: continue
                req = json.loads(data)
                if req.get("do") != "open": continue
                cid, ch = req["id"], req.get("channel", {})
                ws_send(self.wfile, {"is": "opened", "id": cid})
                if ch.get("kind") == "chat":
                    ws_send(self.wfile, {"is": "chat", "id": cid, "frame": {"t": "replay", "seq": 9, "entries": ws_frames(int(time.time() * 1000) - 600000)}})
                    ws_send(self.wfile, {"is": "chat", "id": cid, "frame": {"t": "state", "state": {"activity": "running", "pending": [], "mode": "default", "model": "opus", "effort": None, "models": [{"value": "opus", "displayName": "Opus", "description": ""}], "commands": [], "status": None, "cost_usd": 1.23, "context_percentage": 37, "started": True, "files": True, "receipts": True}}})
        except (ConnectionError, OSError):
            return

    def log_message(self, fmt, *a): sys.stderr.write("%s\n" % (fmt % a))
    def send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_GET(self):
        if self.path == "/ws": return self.ws()
        if self.path == "/version": return self.send(200, {"server": "hurad", "protocol": 1, "version": "0.39.0"})
        self.send(404, {})
    def do_POST(self):
        if self.headers.get("Authorization") != f"Bearer {TOKEN}": return self.send(401, {})
        req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        ok = answer(req)
        if ok is None:
            print("unknown op:", req.get("op"), flush=True)
            return self.send(200, {"error": {"kind": "unsupported", "message": f"the fake server has no {req.get('op')}"}})
        self.send(200, {"ok": ok})

# hura cloud: plain HTTP, which the app allows to a loopback address. Signed in
# with any code; the trackers' tickets and pull requests are canned.
CLOUD = PORT + 1
iso = lambda hours: time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(now - hours * 3600))
def task(key, title, status, relation, hours, item_type="Story", comments=None, by=None, mine=False, session=None):
    return {"tracker": "acme-jira", "kind": "jira", "id": key, "key": key, "title": title,
            "url": f"https://acme.atlassian.net/browse/{key}", "status": status, "item_type": item_type,
            "session_name": session or key.lower(), "branch": f"tobias/{key}", "repo": None, "relation": relation,
            "filters": [], "updated": iso(hours), "comments": comments, "last_commenter": by, "last_comment_mine": mine}
def pull(n, title, review, hours, checks="passing", verdict=None, draft=False, conflicts=False, comments=None):
    return {"tracker": "github", "kind": "git-hub", "id": str(n), "key": f"#{n}", "title": title,
            "url": f"https://github.com/acme/storefront/pull/{n}", "repo": "acme/storefront",
            "repo_url": "https://github.com/acme/storefront.git", "relation": "review" if review else "authored",
            "review_requested": review, "author": "sam" if review else "tobias", "branch": f"feature/{n}",
            "base": "main", "draft": draft, "fork": False, "conflicts": conflicts, "checks": checks,
            "review": verdict, "comments": comments, "updated": iso(hours)}
inbox = {
    "tasks": [
        task("SHOP-412", "Checkout total rounds before tax instead of after", "In Progress", "assigned", 2, "Bug", 3, "Sam"),
        task("SHOP-398", "Saved carts survive signing out", "To Do", "assigned", 26),
        task("SHOP-377", "Order confirmation email shows the wrong currency", "In Review", "assigned", 50, "Bug", 1, mine=True),
        task("SHOP-350", "Spike: one-page checkout", "To Do", "created", 120, "Task"),
    ],
    "pulls": [
        pull(231, "Price rounding: round once, after tax", False, 1, "pending", comments=2),
        pull(229, "Cart: keep saved carts across sessions", False, 20, "failing", conflicts=True),
        pull(233, "Search: debounce the filter input", True, 3, "passing", comments=4),
        pull(227, "Payments: retry declined 3DS once", True, 30, "passing", "approved"),
    ],
    "warnings": [], "read": ["acme-jira"], "pulls_read": ["github"], "filters_read": [],
}
trackers = [
    {"source": {"kind": "jira", "name": "acme-jira", "site": "acme.atlassian.net", "email": "tobias@example.com",
                "hide_created": False, "jql_filters": []}, "token_set": True, "connected": None},
    {"source": {"kind": "git-hub", "name": "github", "repo": None, "hide_created": False, "jql_filters": []},
     "token_set": False, "connected": "@tobias"},
]
def cloud(method, path, body):
    if path == "/v1/sign-in/email": return {}
    if path in ("/v1/sign-in/email/verify", "/v1/token"):
        return {"account": "acct_1", "device": "dev_1", "refresh_token": "refresh", "access_token": "access", "expires_in": 3600}
    if path == "/v1/me": return {"provider": "email", "email": "tobias@example.com", "account": "acct_1", "plan": "free",
                                 "sign_ins": [{"provider": "email", "email": "tobias@example.com"}]}
    if path == "/v1/devices": return [
        {"id": "dev_1", "name": "iPhone 17 Pro", "platform": "ios", "created_at": now - 86400 * 30, "last_seen_at": now, "current": True},
        {"id": "dev_2", "name": "Tobias's MacBook Pro", "platform": "macos", "created_at": now - 86400 * 60, "last_seen_at": now - 3600, "current": False}]
    if path == "/v1/seen": return {"items": [], "at": None}
    if path == "/v1/push": return {"configured": False, "broken": None}
    if path == "/v1/trackers": return trackers
    if path == "/v1/connect": return {"git-hub": True, "jira": True}
    if path == "/v1/tracker":
        if body.get("op") == "inbox": return {"ok": inbox}
        return {"error": f"the fake cloud has no {body.get('op')}"}
    return {}

class Cloud(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, fmt, *a): sys.stderr.write("cloud: %s\n" % (fmt % a))
    def answer(self):
        n = int(self.headers.get("Content-Length", 0) or 0)
        body = json.loads(self.rfile.read(n) or b"{}") if n else {}
        data = json.dumps(cloud(self.command, self.path, body)).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)
    do_GET = do_POST = do_PUT = do_DELETE = answer

import threading
threading.Thread(target=ThreadingHTTPServer(("127.0.0.1", CLOUD), Cloud).serve_forever, daemon=True).start()

# A certificate of its own, made fresh each run: the pairing string pins it.
cert_dir = tempfile.mkdtemp(prefix="fake-hurad-")
cert, key = os.path.join(cert_dir, "cert.pem"), os.path.join(cert_dir, "key.pem")
subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
                "-keyout", key, "-out", cert, "-days", "7", "-subj", "/CN=fake-hurad",
                "-addext", "subjectAltName=IP:127.0.0.1"], check=True, capture_output=True)
der = subprocess.run(["openssl", "x509", "-in", cert, "-outform", "der"], check=True, capture_output=True).stdout
pairing = f"hura://127.0.0.1:{PORT}/{TOKEN}#{hashlib.sha256(der).hexdigest()}"

ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain(cert, key)
srv = ThreadingHTTPServer(("127.0.0.1", PORT), H); srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
print(f"""fake hurad on 127.0.0.1:{PORT}. With a Debug build installed in a booted simulator:

    xcrun simctl launch booted dev.tobiaswadseth.hura.ios -HuraPair '{pairing}' \\
        -HuraCloud http://127.0.0.1:{CLOUD} -HuraNoLock YES

and `-HuraOpen server`, `inbox`, `account` or a session's name to go straight to a screen.""", flush=True)
srv.serve_forever()
