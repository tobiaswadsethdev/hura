// Points the download button at what the visitor's device can run, and the
// links in the download grid at the newest release's files.
//
// Every link starts out at the releases page, so a page whose script fails, or
// whose request to GitHub is refused (it allows 60 an hour from one address
// without a token), still downloads something; this only makes the links more
// precise.

const REPO = "tobiaswadsethdev/hura";
const RELEASES = `https://github.com/${REPO}/releases/latest`;
const INSTALL = `curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sh`;
// The App Store page, once there is one. Until then the iPhone button says so.
const APP_STORE = "";

// Which file each data-asset link wants, by the shape of the names
// .github/workflows/release.yml gives them.
const ASSETS = {
  "mac-dmg": /^hura-desktop-.*-aarch64-apple-darwin\.dmg$/,
  "mac-hurad": /^hurad-.*-aarch64-apple-darwin\.tar\.gz$/,
  "win-exe": /^hura-desktop-.*-x86_64-pc-windows-msvc-setup\.exe$/,
  "win-msi": /^hura-desktop-.*-x86_64-pc-windows-msvc\.msi$/,
  "linux-x64": /^hurad-.*-x86_64-unknown-linux-musl\.tar\.gz$/,
  "linux-arm64": /^hurad-.*-aarch64-unknown-linux-musl\.tar\.gz$/,
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

async function latestRelease() {
  try {
    const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) return null;
    return await res.json();
  } catch {
    return null;
  }
}

function assetUrl(release, key) {
  const asset = release?.assets?.find((a) => ASSETS[key].test(a.name));
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

  const release = await latestRelease();
  const version = release?.tag_name;
  const v = version ? ` ${version}` : "";
  if (version) document.querySelectorAll("[data-version]").forEach((el) => (el.textContent = version));
  document.querySelectorAll("[data-asset]").forEach((a) => {
    const key = a.dataset.asset;
    if (ASSETS[key]) a.href = assetUrl(release, key);
  });

  switch (platform) {
    case "mac":
      setPrimary({
        label: "Download for macOS",
        sub: `Desktop app${v} · Apple silicon`,
        href: assetUrl(release, "mac-dmg"),
        note: "Running the sessions on this Mac too? hurad is in Other platforms.",
      });
      break;
    case "windows":
      setPrimary({
        label: "Download for Windows",
        sub: `Desktop app${v} · x64 installer`,
        href: assetUrl(release, "win-exe"),
        note: "The window connects to hurad, in WSL2 on this machine or on another one.",
      });
      break;
    case "linux": {
      const key = await linuxArch();
      setPrimary({
        label: "Download hurad for Linux",
        sub: `${v.trim() || "Latest"} · ${key === "linux-arm64" ? "arm64" : "x86_64"}`,
        href: assetUrl(release, key),
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
      setPrimary({ label: "Download hura", sub: `${v.trim() || "Latest"} for macOS, Windows and Linux`, href: "#download" });
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
