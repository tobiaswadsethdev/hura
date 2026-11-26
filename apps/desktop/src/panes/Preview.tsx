// A port in the sandbox, drawn in the window: the page the agent is working on,
// in a tab beside the conversation about it, without a trip to the browser.
//
// The address is this machine's end of the port's preview (see
// `previews.rs`), so what loads here is what a browser on this machine would
// get, and "Open in Browser" is the same page in one.
//
// The page is another origin than the window, so its history and its address
// are its own: back and forward are the page's to offer, and the path here is
// where it was sent, not where it has since gone. It is a sandboxed frame
// without `allow-top-navigation`, so nothing on it can take the window
// somewhere else.

import { useState } from "react";

import { Desktop, Elsewhere, Phone, Restart } from "../icons";
import { cn } from "../lib/cn";
import { openExternal } from "../open";

/// The width a phone is drawn at: an iPhone's, in CSS pixels.
const PHONE_WIDTH = 390;

export function PreviewPane({ url, port }: { url: string; port: number }) {
  const [path, setPath] = useState("/");
  const [typed, setTyped] = useState("/");
  const [loads, setLoads] = useState(0);
  const [phone, setPhone] = useState(false);
  const at = new URL(path, url).href;

  const go = () => {
    const next = typed.trim() ? (typed.trim().startsWith("/") ? typed.trim() : `/${typed.trim()}`) : "/";
    setTyped(next);
    setPath(next);
    setLoads((n) => n + 1);
  };
  const tool =
    "inline-flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground [&_svg]:size-3.5";

  return (
    <div data-tw className="flex h-full min-h-0 flex-col bg-background">
      <div className="flex h-10 shrink-0 items-center gap-1 border-b border-border px-2">
        <button className={tool} title="Load It Again" aria-label="Reload" onClick={() => setLoads((n) => n + 1)}>
          <Restart />
        </button>
        <form
          className="mx-1 flex h-7 min-w-0 flex-1 items-center rounded-md bg-foreground/5 px-2.5 text-xs ring-1 ring-border focus-within:ring-ring"
          onSubmit={(e) => {
            e.preventDefault();
            go();
          }}
        >
          <span className="shrink-0 text-muted-foreground">localhost:{port}</span>
          <input
            className="h-full min-w-0 flex-1 bg-transparent text-foreground outline-none"
            aria-label="Path"
            spellCheck={false}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
          />
        </form>
        <button
          className={cn(tool, !phone && "bg-accent text-foreground")}
          title="As Wide as the Tab"
          aria-pressed={!phone}
          onClick={() => setPhone(false)}
        >
          <Desktop />
        </button>
        <button
          className={cn(tool, phone && "bg-accent text-foreground")}
          title={`As Wide as a Phone (${PHONE_WIDTH} Pixels)`}
          aria-pressed={phone}
          onClick={() => setPhone(true)}
        >
          <Phone />
        </button>
        <button className={tool} title="Open in Browser" aria-label="Open in browser" onClick={() => openExternal(at)}>
          <Elsewhere />
        </button>
      </div>
      <div className={cn("flex min-h-0 flex-1 justify-center", phone && "bg-foreground/4 py-4")}>
        <iframe
          key={loads}
          title={`localhost:${port}`}
          src={at}
          sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-modals allow-downloads"
          className={cn("h-full border-0 bg-white", phone ? "rounded-xl shadow-lg ring-1 ring-border" : "w-full")}
          style={phone ? { width: PHONE_WIDTH } : undefined}
        />
      </div>
    </div>
  );
}
