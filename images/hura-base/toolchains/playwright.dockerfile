# playwright: what Playwright's own Chromium needs to start, and no browser.
#
# `npx playwright install chromium` downloads a browser into the agent's home,
# and the build of it Playwright wants is pinned by the version of Playwright a
# project depends on, so a browser here would be the wrong one for most
# checkouts. What a session cannot fetch for itself is the system libraries
# that browser links: the base image has fonts but not these, and measured, the
# Chromium and headless shell of Playwright 1.64 each named seventeen to twenty
# libraries as missing (glib, the X11 client libraries, gbm, cups, alsa, dbus,
# atk and the rest) and would not start. Installing them is apt, as root, from
# archive.ubuntu.com, which no session may reach, so `npx playwright
# install-deps` inside a sandbox fails.
#
# The list is Playwright's own, its `chromium` entry for ubuntu26.04 (in
# playwright-core's nativeDeps), the release the base image is on. With it
# installed every library the browser links resolves, and a page renders with
# the base's fonts. Firefox and WebKit need lists of their own and are not
# covered. libgbm1 brings Mesa, and with it LLVM, which is most of the layer's
# 200 MB, and is why this is a toolchain a session asks for rather than part of
# the base: a session that never starts a browser should not carry it.
#
# The check at the end looks each library up by the name the browser asks for,
# so a base on another release, where a package is named differently or does
# not exist, fails the build here rather than in a session.
RUN set -eu; \
    apt-get update; \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        libasound2t64 \
        libatk-bridge2.0-0t64 \
        libatk1.0-0t64 \
        libatspi2.0-0t64 \
        libcairo2 \
        libcups2t64 \
        libdbus-1-3 \
        libdrm2 \
        libgbm1 \
        libglib2.0-0t64 \
        libnspr4 \
        libnss3 \
        libpango-1.0-0 \
        libx11-6 \
        libxcb1 \
        libxcomposite1 \
        libxdamage1 \
        libxext6 \
        libxfixes3 \
        libxkbcommon0 \
        libxrandr2; \
    rm -rf /var/lib/apt/lists/*; \
    for lib in libglib-2.0.so.0 libgobject-2.0.so.0 libgio-2.0.so.0 libatk-1.0.so.0 \
        libatk-bridge-2.0.so.0 libatspi.so.0 libdbus-1.so.3 libcups.so.2 libxcb.so.1 \
        libxkbcommon.so.0 libasound.so.2 libgbm.so.1 libdrm.so.2 libX11.so.6 libXext.so.6 \
        libXcomposite.so.1 libXdamage.so.1 libXfixes.so.3 libXrandr.so.2 libcairo.so.2 \
        libpango-1.0.so.0 libnss3.so libnspr4.so; do \
        ldconfig -p | grep -q "^[[:space:]]*$lib " \
            || { echo "playwright: $lib is not installed" >&2; exit 1; }; \
    done; \
    . /etc/os-release; \
    printf 'playwright %s\n' "chromium libraries for ubuntu$VERSION_ID" >> /usr/local/share/hura/toolchains
