// Links leave the window through the system's browser.
//
// A Tauri window has no browser behind it, so an anchor with
// `target="_blank"` -- every link to a ticket, a release, a remote -- was a
// click that did nothing. One listener for the whole document hands such a
// click to the opener plugin instead, so every link in the window works the
// same way without each of them knowing, including the ones in a ticket's
// description. The capability lets it open `http`, `https` and `mailto` and
// nothing else, and this checks the same three before asking.

import { openUrl } from "@tauri-apps/plugin-opener";

const EXTERNAL = /^(https?:|mailto:)/i;

/// Open an address in the system's browser or mail client. Anything that is
/// not a web or mail address is refused here rather than handed on.
export function openExternal(url: string) {
  if (!EXTERNAL.test(url)) return;
  void openUrl(url).catch((e) => console.error("could not open", url, e));
}

/// Route clicks on external links to `openExternal`, for the life of the
/// window. Middle-click as well, which a browser would open in a new tab.
export function routeLinks() {
  const onClick = (e: MouseEvent) => {
    const primary = e.type === "click" && e.button === 0;
    const middle = e.type === "auxclick" && e.button === 1;
    if (e.defaultPrevented || !(primary || middle)) return;
    const a = (e.target as Element | null)?.closest?.("a[href]");
    const href = a?.getAttribute("href");
    if (!href || !EXTERNAL.test(href)) return;
    e.preventDefault();
    openExternal(href);
  };
  document.addEventListener("click", onClick);
  document.addEventListener("auxclick", onClick);
}
