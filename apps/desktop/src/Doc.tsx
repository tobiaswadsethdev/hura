// A tracker's rich text, drawn from the typed tree `tracker::doc` makes of it.
//
// Elements, never markup: there is no `dangerouslySetInnerHTML` here and no
// HTML anywhere on the way in, so a comment cannot carry a script into the
// window. A link is an anchor only when the Rust side kept its `href`, which it
// does for `http`, `https` and `mailto` and nothing else.

import { createContext, useContext, useEffect, useState } from "react";

import { api, messageOf } from "./api";
import type { DocBlock } from "./gen/DocBlock";
import type { DocInline } from "./gen/DocInline";

// The tracker a document's images are attachments of. Without one, an image
// is named, as it always was.
const DocTracker = createContext<string | null>(null);

export function Doc({ blocks, tracker = null }: { blocks: DocBlock[]; tracker?: string | null }) {
  return (
    <DocTracker.Provider value={tracker}>
      <div className="doc">
        {blocks.map((b, i) => (
          <Block key={i} block={b} />
        ))}
      </div>
    </DocTracker.Provider>
  );
}

function Blocks({ blocks }: { blocks: DocBlock[] }) {
  return (
    <>
      {blocks.map((b, i) => (
        <Block key={i} block={b} />
      ))}
    </>
  );
}

function Block({ block: b }: { block: DocBlock }) {
  switch (b.type) {
    case "paragraph":
      return (
        <p>
          <Inlines content={b.content} />
        </p>
      );
    case "heading": {
      // Two levels below the panel's own title, so a document's `h1` is not
      // louder than the ticket it is in.
      const level = Math.min(6, b.level + 2);
      const H = `h${level}` as "h3";
      return (
        <H>
          <Inlines content={b.content} />
        </H>
      );
    }
    case "list": {
      const items = b.items.map((item, i) => (
        <li key={i} className={item.checked === null ? undefined : "check"}>
          {item.checked !== null && (
            <input type="checkbox" checked={item.checked} readOnly tabIndex={-1} />
          )}
          <div>
            <Blocks blocks={item.content} />
          </div>
        </li>
      ));
      return b.ordered ? <ol start={b.start}>{items}</ol> : <ul>{items}</ul>;
    }
    case "code":
      return (
        <pre className="doc-code" data-language={b.language ?? undefined}>
          <code>{b.text}</code>
        </pre>
      );
    case "quote":
      return (
        <blockquote>
          <Blocks blocks={b.content} />
        </blockquote>
      );
    case "panel":
      return (
        <div className={`doc-panel ${b.kind}`}>
          <Blocks blocks={b.content} />
        </div>
      );
    case "expand":
      return (
        <div className="doc-expand">
          {b.title && <div className="doc-expand-title">{b.title}</div>}
          <Blocks blocks={b.content} />
        </div>
      );
    case "rule":
      return <hr />;
    case "table":
      return (
        <div className="doc-table">
          <table>
            <tbody>
              {b.rows.map((row, r) => (
                <tr key={r}>
                  {row.map((cell, c) =>
                    cell.header ? (
                      <th key={c}>
                        <Blocks blocks={cell.content} />
                      </th>
                    ) : (
                      <td key={c}>
                        <Blocks blocks={cell.content} />
                      </td>
                    ),
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case "attachment":
      return b.image ? (
        <Picture id={b.image} name={b.name} />
      ) : (
        <p className="doc-attachment">attachment: {b.name}</p>
      );
    case "card":
      return (
        <p>
          <a href={b.url} target="_blank" rel="noreferrer">
            {b.url}
          </a>
        </p>
      );
  }
}

// Each image once per window, as a `data:` URL: the same screenshot is quoted
// in comment after comment. A failure is forgotten, so reopening the ticket
// asks again.
const pictures = new Map<string, Promise<string>>();

function picture(tracker: string, id: string): Promise<string> {
  const key = `${tracker}\u0000${id}`;
  let found = pictures.get(key);
  if (!found) {
    found = api.ticketImage(tracker, id).then((i) => `data:${i.mime};base64,${i.data}`);
    found.catch(() => pictures.delete(key));
    pictures.set(key, found);
  }
  return found;
}

function Picture({ id, name }: { id: string; name: string }) {
  const tracker = useContext(DocTracker);
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  useEffect(() => {
    if (!tracker) return;
    let live = true;
    setSrc(null);
    setFailed(null);
    picture(tracker, id).then(
      (s) => live && setSrc(s),
      (e) => live && setFailed(messageOf(e)),
    );
    return () => {
      live = false;
    };
  }, [tracker, id]);

  if (src) {
    return (
      <p className="doc-image">
        <img src={src} alt={name} title={name} />
      </p>
    );
  }
  const said = !tracker ? `attachment: ${name}` : failed ? `image: ${name} (not read)` : `image: ${name}…`;
  return (
    <p className="doc-attachment" title={failed ?? undefined}>
      {said}
    </p>
  );
}

function Inlines({ content }: { content: DocInline[] }) {
  return (
    <>
      {content.map((n, i) => (
        <Inline key={i} node={n} />
      ))}
    </>
  );
}

function Inline({ node: n }: { node: DocInline }) {
  switch (n.type) {
    case "text": {
      let el: React.ReactNode = n.text;
      if (n.code) el = <code>{el}</code>;
      if (n.bold) el = <strong>{el}</strong>;
      if (n.italic) el = <em>{el}</em>;
      if (n.strike) el = <s>{el}</s>;
      if (n.underline) el = <u>{el}</u>;
      if (n.href)
        el = (
          <a href={n.href} target="_blank" rel="noreferrer" title={n.href}>
            {el}
          </a>
        );
      return <>{el}</>;
    }
    case "mention":
      return <span className="doc-mention">@{n.name}</span>;
    case "emoji":
      return <>{n.text}</>;
    case "status":
      return <span className={`doc-status ${n.color}`}>{n.text}</span>;
    case "date":
      return <span className="doc-date">{new Date(n.at).toLocaleDateString()}</span>;
    case "break":
      return <br />;
  }
}
