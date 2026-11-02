# The .NET SDK: the newest release, plus whatever a checkout pins.
#
# The install is `dotnet-sdk`, beside this file, which resolves a spec against
# the publisher's release manifest, checks the sha512 published for it, and
# installs into /usr/local/dotnet. It is copied into the image rather than
# inlined here because it runs twice: once in this layer, and again in the layer
# hura adds on top when a checkout's global.json pins an SDK this one does not
# satisfy (see crates/hura-core/src/dotnet.rs). One script, so both installs go
# the same way and land in the same place.
#
# `latest` is the newest SDK of the newest channel in active support, so the
# variant follows .NET's own releases instead of a channel named here going out
# of date. `--build-arg DOTNET_SDKS="9.0 10.0.401"` builds anything else: any
# number of channels or exact versions.
#
# The Ubuntu in the base image already carries every native dependency the SDK
# needs (libicu, libssl3t64, libstdc++6, zlib1g), so there is no apt-get in this
# layer at all; the script checks libicu is there rather than assuming it.
#
# `dotnet` reaches the PATH as a symlink in /usr/local/bin, rather than by
# adding a directory to PATH. `ENV PATH` did not reach the agent under
# OpenShell, whose exec passed none of the image's environment through, and a
# symlink works however the environment arrives, with nothing in tmux.conf to
# keep in step.
COPY dotnet-sdk /usr/local/lib/hura/dotnet-sdk
ARG DOTNET_SDKS=latest
RUN sh /usr/local/lib/hura/dotnet-sdk $DOTNET_SDKS

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
# `SSL_CERT_DIR` is for the HTTPS development certificate, also measured: an
# ASP.NET Core project's frontend commonly runs `dotnet dev-certs https --trust`
# before its dev server starts, and on Linux that trusts the certificate for
# OpenSSL by writing it to ~/.aspnet/dev-certs/trust -- then exits 4, failing
# the dev server's start, unless that directory is in SSL_CERT_DIR. Listed in
# front of the system's own directory, which SSL_CERT_DIR replaces rather than
# adds to; the sandbox runtime's proxy CA is in the system bundle that
# SSL_CERT_FILE names, so outbound HTTPS is unaffected (measured too).
#
# Set twice, for the reason the base image sets its locale twice: `ENV` covers a
# shell someone opens by hand, and `set-environment` in tmux.conf is what reaches
# the agent, whose environment comes from the tmux server the seeder starts.
ENV DOTNET_ROOT=/usr/local/dotnet \
    DOTNET_CLI_HOME=/home/agent/.dotnet \
    NUGET_PACKAGES=/home/agent/.nuget/packages \
    DOTNET_CLI_TELEMETRY_OPTOUT=1 \
    DOTNET_NOLOGO=1 \
    NUGET_CERT_REVOCATION_MODE=offline \
    SSL_CERT_DIR=/home/agent/.aspnet/dev-certs/trust:/usr/lib/ssl/certs

RUN printf '%s\n' \
        'set-environment -g DOTNET_ROOT /usr/local/dotnet' \
        'set-environment -g DOTNET_CLI_HOME /home/agent/.dotnet' \
        'set-environment -g NUGET_PACKAGES /home/agent/.nuget/packages' \
        'set-environment -g DOTNET_CLI_TELEMETRY_OPTOUT 1' \
        'set-environment -g DOTNET_NOLOGO 1' \
        'set-environment -g NUGET_CERT_REVOCATION_MODE offline' \
        'set-environment -g SSL_CERT_DIR /home/agent/.aspnet/dev-certs/trust:/usr/lib/ssl/certs' \
    >> /etc/tmux.conf
