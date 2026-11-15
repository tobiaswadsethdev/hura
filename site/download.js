// Points the download button at what the visitor's device can run, and the
// links in the download grid at the newest release's files.
//
// hurad and the desktop app release separately from the one repository, as
// `hurad-v...` and `desktop-v...` (both were one `v...` release up to v0.54.0),
// so each link is matched against the newest release of its own component.
//
// Every link starts out at the releases page, so a page whose script fails, or
// whose request to GitHub is refused (it allows 60 an hour from one address
// without a token), still downloads something; this only makes the links more
// precise.

const REPO = "tobiaswadsethdev/hura";
const RELEASES = `https://github.com/${REPO}/releases`;
const INSTALL = `curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sh`;
// The App Store page, once there is one. Until then the iPhone button says so.
const APP_STORE = "";

// Which release each kind of file comes from, by its tag.
const TAGS = {
  hurad: /^(hurad-)?v\d+\.\d+\.\d+$/,
  desktop: /^(desktop-)?v\d+\.\d+\.\d+$/,
};

// Which file each data-asset link wants, and from which component's release, by
// the shape of the names .github/workflows/release-{hurad,desktop}.yml give
// them.
const ASSETS = {
  "mac-dmg": ["desktop", /^hura-desktop-.*-aarch64-apple-darwin\.dmg$/],
  "mac-hurad": ["hurad", /^hurad-.*-aarch64-apple-darwin\.tar\.gz$/],
  "win-exe": ["desktop", /^hura-desktop-.*-x86_64-pc-windows-msvc-setup\.exe$/],
  "win-msi": ["desktop", /^hura-desktop-.*-x86_64-pc-windows-msvc\.msi$/],
  "linux-x64": ["hurad", /^hurad-.*-x86_64-unknown-linux-musl\.tar\.gz$/],
  "linux-arm64": ["hurad", /^hurad-.*-aarch64-unknown-linux-musl\.tar\.gz$/],
};

function detectPlatform() {
  const ua = navigator.userAgent;
  // iPadOS asks for the desktop site and says Macintosh; a Mac has no touch.
  if (/iPhone|iPad|iPod/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)) return "ios";
  if (/Android/.test(ua)) return "android";
  if (/Windows/.test(ua)) return "windows";
  if (/Macintosh|Mac OS X/.test(ua)) return "mac";
  if (/Linux|X11|CrOS/.test(ua)) return "linux";
  return "unknown";
}

// Only Chromium says which architecture it runs on; anything else gets x86_64,
// which is what most Linux desktops are.
async function linuxArch() {
  try {
    const { architecture } = await navigator.userAgentData.getHighEntropyValues(["architecture"]);
    return architecture === "arm" ? "linux-arm64" : "linux-x64";
  } catch {
    return /aarch64|arm64/i.test(navigator.userAgent) ? "linux-arm64" : "linux-x64";
  }
}

// The newest release of each component, from one request: the list is newest
// first, so the first of each kind is the one. A `v...` release from before the
// split counts for both, since it carried both.
async function latestReleases() {
  try {
    const res = await fetch(`https://api.github.com/repos/${REPO}/releases?per_page=50`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) return {};
    const list = (await res.json()).filter((r) => !r.draft && !r.prerelease);
    return Object.fromEntries(
      Object.entries(TAGS).map(([component, tag]) => [component, list.find((r) => tag.test(r.tag_name))]),
    );
  } catch {
    return {};
  }
}

// `1.2.3` out of `hurad-v1.2.3`, `desktop-v1.2.3` or `v1.2.3`.
function versionOf(release) {
  return release?.tag_name?.replace(/^[a-z-]*v/, "");
}

function assetUrl(releases, key) {
  const [component, pattern] = ASSETS[key];
  const asset = releases[component]?.assets?.find((a) => pattern.test(a.name));
  return asset ? asset.browser_download_url : RELEASES;
}

function setPrimary({ label, sub, href, command, note, disabled }) {
  const button = document.getElementById("cta-primary");
  button.querySelector(".button-label").textContent = label;
  document.getElementById("cta-sub").textContent = sub;
  if (href) button.href = href;
  else button.removeAttribute("href");
  button.classList.toggle("disabled", !!disabled);
  if (disabled) button.setAttribute("aria-disabled", "true");
  document.getElementById("cta-note").textContent = note || "";
  const box = document.getElementById("cta-command");
  box.hidden = !command;
  if (command) document.getElementById("cta-command-text").textContent = command;
}

async function main() {
  const platform = detectPlatform();
  document.querySelector(`.platform[data-platform="${platform}"]`)?.classList.add("current");

  if (APP_STORE) {
    const ios = document.querySelector('[data-asset="ios"]');
    ios.href = APP_STORE;
    ios.textContent = "Download on the App Store";
    ios.classList.remove("disabled");
    ios.removeAttribute("aria-disabled");
  }

  const releases = await latestReleases();
  const desktop = versionOf(releases.desktop);
  const hurad = versionOf(releases.hurad);
  const v = desktop ? ` ${desktop}` : "";
  const h = hurad ? `hurad ${hurad}` : "";
  document.querySelectorAll("[data-version]").forEach((el) => {
    const version = versionOf(releases[el.dataset.version]);
    if (version) el.textContent = version;
  });
  document.querySelectorAll("[data-asset]").forEach((a) => {
    const key = a.dataset.asset;
    if (ASSETS[key]) a.href = assetUrl(releases, key);
  });

  switch (platform) {
    case "mac":
      setPrimary({
        label: "Download for macOS",
        sub: `Desktop app${v} · Apple silicon`,
        href: assetUrl(releases, "mac-dmg"),
        note: "Running the sessions on this Mac too? hurad is in Other platforms.",
      });
      break;
    case "windows":
      setPrimary({
        label: "Download for Windows",
        sub: `Desktop app${v} · x64 installer`,
        href: assetUrl(releases, "win-exe"),
        note: "The window connects to hurad, in WSL2 on this machine or on another one.",
      });
      break;
    case "linux": {
      const key = await linuxArch();
      setPrimary({
        label: "Download hurad for Linux",
        sub: `${h || "Latest"} · ${key === "linux-arm64" ? "arm64" : "x86_64"}`,
        href: assetUrl(releases, key),
        command: INSTALL,
        note: "Or install it with one line, which checks the download and runs hurad doctor.",
      });
      break;
    }
    case "ios":
      setPrimary(
        APP_STORE
          ? { label: "Download on the App Store", sub: "hura for iPhone", href: APP_STORE }
          : { label: "Coming to the App Store", sub: "hura for iPhone is in testing", disabled: true },
      );
      break;
    default:
      setPrimary({ label: "Download hura", sub: "For macOS, Windows and Linux", href: "#download" });
  }
}

document.addEventListener("click", async (e) => {
  const button = e.target.closest("[data-copy-from]");
  if (!button) return;
  const text = document.getElementById(button.dataset.copyFrom).textContent;
  try {
    await navigator.clipboard.writeText(text);
    button.textContent = "Copied";
  } catch {
    button.textContent = "Select it";
  }
  setTimeout(() => (button.textContent = "Copy"), 1600);
});

main();
