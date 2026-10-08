# syntax=docker/dockerfile:1
#
# hura sandbox image.
#
# Docker's `shell-docker` sandbox template has git, node, python, uv, gh, socat
# and its own Docker Engine, which is the reason to be on it: an agent can build
# and run containers inside its sandbox. It has no agent and no tmux. hura runs
# each agent inside an in-sandbox tmux session so the agent survives losing its
# connection, and so its output can be scraped with capture-pane without
# depending on anything on the host.
#
# The template's labels carry over to this image, and one of them
# (`com.docker.sandboxes.start-docker`) is what makes the runtime start that
# engine, so the base has to stay a `FROM` rather than a copy of its contents.
#
# The workload runs as the template's `agent` user, with HOME at /home/agent.
# Everything hura itself puts in a session stays under /sandbox, which is made
# here and given to `agent`.
#
# Build:  hurad image build   (or: docker build -t hura-base:latest images/hura-base,
#         then load it into the sandbox runtime with `sbx template load`)
FROM docker/sandbox-templates:shell-docker

USER root

# tmux    - keeps the agent alive independently of any connection
# less    - git's pager; without it git paging misbehaves under a pty
# jq      - reading agent JSON output
# ripgrep - agents reach for it constantly
RUN apt-get update && apt-get install -y --no-install-recommends \
        tmux \
        less \
        jq \
        ripgrep \
    && rm -rf /var/lib/apt/lists/*

# Claude Code, at the newest release rather than whatever the base image froze.
#
# The template has no Claude Code, and the agent should not be the one picking
# its version: a session under `feature-work` cannot reach the download host,
# and a self-update would leave each session on whatever it found that day. So
# the version is the image's business.
#
# `latest` is resolved to a concrete version, either here or by the caller, and
# `--build-arg CLAUDE_VERSION=2.1.246` pins it when a specific one is wanted.
# `hura image build` always resolves it on the host and passes it in, because
# docker would otherwise answer a rebuild from the cached layer and `latest`
# would quietly mean "whatever was newest the first time" -- which is the exact
# staleness this step exists to fix. `hura doctor` compares the built image
# against the newest release and says when it has fallen behind.
#
# The release binary, not install.sh: the installer lands a launcher under
# `$HOME`, where the agent's own tools would shadow it. /usr/local/bin keeps it
# on PATH for every user, at the path the rest of hura names.
#
# The checksum comes from the release manifest, so a truncated or substituted
# download fails the build instead of shipping. The `--version` check at the end
# catches the subtler failure: a manifest and a binary that disagree, leaving the
# old binary in place and the build green.
ARG CLAUDE_VERSION=latest
RUN set -eu; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
        amd64) platform=linux-x64 ;; \
        arm64) platform=linux-arm64 ;; \
        *) echo "no claude release for architecture $arch" >&2; exit 1 ;; \
    esac; \
    base=https://downloads.claude.ai/claude-code-releases; \
    version="$CLAUDE_VERSION"; \
    if [ "$version" = latest ]; then version="$(curl -fsSL "$base/latest")"; fi; \
    case "$version" in \
        [0-9]*.[0-9]*.[0-9]*) ;; \
        *) echo "not a claude version: $version" >&2; exit 1 ;; \
    esac; \
    checksum="$(curl -fsSL "$base/$version/manifest.json" \
        | jq -re --arg p "$platform" '.platforms[$p].checksum')"; \
    curl -fsSL -o /tmp/claude "$base/$version/$platform/claude"; \
    echo "$checksum  /tmp/claude" | sha256sum -c -; \
    install -m 0755 -o root -g root /tmp/claude /usr/local/bin/claude; \
    rm -f /tmp/claude; \
    installed="$(claude --version | awk '{print $1}')"; \
    test "$installed" = "$version" \
        || { echo "wanted claude $version, got $installed" >&2; exit 1; }

# The sandbox runtime sets ANTHROPIC_API_KEY to a sentinel in every exec, for an
# Anthropic integration of its own that hura does not use. A tmux server keeps
# the environment it started with, so whichever exec starts it, the agent in it
# must not see that variable: Claude Code stops to ask about the key, and would
# prefer it to the session's own credential.
#
# A long scrollback is what makes capture-pane useful for previews and, later,
# for detecting what the agent is waiting on.
#
# `default-size` is the width an unattached session gets, and unattached is how
# the agent spends most of its life -- so it is the width every status scrape
# reads. Claude Code's footer is a list of hints truncated to the pane, and at
# tmux's default 80x24 it already ends in an ellipsis: `esc to interrupt`, which
# is how a working agent is recognised, sits in the middle of that list and would
# be the next thing to fall off it. 200 columns is not a display choice; it keeps
# the markers on screen. Attaching resizes the window to the terminal as usual.
RUN printf '%s\n' \
        'set -g history-limit 50000' \
        'set -g status off' \
        'set -g mouse on' \
        'set -g escape-time 10' \
        'set -g default-size 200x50' \
        'set -g default-terminal tmux-256color' \
        'set -as terminal-features ",*:RGB"' \
        'set-environment -g LANG C.UTF-8' \
        'set-environment -g LC_ALL C.UTF-8' \
        'set-environment -g COLORTERM truecolor' \
        'set-environment -gu ANTHROPIC_API_KEY' \
    > /etc/tmux.conf \
    && chmod 0644 /etc/tmux.conf

# UTF-8, and colour, and the reason both have to be said twice.
#
# The template sets no locale at all. Under OpenShell the gateway also stripped
# the image's environment from every exec, so a tmux client it started had
# neither; the sandbox runtime passes `ENV` through, but the tmux conf and
# `ops::attach_script` still say it so nothing depends on which one ran. A tmux
# client that does not believe it is on a UTF-8 terminal draws with the DEC
# line-drawing set and replaces every character it cannot map with `_`, which is
# what Claude Code's banner, its box rules and its `⏸` and `❯` glyphs turned
# into. `ENV` here fixes anything started from the image; `set-environment` in
# the tmux conf above fixes the agent's own process, whose environment comes
# from the tmux server; and `ops::attach_script` exports it a third time for the
# client, which is the one OpenShell stripped.
#
# `default-terminal` and `terminal-features RGB` are the colour half. Inside tmux
# a pane's TERM decides how many colours the agent thinks it has, and RGB has to
# be declared before tmux will pass a 24-bit colour through to the terminal
# outside rather than flattening it.
ENV LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    COLORTERM=truecolor

# The agent does not manage its own version; this image does.
#
# Claude Code otherwise tries to update itself on startup and fails twice over:
# /usr/local/bin is root-owned, and no policy template lets a sandbox reach the
# download service. The visible cost was `✘ Auto-update failed · Run claude
# doctor` sitting on the agent's screen for the rest of the session, and a denied
# egress event in the policy feed with nothing behind it worth investigating.
#
# Set twice on purpose. `ENV` covers anything started in the sandbox, including a
# shell someone opens by hand; the same variable in `settings.json` is what
# Claude Code reads whatever its environment, which mattered under OpenShell,
# whose exec passed none of the image's through.
ENV DISABLE_AUTOUPDATER=1

# Skip Claude Code's first-run onboarding, and turn copy-on-select off.
#
# A fresh sandbox has a fresh HOME, so without this every session opens on the
# theme picker and then the "do you trust this folder?" prompt, and the agent
# never reaches its task. lastOnboardingVersion is pinned to the version baked
# into this image so a future upgrade can legitimately re-run onboarding.
#
# copyOnSelect defaults to true, and this is the file it is read from -- it is
# the `/config` toggle labelled "Copy on select", not a setting.json key. In a
# sandbox it is worse than on a host: selecting text to read it takes the
# clipboard, and the terminal it takes it from is one you are borrowing to watch
# an agent work.
RUN mkdir -p /sandbox \
    && chown agent:agent /sandbox \
    && CLAUDE_VERSION="$(claude --version 2>/dev/null | awk '{print $1}')" \
    && jq -n --arg v "$CLAUDE_VERSION" '{ \
         hasCompletedOnboarding: true, \
         lastOnboardingVersion: $v, \
         copyOnSelect: false, \
         projects: { \
           "/sandbox": {hasTrustDialogAccepted: true, projectOnboardingSeenCount: 1}, \
           "/sandbox/repo": {hasTrustDialogAccepted: true, projectOnboardingSeenCount: 1} \
         } \
       }' > /home/agent/.claude.json \
    && chown agent:agent /home/agent/.claude.json

# Status reporting, so the list view can say "this agent needs you" rather than
# only "this sandbox is up".
#
# The reporter is a script on PATH rather than seven copies of a shell one-liner
# inlined into settings.json: the payload handling lives in one place, and both
# files can be linted and tested outside the image.
#
# `Notification` is the signal the whole tool exists to deliver -- Claude Code
# fires it when it needs permission for a tool, or when it has been waiting on
# input. `Stop` ends a turn, so it means idle rather than finished for good.
# PreToolUse and PostToolUse mostly keep the timestamp fresh, which is what
# distinguishes an agent still working from a stale file left by a crashed one.
#
# `hura-usage` is the status line command, and the status line is the only place
# Claude Code hands out what it knows about cost and rate limits -- there is no
# file it writes them to and no API to ask. It keeps the payload where hura can
# read it and prints the line the agent shows.
COPY hura-status /usr/local/bin/hura-status
COPY hura-usage /usr/local/bin/hura-usage
COPY claude-settings.json /home/agent/.claude/settings.json

RUN chmod 0755 /usr/local/bin/hura-status /usr/local/bin/hura-usage \
    && chmod 0644 /home/agent/.claude/settings.json \
    && chown -R agent:agent /home/agent/.claude

# The chat agent: Claude Code driven through the Agent SDK, by a host process
# that lives in here rather than on the machine outside.
#
# The SDK release is the one paired with the Claude Code installed above. Each
# release names its CLI in `claudeCodeVersion`, and the two speak a control
# protocol to each other, so the pair is chosen together rather than each being
# whatever was newest. Optional dependencies are left out: they are the SDK's
# own copy of Claude Code for every platform, and the host points it at
# /usr/local/bin/claude instead, which is the one installed and pinned above.
# Peer dependencies are left out too; the SDK bundles what it imports.
#
# Root-owned, like the claude binary. The agent has sudo in its VM, so this
# keeps an accident from rewriting the process that answers its permission
# prompts rather than a determined agent; the VM is the boundary.
COPY hura-agent.mjs /usr/local/lib/hura-agent/hura-agent.mjs
RUN set -eu; \
    claude_version="$(claude --version | awk '{print $1}')"; \
    sdk="$(npm view '@anthropic-ai/claude-agent-sdk@>=0.0.0' claudeCodeVersion 2>/dev/null \
        | awk -v want="'$claude_version'" '$2 == want { sub(/.*@/, "", $1); print $1 }' | tail -n 1)"; \
    [ -n "$sdk" ] || { echo "no Agent SDK release is paired with claude $claude_version" >&2; exit 1; }; \
    cd /usr/local/lib/hura-agent; \
    npm install --no-save --no-audit --no-fund --omit=optional --legacy-peer-deps \
        "@anthropic-ai/claude-agent-sdk@$sdk"; \
    chmod 0755 hura-agent.mjs; \
    ln -s /usr/local/lib/hura-agent/hura-agent.mjs /usr/local/bin/hura-agent; \
    printf '%s\n' "$sdk" > /usr/local/lib/hura-agent/sdk-version; \
    node --input-type=module -e "await import('@anthropic-ai/claude-agent-sdk')"

# What `image::supports_chat` asks the image, without starting a container. A
# variant inherits it, so a variant built from an older base says so too.
LABEL hura.chat="1"

USER agent
