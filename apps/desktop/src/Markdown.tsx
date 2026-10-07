// Markdown, as the agent writes it, drawn as elements.
//
// The same rule as `Doc.tsx`: no HTML on the way in, so nothing an agent
// writes can put a script in the window. react-markdown builds React elements
// from the syntax tree and ignores raw HTML unless told otherwise, which is
// why it is the renderer rather than a parser feeding `innerHTML`. Links are
// anchors only for the three schemes the window opens; anything else is its
// text.

import { memo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

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
  img: ({ alt }) => <>{alt ? `[${alt}]` : null}</>,
};

export const Markdown = memo(function Markdown({ text }: { text: string }) {
  return (
    <div className="doc">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components} skipHtml>
        {text}
      </ReactMarkdown>
    </div>
  );
});
