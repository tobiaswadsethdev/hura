// Markdown, as the agent writes it, drawn as elements.
//
// The same rule as `Doc.tsx`: no HTML on the way in, so nothing an agent
// writes can put a script in the window. react-markdown builds React elements
// from the syntax tree and ignores raw HTML unless told otherwise, which is
// why it is the renderer rather than a parser feeding `innerHTML`. Links are
// anchors only for the three schemes the window opens; anything else is its
// text. An image is drawn only when it is a file in the working copy, read
// through the server like any other, or -- in a pull request's text -- one of
// the forge's own images, read through the tracker: the window loads nothing
// from the network on anybody's say-so.

import { createContext, memo, useContext, useEffect, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { api } from "./api";
import type { Picture } from "./gen/Picture";

/// Reads a working-copy path as an image, for the session a reply is about.
/// Absent where there is no session, and then an image is its alt text.
export type ReadPicture = (path: string) => Promise<Picture | undefined>;
export const Pictures = createContext<ReadPicture | null>(null);

/// Where the working copy is in the sandbox, so an absolute path the agent
/// writes can be read as a relative one.
const REPO = "/sandbox/repo/";

/// The working-copy path an image's source names, or null for one that is
/// not a file there: a URL of any scheme.
export function workingPath(src: string): string | null {
  if (/^[a-z][a-z0-9+.-]*:/i.test(src)) return null;
  const path = src.startsWith(REPO) ? src.slice(REPO.length) : src.replace(/^\.\//, "");
  return path && !path.startsWith("/") ? decodeURI(path) : null;
}

/// The tracker a pull request's text is from, whose images it reads them
/// through. Absent everywhere else.
const Forge = createContext<string | null>(null);

// Each image once per window: a pull request is read again on every refresh,
// and its images with it. A failure is forgotten, so the next read asks again.
const forgePictures = new Map<string, Promise<Picture>>();

function forgePicture(tracker: string, url: string): Promise<Picture> {
  const key = `${tracker}\u0000${url}`;
  let found = forgePictures.get(key);
  if (!found) {
    found = api.pullImage(tracker, url);
    found.catch(() => forgePictures.delete(key));
    forgePictures.set(key, found);
  }
  return found;
}

function Img({ src, alt }: { src?: string; alt?: string }) {
  const read = useContext(Pictures);
  const forge = useContext(Forge);
  const path = src ? workingPath(src) : null;
  const url = forge && src?.startsWith("https://") ? src : null;
  const [picture, setPicture] = useState<Picture | null>(null);
  useEffect(() => {
    const reading = path && read ? read(path) : url && forge ? forgePicture(forge, url) : null;
    if (!reading) return;
    let live = true;
    reading.then((p) => live && setPicture(p ?? null)).catch(() => {});
    return () => {
      live = false;
    };
  }, [read, path, forge, url]);
  if (!picture) return <>{alt ? `[${alt}]` : null}</>;
  return <img className="doc-img" src={`data:${picture.mime};base64,${picture.data}`} alt={alt ?? path ?? ""} />;
}

const OPENABLE = /^(https?:|mailto:)/i;

const components: Components = {
  a: ({ href, children }) =>
    href && OPENABLE.test(href) ? (
      <a href={href} target="_blank" rel="noreferrer">
        {children}
      </a>
    ) : (
      <>{children}</>
    ),
  // A heading in a reply is a section of a message, not of the window, so
  // they are all drawn two ranks down, as `Doc` draws a tracker's.
  h1: ({ children }) => <h3>{children}</h3>,
  h2: ({ children }) => <h4>{children}</h4>,
  h3: ({ children }) => <h5>{children}</h5>,
  pre: ({ children }) => <pre className="doc-code">{children}</pre>,
  table: ({ children }) => (
    <div className="doc-table">
      <table>{children}</table>
    </div>
  ),
  img: ({ src, alt }) => <Img src={typeof src === "string" ? src : undefined} alt={alt} />,
};

/// `forge`: the tracker a pull request's text is from, for its images.
export const Markdown = memo(function Markdown({ text, forge = null }: { text: string; forge?: string | null }) {
  return (
    <Forge.Provider value={forge}>
      <div className="doc">
        <ReactMarkdown remarkPlugins={[remarkGfm]} components={components} skipHtml>
          {text}
        </ReactMarkdown>
      </div>
    </Forge.Provider>
  );
});
