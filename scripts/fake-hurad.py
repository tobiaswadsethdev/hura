#!/usr/bin/env python3
"""A stand-in hurad, for looking at the desktop and iPhone apps without a
server: two projects and their worktrees in every state, a poll for each, a
conversation in each chat, one of them waiting on a permission prompt, and the
working copy, git, events and policy behind the desktop's panes; and on the
next port, a stand-in for hura cloud, signed in, with two trackers and an
inbox. What the screenshots on gethura.app are taken against.

    python3 scripts/fake-hurad.py [port]

The desktop selects the first worktree in the list when it opens; set
FAKE_FIRST to a session's name to have that one first.

Not the protocol in full -- an op it does not know answers `unsupported`,
which the apps show as they would any other failure. The shapes are
hura-proto's: see apps/desktop/src/gen for each, since the desktop's client
reads them as strictly as Rust does.

It makes a certificate of its own, and a state directory with the desktop
already paired and signed in to the fake cloud, and prints how to launch each
app against it (for the iPhone, see `pairFromLaunchArguments` in
apps/ios/Hura/HuraApp.swift). The Files pane is this checkout. Python's
standard library and openssl only.
"""
import base64, hashlib, json, os, ssl, struct, subprocess, sys, tempfile, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18555
TOKEN = "fake-token"
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
now = int(time.time())

# ------------------------------------------------------------------ sessions

projects = [
    {"name": "hura", "repo": "git@github.com:tobiaswadsethdev/hura.git", "path": "/srv/hura", "created_at": now - 86400 * 40},
    {"name": "storefront", "repo": "git@github.com:acme/storefront.git", "path": "/srv/storefront", "created_at": now - 86400 * 20},
]
REPOS = {p["name"]: p["repo"] for p in projects}

def session(name, project, branch, state, hours, task, iface="chat", backend="sandbox", **extra):
    """A whole `Session`: every field, since the desktop's client has no defaults for some."""
    return {"name": name, "backend": backend, "sandbox": name, "tmux": "agent", "repo": REPOS[project],
            "project": project, "base_branch": "main", "work_branch": branch, "task": task, "ticket": None,
            "pull": None, "policy": "feature-work", "providers": ["claude"], "skills": [], "mcp": [],
            "toolchains": ["node"], "agent": "claude", "interface": iface, "created_at": now - int(hours * 3600),
            "state": state, "failure": None, **extra}

sessions = [
    session("docs-search", "hura", "hura/docs-search", "ready", 2,
            "Search for the docs site, built with it and loaded on first use"),
    session("relay-tls", "hura", "hura/relay-tls-rotation", "ready", 5,
            "Rotate the relay's TLS certificate without dropping connections"),
    session("ios-widgets", "hura", "hura/ios-widgets", "ready", 26,
            "A lock screen widget with the sessions waiting on you", iface="terminal"),
    session("price-rounding", "storefront", "tobias/SHOP-412", "ready", 1,
            "Checkout total rounds before tax instead of after",
            ticket={"tracker": "acme-jira", "kind": "jira", "id": "SHOP-412", "key": "SHOP-412",
                    "url": "https://acme.atlassian.net/browse/SHOP-412", "repo": None}),
    session("checkout-flow", "storefront", "tobias/checkout-flow", "ready", 30,
            "One-page checkout behind a flag", backend="worktree", workdir="/srv/storefront/.worktrees/checkout-flow"),
    session("saved-carts", "storefront", "tobias/SHOP-398", "seeding", 0.02, "Saved carts survive signing out"),
]

# The desktop opens on whichever comes first, which is how a screenshot picks one.
first = os.environ.get("FAKE_FIRST")
sessions.sort(key=lambda s: s["name"] != first)

def report(state, detail=None, prompt=None, ask=None):
    return {"state": state, "detail": detail, "source": "hook", "ask": ask, "prompt": prompt}

def usage(cost, minutes, added, removed, context, model="claude-opus-5-5"):
    return {"model": model, "version": "2.1.293", "cost_usd": cost, "duration_ms": minutes * 60000,
            "lines_added": added, "lines_removed": removed, "context_used_percentage": context,
            "context_size": 1000000,
            "windows": [{"label": "5h", "used_percentage": 34, "resets_at": now + 7200},
                        {"label": "7d", "used_percentage": 61, "resets_at": now + 86400 * 3}]}

polls = {
    "docs-search": (report("running", "Bash"), usage(4.12, 48, 412, 18, 41), (412, 18, 3)),
    "relay-tls": (report("waiting", "Edit", "Edit relay/src/tls.ts", "toolu_relay_edit"), usage(1.87, 22, 94, 31, 23), (94, 31, 0)),
    "ios-widgets": (report("idle"), usage(6.40, 95, 840, 122, 67), (840, 122, 5)),
    "price-rounding": (report("running", "Bash"), usage(0.92, 11, 23, 9, 12, "claude-sonnet-5-5"), (23, 9, 0)),
    "checkout-flow": (report("idle"), usage(2.35, 40, 310, 77, 30), (310, 77, 2)),
}

def poll(name):
    status, use, stat = polls.get(name, (None, None, None))
    return {"stat": stat and {"added": stat[0], "removed": stat[1], "untracked": stat[2]},
            "status": status, "pane": None, "usage": use,
            "ports": [{"port": 5173, "host": "v4"}] if name == "docs-search" else []}

# ------------------------------------------------------------- conversations

def prompt(text, uuid):
    return {"kind": "prompt", "text": text, "uuid": uuid, "files": [], "receipt": True}

def say(mid, text):
    return {"kind": "sdk", "msg": {"type": "assistant", "parent_tool_use_id": None,
                                   "message": {"id": mid, "content": [{"type": "text", "text": text}]}}}

def tool(tid, name, **input):
    return {"kind": "sdk", "msg": {"type": "assistant", "parent_tool_use_id": None,
                                   "message": {"id": "m_" + tid, "content": [{"type": "tool_use", "id": tid, "name": name, "input": input}]}}}

def result(tid, text, error=False):
    return {"kind": "sdk", "msg": {"type": "user", "parent_tool_use_id": None,
                                   "message": {"content": [{"type": "tool_result", "tool_use_id": tid, "content": text, "is_error": error}]}}}

def done(seconds, cost):
    return {"kind": "sdk", "msg": {"type": "result", "subtype": "success", "is_error": False,
                                   "duration_ms": seconds * 1000, "total_cost_usd": cost}}

TLS_OLD = """export function serve(cert: Certificate) {
  const server = tls.createServer({ cert: cert.pem, key: cert.key });
  return server;
}"""
TLS_NEW = """export function serve(cert: Certificate) {
  const server = tls.createServer({ cert: cert.pem, key: cert.key });
  // A renewed certificate is taken for new connections only, so the
  // phones already connected keep theirs until they reconnect.
  cert.onRenew((next) => server.setSecureContext({ cert: next.pem, key: next.key }));
  return server;
}"""

SEARCH_INDEX = """// Every page split into its sections, for the search dialog to load.
for (const page of pages) {
  for (const section of sections(page)) {
    index.push({ url: page.url + section.anchor, path: section.headings, text: section.text });
  }
}"""
RANK_OLD = """function score(hit: Hit): number {
  return hit.matches.length;
}"""
RANK_NEW = """function score(hit: Hit): number {
  // A match in a title or heading says more than one in the text under it.
  return hit.matches.reduce((sum, m) => sum + (m.in === "text" ? 1 : 3), 0);
}"""

conversations = {
    "docs-search": [
        prompt("The docs site needs search. Keep it static: no server, and nothing to run but the build.", "u1"),
        {"kind": "read", "uuid": "u1"},
        say("m1", "I'll build the index when the site builds, and load it only when someone opens search:\n\n"
                  "- a script that splits every page into its sections and writes `search-index.json`\n"
                  "- a **⌘K** dialog that fetches the index on first open\n"
                  "- matches ranked by where they are: title, then heading, then body"),
        tool("t1", "Read", file_path="/sandbox/repo/docs/.vitepress/config.ts"),
        result("t1", "export default defineConfig({\n  title: \"hura\",\n  themeConfig: { nav, sidebar },\n})"),
        tool("t2", "Write", file_path="/sandbox/repo/docs/scripts/build-search-index.ts", content=SEARCH_INDEX),
        result("t2", "File created successfully at: /sandbox/repo/docs/scripts/build-search-index.ts"),
        tool("t3", "Bash", command="npm run docs:build", description="Build the docs with the index"),
        result("t3", "vitepress v2.0.4\n✓ building client + server bundles...\n✓ rendering pages...\n"
                     "✓ indexed 142 pages, 1,206 sections (318 KB, 41 KB gzipped)\nbuild complete in 6.82s."),
        say("m2", "The index builds with the site: 142 pages in 41 KB gzipped, fetched the first time someone presses ⌘K."),
        done(286, 2.10),
        prompt("Rank a match in a heading above one in the text, and show which section it's in.", "u2"),
        {"kind": "read", "uuid": "u2"},
        say("m3", "Each section already keeps its heading path, so the result can show it as a breadcrumb. "
                  "I'll weight heading matches three times over body text."),
        tool("t4", "Edit", file_path="/sandbox/repo/docs/.vitepress/theme/search.ts", old_string=RANK_OLD, new_string=RANK_NEW),
        result("t4", "The file /sandbox/repo/docs/.vitepress/theme/search.ts has been updated."),
        tool("t5", "Bash", command="npm test -- search", description="Run the search tests"),
    ],
    "relay-tls": [
        prompt("The relay drops every phone's connection when its certificate renews. Rotate it without dropping anyone.", "u1"),
        {"kind": "read", "uuid": "u1"},
        tool("t1", "Read", file_path="/sandbox/repo/relay/src/tls.ts"),
        result("t1", TLS_OLD),
        say("m1", "The server is made once with the certificate it started with, and renewing restarts it. "
                  "`setSecureContext` swaps the certificate in place: new connections get the new one and open ones are left alone."),
        tool("t2", "Edit", file_path="/sandbox/repo/relay/src/tls.ts", old_string=TLS_OLD, new_string=TLS_NEW),
    ],
    "price-rounding": [
        prompt("SHOP-412: the checkout total rounds each line before tax instead of once after it.", "u1"),
        {"kind": "read", "uuid": "u1"},
        tool("t1", "Grep", pattern="Math.round", path="src/checkout"),
        result("t1", "src/checkout/total.ts:14:  const line = Math.round(item.price * item.qty * 100) / 100;\n"
                     "src/checkout/total.ts:22:  return Math.round(sum * (1 + rate) * 100) / 100;"),
        say("m1", "Line 14 rounds every line before tax is added, so a cart of many cheap items drifts by a cent or two. "
                  "I'll keep the lines exact and round once, after tax, and add the cart from the ticket as a test."),
        tool("t2", "Bash", command="npm test -- total", description="Run the checkout total tests"),
    ],
    "checkout-flow": [
        prompt("Put the one-page checkout behind the `one_page_checkout` flag.", "u1"),
        {"kind": "read", "uuid": "u1"},
        say("m1", "Done: the flag is read once per request in `routes/checkout.ts`, and both flows have a test."),
        done(840, 2.35),
    ],
}

ASKS = {
    "relay-tls": [{"id": "toolu_relay_edit", "tool": "Edit",
                   "input": {"file_path": "/sandbox/repo/relay/src/tls.ts", "old_string": TLS_OLD, "new_string": TLS_NEW},
                   "title": None, "description": None, "display_name": None, "reason": None, "blocked_path": None,
                   "can_remember": True, "default_to_no": False, "agent": None}],
}

MODELS = [{"value": v, "display_name": d, "description": "", "resolved": None, "efforts": ["low", "medium", "high"]}
          for v, d in [("opus", "Opus 5.5"), ("sonnet", "Sonnet 5.5"), ("haiku", "Haiku 5.5")]]

def chat_frames(name, cid):
    events = conversations.get(name, [])
    start = (now - 1800) * 1000
    entries = [{"seq": i + 1, "at": start + i * 45000, "ev": ev} for i, ev in enumerate(events)]
    pending = ASKS.get(name, [])
    status, use, _ = polls.get(name, (report("idle"), None, None))
    activity = "waiting" if pending else ("running" if status["state"] == "running" else "idle")
    state = {"activity": activity, "pending": pending, "mode": "default",
             "model": "sonnet" if name == "price-rounding" else "opus", "effort": "high", "models": MODELS,
             "commands": [], "status": None, "cost_usd": use and use["cost_usd"],
             "context_percentage": use and use["context_used_percentage"],
             "context_size": use and use["context_size"], "started": True, "files": True, "receipts": True}
    return [{"is": "chat", "id": cid, "frame": {"t": "replay", "seq": len(entries), "entries": entries}},
            {"is": "chat", "id": cid, "frame": {"t": "state", "state": state}}]

TERMINAL = ("\x1b[1;32m~/repo\x1b[0m on \x1b[1;35mhura/ios-widgets\x1b[0m\r\n$ claude\r\n\r\n"
            " \x1b[38;5;214m✻\x1b[0m Welcome back!\r\n\r\n"
            "> Add a lock screen widget with the sessions waiting on you\r\n\r\n"
            "\x1b[38;5;245m⏺\x1b[0m The widget reads the same poll the Servers tab does, through the app group,\r\n"
            "  and shows up to three sessions waiting on a prompt.\r\n\r\n"
            "\x1b[38;5;245m⏺\x1b[0m \x1b[1mUpdate\x1b[0m(apps/ios/HuraWidgets/Waiting.swift)\r\n"
            "  ⎿  Updated with \x1b[32m84 additions\x1b[0m\r\n\r\n"
            "\x1b[38;5;245m⏺\x1b[0m Built and ran in the simulator; the widget refreshes when a push arrives.\r\n\r\n"
            "\x1b[38;5;240m────────────────────────────────────────────────────────────\x1b[0m\r\n> ")

# ---------------------------------------------------------- the dock's panes

def ev(minutes_ago, subject, verdict, count=1, cls="NET:OPEN", policy=None, reason=None):
    at = now - minutes_ago * 60
    return {"at": at - count * 30, "count": count, "last": at, "class": cls, "severity": "Info",
            "verdict": verdict, "subject": subject, "policy": policy, "reason": reason}

events = [
    ev(1, "api.anthropic.com:443", "Allowed", 214, policy="feature-work"),
    ev(2, "github.com:443", "Allowed", 6, policy="feature-work"),
    ev(3, "GET github.com:443/settings/tokens", "Denied", cls="HTTP:GET", reason="no rule allows it"),
    ev(4, "registry.npmjs.org:443", "Allowed", 38, policy="node"),
    ev(6, "pastebin.com:443", "Denied", 2, reason="no rule allows it"),
    ev(9, "fonts.googleapis.com:443", "Denied", 4, reason="no rule allows it"),
    ev(31, "archive.ubuntu.com:80", "Denied", 3, reason="no rule allows it"),
]

def rule(id, hosts, methods=(), path=None, allow=True, glob=False):
    return {"id": id, "allow": allow, "hosts": hosts, "methods": list(methods), "path": path, "global": glob, "watch": False}

policy = {
    "template": "feature-work",
    "rules": [
        rule("a1", ["api.anthropic.com:443"]),
        rule("a2", ["github.com:443"], ["GET", "POST"], "/**/git-upload-pack"),
        rule("a3", ["github.com:443"], ["POST"], "/**/git-receive-pack"),
        rule("a4", ["github.com:443"], ["GET"], "/**/info/refs"),
        rule("a5", ["api.github.com:443"], ["GET"], "/repos/**"),
        rule("a6", ["registry.npmjs.org:443"]),
        rule("a7", ["codeload.github.com:443"], ["GET"]),
        rule("a8", ["pastebin.com:443"], allow=False, glob=True),
    ],
    "watching": [], "lists": None,
}

git_status = {
    "branch": "hura/docs-search", "upstream": "origin/hura/docs-search", "ahead": 2, "behind": 0,
    "staged": [{"path": "docs/scripts/build-search-index.ts", "change": "added"}, {"path": "docs/package.json", "change": "modified"}],
    "unstaged": [{"path": "docs/.vitepress/theme/search.ts", "change": "modified"}, {"path": "docs/.vitepress/theme/Search.vue", "change": "untracked"}],
}

def read_repo(path):
    try:
        with open(os.path.join(REPO_ROOT, path), encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""

SKIP = {".git", "target", "node_modules", "dist", ".DS_Store"}
def files(path):
    full = os.path.join(REPO_ROOT, path)
    names = sorted(n for n in os.listdir(full) if n not in SKIP) if os.path.isdir(full) else []
    entries = [{"name": n, "dir": os.path.isdir(os.path.join(full, n))} for n in names]
    entries.sort(key=lambda e: (not e["dir"], e["name"].lower()))
    return {"path": path, "entries": entries}

def file(path):
    text = read_repo(path)
    return {"path": path, "text": text[:200000], "bytes": len(text.encode()), "truncated": len(text) > 200000, "binary": False}

# Both sides of each file git_status names, since none of them is in this checkout.
PACKAGE_OLD = """{
  "scripts": {
    "docs:dev": "vitepress dev",
    "docs:build": "vitepress build"
  }
}"""
PACKAGE_NEW = """{
  "scripts": {
    "docs:dev": "vitepress dev",
    "docs:build": "tsx scripts/build-search-index.ts && vitepress build"
  }
}"""
SEARCH_VUE = """<script setup lang="ts">
const open = ref(false);
const results = computed(() => search(query.value).slice(0, 8));
onKeyStroke("k", (e) => (e.metaKey || e.ctrlKey) && (open.value = true));
</script>"""
DIFFS = {
    "docs/scripts/build-search-index.ts": ("", SEARCH_INDEX),
    "docs/package.json": (PACKAGE_OLD, PACKAGE_NEW),
    "docs/.vitepress/theme/search.ts": (RANK_OLD, RANK_NEW),
    "docs/.vitepress/theme/Search.vue": ("", SEARCH_VUE),
}

def git_diff(path, against):
    old, new = DIFFS.get(path, ("", read_repo(path)))
    labels = {"staged": ("HEAD", "index"), "worktree": ("index", "working copy"), "base": ("main", "working copy")}[against]
    return {"path": path, "original": old, "modified": new, "original_label": labels[0], "modified_label": labels[1], "binary": False}

ports = {
    "listening": [{"port": 5173, "host": "v4", "owner": {"pid": 412, "command": "node vitepress dev docs --host", "certain": True}}],
    "processes": [{"pid": 412, "ppid": 1, "command": "node vitepress dev docs --host", "protected": False}],
    "forwards": [],
}

def answer(req):
    op, name = req.get("op"), req.get("name")
    if op == "ls": return {"reply": "ls", "sessions": sessions, "adopted": [], "dead": [], "warnings": []}
    if op == "projects": return {"reply": "projects", "projects": projects}
    if op == "poll": return {"reply": "poll", **poll(name)}
    if op == "shells": return {"reply": "shells", "shells": []}
    if op == "chats": return {"reply": "chats", "chats": ["agent"], "opened": None}
    if op == "events": return {"reply": "events", "events": events}
    if op == "policy": return {"reply": "policy", **policy}
    if op == "git-status": return {"reply": "git", "said": "", "status": git_status}
    if op == "git": return {"reply": "git", "said": "Everything up-to-date", "status": git_status}
    if op == "git-diff": return {"reply": "git-diff", **git_diff(req["path"], req["against"])}
    if op == "files": return {"reply": "files", **files(req.get("path", ""))}
    if op == "file": return {"reply": "file", **file(req["path"])}
    if op == "comments": return {"reply": "comments", "comments": []}
    if op == "ports": return {"reply": "ports", **ports}
    if op == "diff": return {"reply": "diff", "body": ""}
    if op == "settings":
        return {"reply": "settings", "path": "/home/tobias/.config/hura/config.toml", "present": True,
                "default_branch_prefix": "hura/",
                "settings": {"branch_prefix": None, "base": None, "policy": "feature-work", "providers": ["claude"],
                             "auto_update": True, "interface": "chat"}}
    return None

# ------------------------------------------------------------------ websocket

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
                kind, name = ch.get("kind"), ch.get("session")
                ws_send(self.wfile, {"is": "opened", "id": cid})
                if kind == "chat":
                    for frame in chat_frames(name, cid): ws_send(self.wfile, frame)
                elif kind == "status":
                    ws_send(self.wfile, {"is": "status", "id": cid, "poll": poll(name)})
                elif kind == "events":
                    ws_send(self.wfile, {"is": "events", "id": cid, "events": events})
                elif kind == "terminal":
                    ws_send(self.wfile, {"is": "output", "id": cid, "data": base64.b64encode(TERMINAL.encode()).decode()})
        except (ConnectionError, OSError):
            return

    def log_message(self, fmt, *a): sys.stderr.write("%s\n" % (fmt % a))
    def send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_GET(self):
        if self.path == "/ws": return self.ws()
        if self.path == "/version": return self.send(200, {"server": "hurad", "protocol": 4, "version": "0.40.1"})
        self.send(404, {})
    def do_POST(self):
        if self.headers.get("Authorization") != f"Bearer {TOKEN}": return self.send(401, {})
        req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        ok = answer(req)
        if ok is None:
            print("unknown op:", req.get("op"), flush=True)
            return self.send(200, {"error": {"kind": "unsupported", "message": f"the fake server has no {req.get('op')}"}})
        self.send(200, {"ok": ok})

# ------------------------------------------------------------------ hura cloud

# Plain HTTP, which both apps allow to a loopback address. Signed in with any
# code; the trackers' tickets and pull requests are canned.
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
            "repo_url": "https://github.com/acme/storefront.git", "relation": "assigned" if review else "created",
            "review_requested": review, "author": "sam" if review else "tobias", "branch": f"feature/{n}",
            "base": "main", "draft": draft, "fork": False, "conflicts": conflicts, "checks": checks,
            "review": verdict, "comments": comments, "updated": iso(hours)}
inbox = {
    "tasks": [
        task("SHOP-412", "Checkout total rounds before tax instead of after", "In Progress", "assigned", 2, "Bug", 3, "Sam", session="price-rounding"),
        task("SHOP-398", "Saved carts survive signing out", "To Do", "assigned", 26, session="saved-carts"),
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

threading.Thread(target=ThreadingHTTPServer(("127.0.0.1", CLOUD), Cloud).serve_forever, daemon=True).start()

# ------------------------------------------------------------------- starting

# A certificate of its own, made fresh each run: the pairing string pins it.
cert_dir = tempfile.mkdtemp(prefix="fake-hurad-")
cert, key = os.path.join(cert_dir, "cert.pem"), os.path.join(cert_dir, "key.pem")
subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
                "-keyout", key, "-out", cert, "-days", "7", "-subj", "/CN=fake-hurad",
                "-addext", "subjectAltName=IP:127.0.0.1"], check=True, capture_output=True)
der = subprocess.run(["openssl", "x509", "-in", cert, "-outform", "der"], check=True, capture_output=True).stdout
fingerprint = hashlib.sha256(der).hexdigest()
pairing = f"hura://127.0.0.1:{PORT}/{TOKEN}#{fingerprint}"

# The desktop's state, paired and signed in, in a directory of its own so the
# real list of servers and cloud session are never touched: XDG_STATE_HOME is
# where the desktop reads both from (hura_core::state::dir).
state = os.path.join(cert_dir, "state", "hura")
os.makedirs(state, mode=0o700)
for name, value in [
    ("remotes.json", {"remotes": [{"name": "devbox", "host": "127.0.0.1", "port": PORT, "token": TOKEN, "fingerprint": fingerprint}]}),
    ("cloud.json", {"base": f"http://127.0.0.1:{CLOUD}", "email": "tobias@example.com", "account": "acct_1",
                    "device": "dev_2", "refresh_token": "refresh"}),
]:
    with open(os.path.join(state, name), "w") as f: json.dump(value, f)

ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain(cert, key)
srv = ThreadingHTTPServer(("127.0.0.1", PORT), H); srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
print(f"""fake hurad on 127.0.0.1:{PORT}, fake hura cloud on 127.0.0.1:{CLOUD}.

The desktop app, already paired and signed in:

    XDG_STATE_HOME={os.path.dirname(state)} target/debug/hura-desktop

A Debug build of the iPhone app installed in a booted simulator:

    xcrun simctl launch booted dev.tobiaswadseth.hura.ios -HuraPair '{pairing}' \\
        -HuraName devbox -HuraCloud http://127.0.0.1:{CLOUD} -HuraNoLock YES

and `-HuraOpen server`, `inbox`, `account` or a session's name to go straight to a screen.""", flush=True)
srv.serve_forever()
