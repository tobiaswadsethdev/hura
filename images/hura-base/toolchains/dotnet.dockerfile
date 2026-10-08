# dotnet -- the .NET SDK, from the release manifest rather than a package feed.
#
# The tarball and not packages.microsoft.com: an apt feed pins nothing, needs a
# key and a source list added to the image, and answers `apt-get install
# dotnet-sdk-9.0` with whatever it happens to serve that day. The manifest route
# is the one the Claude Code step already uses -- resolve a version, take the
# checksum the publisher published for it, verify what arrived -- and it is the
# only one that makes a rebuild honest about what it installed.
#
# `--build-arg DOTNET_CHANNEL=8.0` builds against another channel. A concrete
# version is deliberately not a build arg: the manifest is indexed by channel,
# and resolving is how the newest patch of that channel is found.
#
# The Ubuntu in the base image already carries every native dependency the SDK
# needs (libicu, libssl3t64, libstdc++6, zlib1g), so there is no apt-get in this
# layer at all. libicu is checked rather than assumed, by any version, since
# each Ubuntu release moves it (74 on 24.04, 78 on 26.04): without it the SDK
# starts and then fails on the first culture-aware call, which is a bad way to
# find out.
#
# The symlink at the end puts `dotnet` on the PATH the base image already has,
# rather than adding a directory to PATH. `ENV PATH` did not reach the agent
# under OpenShell, whose exec passed none of the image's environment through,
# and a symlink works however the environment arrives, with nothing in
# tmux.conf to keep in step. The policy does not mind either way: its rules are
# for the whole sandbox, not for one program in it.
ARG DOTNET_CHANNEL=9.0
RUN set -eu; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
        amd64) rid=linux-x64 ;; \
        arm64) rid=linux-arm64 ;; \
        *) echo "no dotnet sdk for architecture $arch" >&2; exit 1 ;; \
    esac; \
    dpkg -l 'libicu[0-9]*' 2>/dev/null | grep -q '^ii' \
        || { echo "libicu is gone from the base image; the sdk needs it" >&2; exit 1; }; \
    meta="https://builds.dotnet.microsoft.com/dotnet/release-metadata/$DOTNET_CHANNEL/releases.json"; \
    curl -fsSL --retry 3 -o /tmp/releases.json "$meta"; \
    sdk="$(jq -re --arg rid "$rid" ' \
        ."latest-sdk" as $v \
        | [.releases[] | .sdk, (.sdks[]?)] \
        | map(select(.version == $v)) | .[0] \
        | .files[] | select(.rid == $rid and (.name | endswith(".tar.gz"))) \
        | "\($v) \(.url) \(.hash)"' /tmp/releases.json)"; \
    version="${sdk%% *}"; rest="${sdk#* }"; url="${rest%% *}"; hash="${rest#* }"; \
    echo "dotnet sdk $version ($rid)"; \
    curl -fsSL --retry 3 -o /tmp/dotnet.tar.gz "$url"; \
    echo "$hash  /tmp/dotnet.tar.gz" | sha512sum -c -; \
    mkdir -p /usr/local/dotnet; \
    tar -xzf /tmp/dotnet.tar.gz -C /usr/local/dotnet; \
    rm -f /tmp/dotnet.tar.gz /tmp/releases.json; \
    ln -sf /usr/local/dotnet/dotnet /usr/local/bin/dotnet; \
    installed="$(dotnet --version)"; \
    test "$installed" = "$version" \
        || { echo "wanted dotnet $version, got $installed" >&2; exit 1; }; \
    printf 'dotnet %s\n' "$version" >> /usr/local/share/hura/toolchains

# Everything the SDK writes, pointed somewhere writable, and everything it would
# send home, turned off.
#
# /usr/local is read-only to the sandbox user by design, and the SDK wants to
# write on first run: a CLI home for its own state, a package cache for restore.
# Both default to somewhere under $HOME, which here is /home/agent and writable, so
# this is less about making it work than about saying it out loud -- a restore
# writes gigabytes, and which directory that is belongs beside the toolchain.
#
# The three opt-outs are there because the sandbox *denies* the traffic behind
# them. Telemetry on every command, and the first-run banner's workload
# advertising, would each produce a denied egress event with nothing worth
# investigating behind it -- the same noise the base image already suppresses for
# Claude Code's auto-updater.
#
# `NUGET_CERT_REVOCATION_MODE=offline` is the third, and it was measured rather
# than predicted: a `dotnet add package Newtonsoft.Json` succeeded and left six
# denials in the events feed, all of them NuGet checking the *signing*
# certificates against `www.microsoft.com/pkiops/crl/...`, `crl3.digicert.com`
# and `ocsp.digicert.com`. The restore does not need them -- the online
# revocation check is soft-fail, which is why it worked -- so this is a choice
# between allowing three more hosts and not making the request. Not making it is
# the same answer the auto-updater got, and it is the one that leaves a feed
# worth reading. Signature verification itself is untouched; only the online
# revocation lookup is.
#
# Set twice, for the reason the base image sets its locale twice: `ENV` covers a
# shell someone opens by hand, and `set-environment` in tmux.conf is what reaches
# the agent, whose environment comes from the tmux server the seeder starts.
ENV DOTNET_ROOT=/usr/local/dotnet \
    DOTNET_CLI_HOME=/home/agent/.dotnet \
    NUGET_PACKAGES=/home/agent/.nuget/packages \
    DOTNET_CLI_TELEMETRY_OPTOUT=1 \
    DOTNET_NOLOGO=1 \
    NUGET_CERT_REVOCATION_MODE=offline

RUN printf '%s\n' \
        'set-environment -g DOTNET_ROOT /usr/local/dotnet' \
        'set-environment -g DOTNET_CLI_HOME /home/agent/.dotnet' \
        'set-environment -g NUGET_PACKAGES /home/agent/.nuget/packages' \
        'set-environment -g DOTNET_CLI_TELEMETRY_OPTOUT 1' \
        'set-environment -g DOTNET_NOLOGO 1' \
        'set-environment -g NUGET_CERT_REVOCATION_MODE offline' \
    >> /etc/tmux.conf
