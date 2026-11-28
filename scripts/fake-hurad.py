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

A terminal shows a canned screen. With FAKE_TMUX=1 it is a real tmux instead,
on a socket of its own (`tmux -L hura-fake`), set up as hurad sets up the
agent's -- no status line, the mouse on, copies passed out as OSC 52 -- with a
few hundred lines of history to scroll back through. For looking at what the
terminals do with the mouse, the wheel and a selection.

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
import base64, fcntl, hashlib, json, os, pty, socket, ssl, struct, subprocess, sys, tempfile, termios, threading, time
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

# What is waiting: a session queued for room and one scheduled for later,
# and messages for three of the sessions above -- one for after its turn, and
# the rest at times -- so each list shows a row's mark in every form.
def queued(id, name, project, task, at=None, repeat=None, runs=0, last=None):
    new = {"project": project, "name": name, "title": None, "repo": REPOS[project], "task": task, "base": None,
           "branch": None, "policy": "feature-work", "providers": ["claude"], "toolchains": ["node"], "start": True,
           "ticket": None, "pull": None, "interface": "chat", "worktree": False}
    return {"id": id, "new": new, "at": at, "added": now - 600, "started": False, "failed": None,
            "repeat": repeat, "runs": runs, "last": last}

def next_time(repeat, after):
    """When a repeating one next comes round, on this machine's clock."""
    import datetime as dt
    if repeat["type"] == "interval":
        return after + repeat["minutes"] * 60
    base = dt.datetime.fromtimestamp(after)
    for ahead in range(8):
        day = (base + dt.timedelta(days=ahead)).replace(hour=repeat["hour"], minute=repeat["minute"], second=0, microsecond=0)
        if (not repeat["weekdays"] or (day.isoweekday() % 7) in repeat["weekdays"]) and day.timestamp() > after:
            return int(day.timestamp())
    return None

def message(id, session, text, at=None, repeat=None, runs=0):
    return {"id": id, "session": session, "text": text, "at": at, "added": now - 300, "repeat": repeat, "runs": runs}

queue = {
    "sessions": [
        queued(1, "release-notes", "hura", "Draft the release notes for the next hurad"),
        queued(2, "nightly-deps", "storefront", "Update dependencies and run the suite",
               at=next_time({"type": "fixed_time", "hour": 2, "minute": 0, "weekdays": [1, 2, 3, 4, 5]}, now),
               repeat={"type": "fixed_time", "hour": 2, "minute": 0, "weekdays": [1, 2, 3, 4, 5]},
               runs=3, last="nightly-deps-3"),
    ],
    "messages": [
        message(3, "docs-search", "Now add a keyboard shortcut to open the search"),
        message(4, "relay-tls", "Check the rotation held overnight",
                at=next_time({"type": "fixed_time", "hour": 9, "minute": 0, "weekdays": []}, now),
                repeat={"type": "fixed_time", "hour": 9, "minute": 0, "weekdays": []}, runs=4),
        message(5, "relay-tls", "Then open a pull request", at=now + 4 * 3600),
        message(6, "price-rounding", "Re-run the checkout tests against staging", at=now + 86400 * 2),
    ],
    "limit": 2,
    # This machine's offset: a server that can repeat a worktree says it.
    "utc_offset": int(time.localtime().tm_gmtoff),
}

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
    "price-rounding": (report("waiting", "AskUserQuestion", "Where should a price be rounded?", "toolu_price_ask"),
                       usage(0.92, 11, 23, 9, 12, "claude-sonnet-5-5"), (23, 9, 0)),
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

def system(subtype, **fields):
    """A Claude Code system event, as the transcript keeps it: a background
    task starting or ending."""
    return {"kind": "sdk", "msg": {"type": "system", "subtype": subtype, **fields}}

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
        tool("t3b", "Bash", command="npm run docs:dev -- --port 5173", description="Serve the docs with search",
             run_in_background=True),
        result("t3b", "Command running in background with ID: b7k2x9. "
                      "Output is being written to: /tmp/claude-1000/tasks/b7k2x9.output"),
        system("task_started", task_id="b7k2x9", tool_use_id="t3b", task_type="local_bash",
               description="Serve the docs with search"),
        tool("t3c", "Bash", command="npx lychee docs --offline", description="Check the docs' links",
             run_in_background=True),
        result("t3c", "Command running in background with ID: b4m1q8. "
                      "Output is being written to: /tmp/claude-1000/tasks/b4m1q8.output"),
        system("task_started", task_id="b4m1q8", tool_use_id="t3c", task_type="local_bash",
               description="Check the docs' links"),
        system("task_notification", task_id="b4m1q8", status="completed",
               summary="Checked 1,206 links in 142 pages: none broken.",
               output_file="/tmp/claude-1000/tasks/b4m1q8.output"),
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
        # A turn you stopped: the call refused as Claude Code records an
        # interrupt, and the turn ended with `error_during_execution`.
        prompt("Now migrate every checkout test to the new flow.", "u2"),
        {"kind": "read", "uuid": "u2"},
        tool("t9", "Bash", command="npm test -- checkout", description="Run every checkout test"),
        result("t9", "The user doesn't want to proceed with this tool use. The tool use was rejected.", error=True),
        {"kind": "sdk", "msg": {"type": "user", "parent_tool_use_id": None,
                                "message": {"content": [{"type": "text", "text": "[Request interrupted by user]"}]}}},
        {"kind": "sdk", "msg": {"type": "result", "subtype": "error_during_execution", "is_error": True,
                                "duration_ms": 97000, "total_cost_usd": 0.4}},
    ],
}

ASKS = {
    "relay-tls": [{"id": "toolu_relay_edit", "tool": "Edit",
                   "input": {"file_path": "/sandbox/repo/relay/src/tls.ts", "old_string": TLS_OLD, "new_string": TLS_NEW},
                   "title": None, "description": None, "display_name": None, "reason": None, "blocked_path": None,
                   "can_remember": True, "default_to_no": False, "agent": None}],
    # A question with as many choices as Claude usually offers, which is
    # taller than the room left above the keyboard.
    "price-rounding": [{"id": "toolu_price_ask", "tool": "AskUserQuestion",
                        "input": {"questions": [{
                            "question": "Where should a price be rounded: once on the total, or on each line?",
                            "header": "Rounding", "multiSelect": False,
                            "options": [
                                {"label": "Once, after tax", "description": "Round the order total only; lines keep their exact amounts."},
                                {"label": "Each line", "description": "Round every line, then add them up, as the receipts do now."},
                                {"label": "Each line, before tax", "description": "Round lines before tax and the tax once on the total."},
                                {"label": "Ask finance", "description": "Leave it as it is until finance says which the books expect."}]}]},
                        "title": None, "description": None, "display_name": None, "reason": None, "blocked_path": None,
                        "can_remember": False, "default_to_no": False, "agent": None}],
}

MODELS = [{"value": v, "display_name": d, "description": "", "resolved": None, "efforts": ["low", "medium", "high"]}
          for v, d in [("opus", "Opus 5.5"), ("sonnet", "Sonnet 5.5"), ("haiku", "Haiku 5.5")]]

# A few of Claude Code's own, and a skill, for the composer's `/` menu.
SLASH_COMMANDS = [{"name": n, "description": d, "argument_hint": h, "aliases": a} for n, d, h, a in [
    ("compact", "Clear the conversation but keep a summary of it in context", "<instructions>", []),
    ("context", "Show what is using the context window", "", []),
    ("review", "Review a pull request", "<number>", []),
    ("security-review", "Review the pending changes on this branch for security issues", "", []),
    ("usage", "Show the session's cost and the account's limits", "", ["cost"]),
    ("frontend-design", "Guidance for intentional visual design (skill)", "", []),
]]

# The conversations opened beside each session's first, by session. A new one
# (a side chat, or one forked from an earlier message) starts empty here.
CHATS = {}
# What a shell was opened to run (a project's script), by session and shell.
SHELL_RUNS = {}

def chat_frames(name, cid, conv=None):
    events = conversations.get(name, []) if conv in (None, "agent") else []
    start = (now - 1800) * 1000
    entries = [{"seq": i + 1, "at": start + i * 45000, "ev": ev} for i, ev in enumerate(events)]
    pending = ASKS.get(name, [])
    status, use, _ = polls.get(name, (report("idle"), None, None))
    activity = "waiting" if pending else ("running" if status["state"] == "running" else "idle")
    state = {"activity": activity, "pending": pending, "mode": "default",
             "model": "sonnet" if name == "price-rounding" else "opus", "effort": "high", "models": MODELS,
             "commands": [c["name"] for c in SLASH_COMMANDS], "slash_commands": SLASH_COMMANDS,
             "status": None, "cost_usd": use and use["cost_usd"],
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

# With FAKE_TMUX, a real tmux behind each terminal channel. See the top.
FAKE_TMUX = os.environ.get("FAKE_TMUX") == "1"
TMUX = ["tmux", "-L", "hura-fake", "-f", "/dev/null"]
HISTORY = ("for i in $(seq 1 300); do printf '\\033[38;5;%dm%3d\\033[0m  history line, scroll back to read it\\n' $((i % 6 + 31)) $i; done; "
           "printf '\\033[1;32m~/repo\\033[0m on \\033[1;35mhura/ios-widgets\\033[0m  -- the live screen\\n'")

def tmux_attach(name, cols=100, rows=30):
    """A pty with a tmux client in it, attached to `name`'s session."""
    target = "fake-" + name
    if subprocess.run(TMUX + ["has-session", "-t", "=" + target], capture_output=True).returncode != 0:
        subprocess.run(TMUX + ["new-session", "-d", "-s", target, "-x", "200", "-y", "50"], check=True)
        for opt in (["status", "off"], ["mouse", "on"]):
            subprocess.run(TMUX + ["set", "-t", "=" + target + ":", *opt], check=True)
        subprocess.run(TMUX + ["set", "-s", "set-clipboard", "on"], check=True)
        # Typed once the shell is up, or it is typed into nothing.
        time.sleep(1)
        subprocess.run(TMUX + ["send-keys", "-t", "=" + target + ":", HISTORY, "Enter"], check=True)
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.execvp(TMUX[0], TMUX + ["-u", "attach", "-t", "=" + target])
    tty_size(fd, cols, rows)
    return pid, fd

def tty_size(fd, cols, rows):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

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
# The shells opened beside each session's agent, by session.
SHELLS = {}

# What the background commands above have written, for the panel that
# follows one. The dev server's log grows by a line every few seconds.
STARTED = time.time()
def tail_file(path):
    if path.endswith("b7k2x9.output"):
        lines = ["", "  \x1b[32mvitepress\x1b[0m v2.0.4", "",
                 "  \x1b[32m➜\x1b[0m  Local:   \x1b[36mhttp://localhost:5173/\x1b[0m",
                 "  \x1b[2m➜  Network: use --host to expose\x1b[0m", ""]
        pages = ["search.md", "guide/policy.md", "guide/events.md", "reference/hurad.md", "index.md"]
        for i in range(int((time.time() - STARTED) // 3) + 2):
            t = time.strftime("%H:%M:%S", time.localtime(STARTED + i * 3))
            lines.append(f"\x1b[2m{t}\x1b[0m \x1b[36m[vitepress]\x1b[0m page reload docs/{pages[i % len(pages)]}")
        text = "\n".join(lines) + "\n"
    elif path.endswith("b4m1q8.output"):
        text = ("🔍 1206 Total (in 2s) ✅ 1198 OK 🚫 0 Errors 👻 8 Excluded\n"
                "\n8 links excluded (external, --offline)\n")
    else:
        return None
    return {"path": path, "bytes": len(text.encode()), "text": text, "cut": False}

def find_files(query, limit):
    """The composer's `@`: this checkout's files by name, ranked as
    `hura_core::files::rank` ranks them (the name, then the path, then the
    letters in order; shorter first)."""
    out = subprocess.run(["git", "-C", REPO_ROOT, "-c", "core.quotePath=off", "ls-files", "--cached", "--others",
                          "--exclude-standard"], capture_output=True, text=True).stdout.split("\n")
    paths = [p for p in out if p]
    dirs = sorted({p[:i + 1] for p in paths for i in range(len(p)) if p[i] == "/"})
    q = query.strip().lower().removeprefix("./")
    def score(path):
        if not q: return 0
        low = path.lower(); name = low.rstrip("/").rsplit("/", 1)[-1]; stem = name.split(".")[0]
        if name == q or stem == q: return 0
        if name.startswith(q): return 1
        if q in name: return 2
        if q in low: return 3
        rest = iter(low)
        return 4 if all(c in rest for c in q) else None
    ranked = sorted(((score(p), len(p), p) for p in paths + dirs if score(p) is not None))
    limit = min(int(limit), 200)
    return {"paths": [p for _, _, p in ranked[:limit]], "truncated": len(ranked) > limit}

# A folder that is not in the checkout, with a file of each kind the file
# icons tell apart: documents, media, and the languages with marks of their own.
SAMPLES = ["AGENTS.md", "App.test.tsx", "CLAUDE.md", "Cargo.toml", "Main.java", "budget.xlsx", "data.csv", "demo.mp4", "fix.patch",
           "guide.epub", "handbook.pdf", "layout.xml", "notes.docx", "photo.png", "roadmap.pptx", "run.ps1",
           "server.go", "setup.py", "styles.css", "vitest.config.ts"]

def files(path):
    if path == "samples":
        return {"path": path, "entries": [{"name": n, "dir": False} for n in sorted(SAMPLES, key=str.lower)]}
    full = os.path.join(REPO_ROOT, path)
    names = sorted(n for n in os.listdir(full) if n not in SKIP) if os.path.isdir(full) else []
    entries = [{"name": n, "dir": os.path.isdir(os.path.join(full, n))} for n in names]
    if path == "": entries.append({"name": "samples", "dir": True})
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
# The first script only re-indented: a change that is only whitespace, which
# the phone's Settings can hide.
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
    if op == "reorder-projects":
        projects.sort(key=lambda p: req["names"].index(p["name"]) if p["name"] in req["names"] else len(projects))
        return {"reply": "projects", "projects": projects}
    if op == "reorder-sessions":
        for s in sessions:
            if s["name"] in req["names"]: s["rank"] = req["names"].index(s["name"])
        return answer({"op": "ls"})
    if op == "queue": return {"reply": "queue", **queue}
    if op == "queue-message":
        repeat = req.get("repeat")
        at = req.get("at") or (next_time(repeat, int(time.time())) if repeat else None)
        ids = [q["id"] for k in ("sessions", "messages") for q in queue[k]]
        queue["messages"].append(message(max(ids + [0]) + 1, req["name"], req["text"], at=at, repeat=repeat))
        return {"reply": "queue", **queue}
    if op == "set-project-defaults":
        for p in projects:
            if p["name"] == req["name"]:
                p["defaults"] = {k: v for k, v in req.get("defaults", {}).items() if v not in (None, "", [])}
        return {"reply": "projects", "projects": projects}
    # What the new-worktree draft is filled from, and what it asks for.
    if op == "new-options":
        return {"reply": "new-options",
                "policies": [{"spec": "feature-work", "summary": "The package registries and the forge"},
                             {"spec": "locked-down", "summary": "Nothing but the forge"},
                             {"spec": "research", "summary": "Anything over HTTPS, logged"}],
                "toolchains": [{"name": "node", "summary": "Node.js 24 and npm"},
                               {"name": "rust", "summary": "Rust stable and cargo"},
                               {"name": "dotnet", "summary": ".NET 10 SDK"}],
                "providers": [{"name": "claude", "kind": "anthropic"}, {"name": "github", "kind": "github"}],
                "providers_error": None, "default_policy": "feature-work", "default_providers": ["claude"],
                "default_base": "main", "skills": ["frontend-design"], "mcp": ["jira"],
                "default_interface": "chat", "worktrees": True}
    if op == "inspect":
        return {"reply": "inspect", "facts": {"uncommitted": 2, "unpushed": 1, "base_on_remote": True,
                "toolchains": ["node", "rust"]}, "branch": "main", "providers": []}
    if op == "upload-skills":
        return {"reply": "integrations", "mcp": [], "secrets": [], "skills": [], "configured_skills": []}
    if op in ("create", "create-from-pull"):
        name = req.get("name") or "-".join(req.get("task", "").lower().split()[:3]) or "new-worktree"
        # Listed from now on, so the client opening it finds it.
        project = req.get("project") or projects[0]["name"]
        if not any(x["name"] == name for x in sessions):
            sessions.append(session(name, project, "hura/" + name, "creating", 0.01, req.get("task", "")))
        print("created", name, "on", PORT, flush=True)
        return {"reply": "created", "name": name}
    if op == "enqueue":
        new, repeat = req["new"], req.get("repeat")
        new["name"] = new.get("name") or "-".join(new.get("task", "").lower().split()[:3]) or "queued"
        at = req.get("at") or (next_time(repeat, int(time.time())) if repeat else None)
        ids = [q["id"] for k in ("sessions", "messages") for q in queue[k]]
        entry = queued(max(ids + [0]) + 1, new["name"], new["project"], new.get("task", ""), at=at, repeat=repeat)
        entry["new"].update(new)
        queue["sessions"].append(entry)
        return {"reply": "queue", **queue}
    if op == "unqueue":
        for k in ("sessions", "messages"):
            queue[k] = [q for q in queue[k] if q["id"] != req.get("id")]
        return {"reply": "queue", **queue}
    if op == "poll": return {"reply": "poll", **poll(name)}
    if op == "shells": return {"reply": "shells", "shells": SHELLS.get(name, []), "agent": True}
    if op == "new-shell":
        have = SHELLS.setdefault(name, [])
        have.append(f"shell-{len(have) + 1}")
        if req.get("run"): SHELL_RUNS[(name, have[-1])] = req["run"]
        return {"reply": "shells", "shells": have, "agent": True}
    if op == "kill-shell":
        SHELLS[name] = [t for t in SHELLS.get(name, []) if t != req.get("tmux")]
        return {"reply": "shells", "shells": SHELLS[name], "agent": True}
    if op == "chats": return {"reply": "chats", "chats": CHATS.get(name, ["agent"]), "opened": None}
    if op == "new-chat":
        have = CHATS.setdefault(name, ["agent"])
        opened = f"chat-{len(have) + 1}"
        have.append(opened)
        return {"reply": "chats", "chats": have, "opened": opened}
    if op == "close-chat":
        CHATS[name] = [c for c in CHATS.get(name, ["agent"]) if c != req.get("conv")]
        return {"reply": "chats", "chats": CHATS[name], "opened": None}
    if op == "events": return {"reply": "events", "events": events}
    if op == "policy": return {"reply": "policy", **policy}
    if op == "git-status": return {"reply": "git", "said": "", "status": git_status}
    if op == "git": return {"reply": "git", "said": "Everything up-to-date", "status": git_status}
    if op == "git-diff": return {"reply": "git-diff", **git_diff(req["path"], req["against"])}
    if op == "files": return {"reply": "files", **files(req.get("path", ""))}
    if op == "tail-file":
        t = tail_file(req.get("path", ""))
        return {"reply": "tail", **(t or {"path": req.get("path", ""), "bytes": 0, "text": "", "cut": False})}
    if op == "find-files": return {"reply": "found", **find_files(req.get("query", ""), req.get("limit", 50))}
    if op == "file": return {"reply": "file", **file(req["path"])}
    if op == "comments": return {"reply": "comments", "comments": []}
    if op == "ports": return {"reply": "ports", **ports}
    # A shared preview: the stand-in below, two ports up, behind one token.
    if op == "share-preview": return {"reply": "shared", "port": PORT + 3, "token": "fake-share"}
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
        lock = threading.Lock()
        send = lambda obj: (lock.acquire(), ws_send(self.wfile, obj), lock.release())
        ptys = {}
        # Port channels: a preview's connections, each passed to this
        # machine's port FAKE_PORT_OFFSET above the sandbox's (20000 by
        # default, so 5173 is 25173), where a test page can be served.
        socks = {}
        # Open conversations, by channel, with the last seq each has said.
        chats = {}
        def pump_sock(cid, sock):
            while True:
                try: out = sock.recv(65536)
                except OSError: out = b""
                if not out:
                    socks.pop(cid, None)
                    try: send({"is": "closed", "id": cid, "reason": None})
                    except OSError: pass
                    return
                try: send({"is": "output", "id": cid, "data": base64.b64encode(out).decode()})
                except OSError: return
        def pump(cid, fd):
            while True:
                try: out = os.read(fd, 65536)
                except OSError: out = b""
                if not out:
                    try: send({"is": "closed", "id": cid, "reason": "detached"})
                    except OSError: pass
                    return
                try: send({"is": "output", "id": cid, "data": base64.b64encode(out).decode()})
                except OSError: return
        try:
            while True:
                op, data = ws_recv(self.rfile)
                if op == 8: return
                if op != 1: continue
                req = json.loads(data)
                if req.get("do") == "input" and req.get("id") in ptys:
                    os.write(ptys[req["id"]][1], base64.b64decode(req["data"]))
                    continue
                if req.get("do") == "input" and req.get("id") in socks:
                    try: socks[req["id"]].sendall(base64.b64decode(req["data"]))
                    except OSError: pass
                    continue
                if req.get("do") == "close" and req.get("id") in socks:
                    try: socks.pop(req["id"]).shutdown(socket.SHUT_RDWR)
                    except OSError: pass
                    continue
                # A message said in a conversation comes back as your prompt and
                # a one-line answer, so a side chat's first words and an edited
                # message show; nothing else a chat is told does anything.
                if req.get("do") == "chat" and req.get("id") in chats:
                    cmd = req.get("command", {})
                    if cmd.get("op") == "send":
                        said = chats[req["id"]]
                        for ev in [prompt(cmd.get("text", ""), f"u-said-{said['seq'] + 1}"),
                                   {"kind": "sdk", "msg": {"type": "assistant", "uuid": f"a-said-{said['seq'] + 2}",
                                    "parent_tool_use_id": None, "message": {"content": [
                                        {"type": "text", "text": "(The fake server heard that.)"}]}}}]:
                            said["seq"] += 1
                            send({"is": "chat", "id": req["id"], "frame": {"t": "entry", "entry": {
                                "seq": said["seq"], "at": int(time.time() * 1000), "ev": ev}}})
                    continue
                if req.get("do") == "resize" and req.get("id") in ptys:
                    tty_size(ptys[req["id"]][1], req["cols"], req["rows"])
                    continue
                if req.get("do") != "open": continue
                cid, ch = req["id"], req.get("channel", {})
                kind, name = ch.get("kind"), ch.get("session")
                if kind == "terminal" and FAKE_TMUX:
                    pid, fd = tmux_attach(name)
                    ptys[cid] = (pid, fd)
                    send({"is": "opened", "id": cid})
                    threading.Thread(target=pump, args=(cid, fd), daemon=True).start()
                    continue
                if kind == "port":
                    target = ch.get("port", 0) + int(os.environ.get("FAKE_PORT_OFFSET", "20000"))
                    try: sock = socket.create_connection(("127.0.0.1", target), timeout=2)
                    except OSError as e:
                        send({"is": "closed", "id": cid, "reason": f"nothing on {target} here: {e}"})
                        continue
                    sock.settimeout(None)
                    socks[cid] = sock
                    send({"is": "opened", "id": cid})
                    threading.Thread(target=pump_sock, args=(cid, sock), daemon=True).start()
                    continue
                send({"is": "opened", "id": cid})
                if kind == "chat":
                    frames = chat_frames(name, cid, ch.get("conv"))
                    chats[cid] = {"seq": frames[0]["frame"]["seq"]}
                    for frame in frames: send(frame)
                elif kind == "status":
                    send({"is": "status", "id": cid, "poll": poll(name)})
                elif kind == "events":
                    send({"is": "events", "id": cid, "events": events})
                elif kind == "terminal" and (name, ch.get("tmux")) in SHELL_RUNS:
                    run = SHELL_RUNS[(name, ch.get("tmux"))]
                    out = (f"\x1b[1;32m~/repo\x1b[0m $ {run}\r\n\r\n"
                           "  \x1b[1;32mVITE\x1b[0m v7.1.3  ready in \x1b[1m412\x1b[0m ms\r\n\r\n"
                           "  \x1b[32m➜\x1b[0m  \x1b[1mLocal\x1b[0m:   \x1b[36mhttp://localhost:5173/\x1b[0m\r\n")
                    send({"is": "output", "id": cid, "data": base64.b64encode(out.encode()).decode()})
                elif kind == "terminal":
                    send({"is": "output", "id": cid, "data": base64.b64encode(TERMINAL.encode()).decode()})
        except (ConnectionError, OSError):
            return
        finally:
            for pid, fd in ptys.values():
                try: os.kill(pid, 15); os.close(fd)
                except OSError: pass

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
def pull(n, title, review, hours, checks="passing", verdict=None, draft=False, conflicts=False, comments=None, branch=None):
    return {"tracker": "github", "kind": "git-hub", "id": str(n), "key": f"#{n}", "title": title,
            "url": f"https://github.com/acme/storefront/pull/{n}", "repo": "acme/storefront",
            "repo_url": "https://github.com/acme/storefront.git", "relation": "assigned" if review else "created",
            "review_requested": review, "author": "sam" if review else "tobias", "branch": branch or f"feature/{n}",
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
        # On the branches of two worktrees, which show them in the tree and over their chats.
        pull(231, "Price rounding: round once, after tax", False, 1, "pending", comments=2, branch="tobias/SHOP-412"),
        pull(229, "Cart: keep saved carts across sessions", False, 20, "failing", conflicts=True, branch="tobias/SHOP-398"),
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
# What a shared preview's link reaches (`share.rs` in hurad): its token for a
# cookie, then a page saying it got through, over the same certificate.
class Shared(BaseHTTPRequestHandler):
    def do_GET(self):
        if "hura_preview=fake-share" in self.path:
            self.send_response(303)
            self.send_header("Location", self.path.split("?")[0])
            self.send_header("Set-Cookie", "hura_preview_%d=fake-share; Path=/; Secure; HttpOnly" % (PORT + 3))
            self.end_headers()
            return
        ok = "hura_preview_%d=fake-share" % (PORT + 3) in (self.headers.get("Cookie") or "")
        body = ("<!doctype html><meta name=viewport content='width=device-width'><body style='font:17px -apple-system;margin:3em 1.5em'>"
                + ("<h1>Docs search</h1><p>A shared preview of port 5173, through the fake server.</p>" if ok
                   else "<h1>This preview opens from its link</h1>")).encode()
        self.send_response(200 if ok else 403)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a): pass
shared = ThreadingHTTPServer(("127.0.0.1", PORT + 3), Shared)
shared.socket = ctx.wrap_socket(shared.socket, server_side=True)
threading.Thread(target=shared.serve_forever, daemon=True).start()

srv.serve_forever()
