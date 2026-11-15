# node: the newest LTS release, from nodejs.org, over the one the base carries.
#
# The base image has Ubuntu's nodejs at /usr/bin, which is whatever the Ubuntu
# release froze (22 on 26.04) and moves only when Ubuntu does. That is fine for
# the image's own use of it and too old for a project that asks for a current
# engine, so a session that asked for node gets the release node itself calls
# current for production use.
#
# `--build-arg NODE_CHANNEL=24` builds another major line, and `current` the
# newest release of any kind. The version is resolved from the release index
# and the tarball checked against the SHASUMS256.txt published beside it, for
# the reason every other install in this image does the same: a rebuild has to
# be honest about what it installed.
#
# Into /usr/local/node, with node, npm and npx linked into /usr/local/bin. That
# is ahead of /usr/bin on the base image's PATH, so this node wins everywhere,
# the chat host (`hura-agent`) included; the import at the end proves the host's
# SDK still loads under it. npm's global prefix is left where the base image puts
# it (NPM_CONFIG_PREFIX, under /usr/local/share, writable by the agent).
#
# corepack is linked only when the release ships it. Node stopped bundling it in
# 25, and the base image's own copy at /usr/bin/corepack runs on this node too.
ARG NODE_CHANNEL=lts
RUN set -eu; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
        amd64) platform=linux-x64 ;; \
        arm64) platform=linux-arm64 ;; \
        *) echo "no node release for architecture $arch" >&2; exit 1 ;; \
    esac; \
    case "$NODE_CHANNEL" in \
        lts|current) ;; \
        ''|*[!0-9]*) echo "not a node channel: $NODE_CHANNEL" >&2; exit 1 ;; \
    esac; \
    dist=https://nodejs.org/dist; \
    curl -fsSL --retry 3 -o /tmp/node-index.json "$dist/index.json"; \
    version="$(jq -re --arg ch "$NODE_CHANNEL" '[.[] \
        | if $ch == "lts" then select(.lts != false) \
          elif $ch == "current" then . \
          else select(.version | startswith("v" + $ch + ".")) end][0].version' \
        /tmp/node-index.json)"; \
    echo "node $version ($platform)"; \
    tarball="node-$version-$platform.tar.gz"; \
    curl -fsSL --retry 3 -o "/tmp/$tarball" "$dist/$version/$tarball"; \
    curl -fsSL --retry 3 -o /tmp/node.sha256 "$dist/$version/SHASUMS256.txt"; \
    grep " $tarball\$" /tmp/node.sha256 | (cd /tmp && sha256sum -c -); \
    mkdir -p /usr/local/node; \
    tar -xzf "/tmp/$tarball" -C /usr/local/node --strip-components=1 --no-same-owner; \
    rm -f "/tmp/$tarball" /tmp/node.sha256 /tmp/node-index.json; \
    for bin in node npm npx corepack; do \
        if [ -e "/usr/local/node/bin/$bin" ]; then ln -sf "/usr/local/node/bin/$bin" "/usr/local/bin/$bin"; fi; \
    done; \
    installed="$(node --version)"; \
    test "$installed" = "$version" \
        || { echo "wanted node $version, got $installed on PATH" >&2; exit 1; }; \
    npm --version >/dev/null; \
    cd /usr/local/lib/hura-agent && node --input-type=module -e "await import('@anthropic-ai/claude-agent-sdk')"; \
    printf 'node %s\n' "${version#v}" >> /usr/local/share/hura/toolchains
