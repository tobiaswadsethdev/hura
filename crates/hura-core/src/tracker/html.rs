//! Rich text written as HTML -- Fizzy's, which is Rails' ActionText -- read
//! into the same tree a Jira document becomes, and Markdown written from
//! here sent back as HTML.
//!
//! **Read, never rendered.** The reasoning in [`super::doc`] holds for HTML
//! even more than for ADF: markup anyone who can comment on a card controls
//! is not something to hand a webview. So it is parsed, and only the shapes
//! [`Block`] and [`Inline`] have survive: no tag, no attribute, no style,
//! and a link only where [`doc::safe`] allows it.
//!
//! **A small, forgiving parser** rather than an HTML5 one. What comes back is
//! what Fizzy's editor writes and its sanitizer lets through -- paragraphs,
//! headings, lists, code, quotes, rules, the inline marks, links and
//! attachments -- and anything else keeps its text. A tag left open is
//! closed by its parent's end; one closed that was never opened is ignored.
//!
//! **Attachments are named, not fetched.** An image in a card is behind a
//! redirect to wherever the site keeps its files, which hura cloud does not
//! follow, so it reads as its file name, as an unknown Jira image does.

use super::doc::{self, Block, Cell, Inline, Item};

/// How deep a document may nest before the rest of it is flattened: as in
/// [`super::doc`], a bound that keeps a hostile one off the stack.
const DEPTH: usize = 24;

/// How many open elements the parser keeps track of. Past this a tag is read
/// as if it were not there, which keeps the tree -- and dropping it -- shallow.
const OPEN: usize = 64;

/// Elements that never have children.
const VOID: &[&str] = &[
    "br", "hr", "img", "input", "meta", "link", "area", "base", "col", "embed", "source", "wbr",
];

/// Elements whose text is never shown.
const HIDDEN: &[&str] = &["script", "style", "template", "head", "title", "noscript"];

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Element {
        name: String,
        attrs: Vec<(String, String)>,
        children: Vec<Node>,
    },
    Text(String),
}

impl Node {
    fn attr(&self, key: &str) -> Option<&str> {
        match self {
            Node::Element { attrs, .. } => attrs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
                .filter(|v| !v.is_empty()),
            Node::Text(_) => None,
        }
    }
}

/// The document an HTML fragment describes.
pub fn from_html(html: &str) -> Vec<Block> {
    blocks(&parse(html), 0)
}

/// Markdown written here, as the HTML a Fizzy rich text field takes: through
/// the same reading of Markdown a Jira save makes, so the two agree on what
/// was written.
pub fn from_markdown(markdown: &str) -> String {
    to_html(&doc::from_adf(&super::markdown::to_adf(markdown)))
}

// ----------------------------------------------------------------- parsing

enum Token {
    Open {
        name: String,
        attrs: Vec<(String, String)>,
        closed: bool,
    },
    Close(String),
    Text(String),
}

fn tokens(html: &str) -> Vec<Token> {
    let s = html;
    let mut out = Vec::new();
    let mut i = 0;
    let mut text = String::new();
    while i < s.len() {
        let rest = &s[i..];
        if !rest.starts_with('<') {
            let end = rest.find('<').unwrap_or(rest.len());
            text.push_str(&rest[..end]);
            i += end;
            continue;
        }
        if let Some(after) = rest.strip_prefix("<!--") {
            i += 4 + after.find("-->").map_or(after.len(), |e| e + 3);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            i += rest.find('>').map_or(rest.len(), |e| e + 1);
            continue;
        }
        let closing = rest.starts_with("</");
        let from = if closing { 2 } else { 1 };
        let name: String = rest[from..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
            // A `<` that starts no tag: `a < b`.
            text.push('<');
            i += 1;
            continue;
        }
        if !text.is_empty() {
            out.push(Token::Text(decode(&std::mem::take(&mut text))));
        }
        let name = name.to_ascii_lowercase();
        let (attrs, closed, len) = tag(&rest[from + name.len()..]);
        i += from + name.len() + len;
        if closing {
            out.push(Token::Close(name));
        } else if HIDDEN.contains(&name.as_str()) && !closed {
            // Its contents are text to the parser and nothing to the reader.
            let end = format!("</{name}");
            let lower = s[i..].to_ascii_lowercase();
            i += lower.find(&end).unwrap_or(lower.len());
        } else {
            out.push(Token::Open {
                name,
                attrs,
                closed,
            });
        }
    }
    if !text.is_empty() {
        out.push(Token::Text(decode(&text)));
    }
    out
}

/// A tag's attributes, whether it closed itself, and how far its `>` is.
fn tag(s: &str) -> (Vec<(String, String)>, bool, usize) {
    let b = s.as_bytes();
    let mut attrs = Vec::new();
    let mut i = 0;
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            if b[i] == b'/' && b.get(i + 1) == Some(&b'>') {
                return (attrs, true, i + 2);
            }
            i += 1;
        }
        if i >= b.len() {
            return (attrs, false, i);
        }
        if b[i] == b'>' {
            return (attrs, false, i + 1);
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let key = s[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if b.get(i) == Some(&b'=') {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            match b.get(i) {
                Some(&q) if q == b'"' || q == b'\'' => {
                    let end = s[i + 1..].find(q as char).map_or(s.len(), |e| i + 1 + e);
                    value = decode(&s[i + 1..end]);
                    i = (end + 1).min(s.len());
                }
                _ => {
                    let start = i;
                    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                        i += 1;
                    }
                    value = decode(&s[start..i]);
                }
            }
        }
        if !key.is_empty() {
            attrs.push((key, value));
        }
    }
}

/// Character references: the named ones an editor writes, and numbers.
fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest[1..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '#'))
            .map(|e| e + 1);
        let found = end
            .filter(|&e| rest.as_bytes().get(e) == Some(&b';'))
            .and_then(|e| entity(&rest[1..e]).map(|c| (c, e + 1)));
        match found {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    if let Some(n) = name.strip_prefix('#') {
        let code = match n.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => n.parse().ok()?,
        };
        return char::from_u32(code).filter(|c| *c != '\0');
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "bull" => '•',
        "middot" => '·',
        "copy" => '©',
        "times" => '×',
        _ => return None,
    })
}

fn parse(html: &str) -> Vec<Node> {
    struct Open {
        name: String,
        attrs: Vec<(String, String)>,
        children: Vec<Node>,
    }
    fn close(stack: &mut Vec<Open>, root: &mut Vec<Node>) {
        if let Some(o) = stack.pop() {
            let node = Node::Element {
                name: o.name,
                attrs: o.attrs,
                children: o.children,
            };
            match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None => root.push(node),
            }
        }
    }

    let mut root = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    for token in tokens(html) {
        match token {
            Token::Text(t) => match stack.last_mut() {
                Some(o) => o.children.push(Node::Text(t)),
                None => root.push(Node::Text(t)),
            },
            Token::Open {
                name,
                attrs,
                closed,
            } => {
                // A new item or paragraph ends the one before it, as a
                // browser reads `<li>a<li>b`.
                if matches!(name.as_str(), "li" | "p")
                    && stack.last().is_some_and(|o| o.name == name)
                {
                    close(&mut stack, &mut root);
                }
                if closed || VOID.contains(&name.as_str()) {
                    let node = Node::Element {
                        name,
                        attrs,
                        children: Vec::new(),
                    };
                    match stack.last_mut() {
                        Some(o) => o.children.push(node),
                        None => root.push(node),
                    }
                } else if stack.len() < OPEN {
                    stack.push(Open {
                        name,
                        attrs,
                        children: Vec::new(),
                    });
                }
            }
            Token::Close(name) => {
                if let Some(at) = stack.iter().rposition(|o| o.name == name) {
                    while stack.len() > at {
                        close(&mut stack, &mut root);
                    }
                }
            }
        }
    }
    while !stack.is_empty() {
        close(&mut stack, &mut root);
    }
    root
}

// ---------------------------------------------------------------- to a doc

#[derive(Clone, Default)]
struct Marks {
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
    underline: bool,
    href: Option<String>,
}

fn blocks(nodes: &[Node], depth: usize) -> Vec<Block> {
    let mut out = Vec::new();
    let mut line = Vec::new();
    if depth > DEPTH {
        // Too deep to keep its shape: its text, as one paragraph.
        let text = text_of(nodes, false);
        push_text(&mut line, &text, &Marks::default());
        flush(&mut out, &mut line);
        return out;
    }
    for node in nodes {
        block(node, depth, &mut out, &mut line);
    }
    flush(&mut out, &mut line);
    out
}

fn block(node: &Node, depth: usize, out: &mut Vec<Block>, line: &mut Vec<Inline>) {
    let Node::Element { name, children, .. } = node else {
        inline(node, depth, &Marks::default(), line);
        return;
    };
    let mut end_line = |out: &mut Vec<Block>| flush(out, line);
    match name.as_str() {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            end_line(out);
            let mut content = Vec::new();
            for c in children {
                inline(c, depth + 1, &Marks::default(), &mut content);
            }
            tidy(&mut content);
            if !content.is_empty() {
                out.push(Block::Heading {
                    level: name[1..].parse().unwrap_or(1),
                    content,
                });
            }
        }
        "ul" | "ol" => {
            end_line(out);
            let items: Vec<Item> = children
                .iter()
                .filter(|c| !matches!(c, Node::Text(t) if t.trim().is_empty()))
                .map(|c| Item {
                    checked: None,
                    content: match c {
                        Node::Element { name, children, .. } if name == "li" => {
                            blocks(children, depth + 1)
                        }
                        other => blocks(std::slice::from_ref(other), depth + 1),
                    },
                })
                .collect();
            if !items.is_empty() {
                out.push(Block::List {
                    ordered: name == "ol",
                    start: node.attr("start").and_then(|s| s.parse().ok()).unwrap_or(1),
                    items,
                });
            }
        }
        "pre" => {
            end_line(out);
            let language = node.attr("data-language").map(str::to_string).or_else(|| {
                children.iter().find_map(|c| {
                    c.attr("class")?
                        .split_whitespace()
                        .find_map(|k| k.strip_prefix("language-"))
                        .map(str::to_string)
                })
            });
            let text = text_of(children, true);
            out.push(Block::Code {
                language,
                text: text.trim_end_matches('\n').to_string(),
            });
        }
        "blockquote" => {
            end_line(out);
            out.push(Block::Quote {
                content: blocks(children, depth + 1),
            });
        }
        "hr" => {
            end_line(out);
            out.push(Block::Rule);
        }
        "table" => {
            end_line(out);
            let mut rows = Vec::new();
            table_rows(children, depth, &mut rows);
            if !rows.is_empty() {
                out.push(Block::Table { rows });
            }
        }
        "action-text-attachment" | "figure" | "img" => match mention(node) {
            Some(m) => line.push(m),
            None => {
                end_line(out);
                out.push(Block::Attachment {
                    name: attachment_name(node),
                    image: None,
                });
            }
        },
        "br" => line.push(Inline::Break),
        "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "aside" | "nav"
        | "li" | "dl" | "dt" | "dd" | "details" | "summary" | "body" | "html" | "center"
        | "address" | "figcaption" => {
            end_line(out);
            out.extend(blocks(children, depth + 1));
        }
        _ => inline(node, depth, &Marks::default(), line),
    }
}

fn table_rows(nodes: &[Node], depth: usize, rows: &mut Vec<Vec<Cell>>) {
    for node in nodes {
        let Node::Element { name, children, .. } = node else {
            continue;
        };
        match name.as_str() {
            "thead" | "tbody" | "tfoot" => table_rows(children, depth, rows),
            "tr" => rows.push(
                children
                    .iter()
                    .filter_map(|c| match c {
                        Node::Element { name, children, .. } if name == "td" || name == "th" => {
                            Some(Cell {
                                header: name == "th",
                                content: blocks(children, depth + 1),
                            })
                        }
                        _ => None,
                    })
                    .collect(),
            ),
            _ => {}
        }
    }
}

fn inline(node: &Node, depth: usize, marks: &Marks, line: &mut Vec<Inline>) {
    let (name, children) = match node {
        Node::Text(t) => {
            push_text(line, t, marks);
            return;
        }
        Node::Element { name, children, .. } => (name.as_str(), children),
    };
    if depth > DEPTH {
        push_text(line, &text_of(children, false), marks);
        return;
    }
    let mut marks = marks.clone();
    match name {
        "br" => {
            line.push(Inline::Break);
            return;
        }
        "action-text-attachment" | "img" | "figure" => {
            if let Some(m) = mention(node) {
                line.push(m);
            }
            return;
        }
        "strong" | "b" => marks.bold = true,
        "em" | "i" => marks.italic = true,
        "s" | "del" | "strike" => marks.strike = true,
        "u" | "ins" => marks.underline = true,
        "code" | "kbd" | "samp" | "tt" => marks.code = true,
        "a" => marks.href = node.attr("href").and_then(doc::safe),
        _ => {}
    }
    for c in children {
        inline(c, depth + 1, &marks, line);
    }
}

/// A mention of a person, which ActionText writes as an attachment of the
/// person: `@Name`, as the author saw it.
fn mention(node: &Node) -> Option<Inline> {
    let kind = node.attr("content-type").unwrap_or_default();
    if !kind.contains("mention") {
        return None;
    }
    let Node::Element { children, .. } = node else {
        return None;
    };
    let name = text_of(children, false);
    let name = name.trim().trim_start_matches('@').trim();
    let name = if name.is_empty() {
        find_attr(children, "alt").unwrap_or_default()
    } else {
        name.to_string()
    };
    Some(Inline::Mention { name })
}

/// What an attachment is called: its file name, its caption, the picture's
/// description, or the last part of where it is.
fn attachment_name(node: &Node) -> String {
    let Node::Element { children, .. } = node else {
        return String::new();
    };
    let caption = find(children, "figcaption").map(|c| match c {
        Node::Element { children, .. } => text_of(children, false).trim().to_string(),
        Node::Text(_) => String::new(),
    });
    node.attr("filename")
        .or_else(|| node.attr("caption"))
        .map(str::to_string)
        .or(caption.filter(|c| !c.is_empty()))
        .or_else(|| node.attr("alt").map(str::to_string))
        .or_else(|| find_attr(children, "alt"))
        .or_else(|| {
            let src = node
                .attr("src")
                .or_else(|| node.attr("url"))
                .map(str::to_string)
                .or_else(|| find_attr(children, "src"))?;
            let path = src.split(['?', '#']).next().unwrap_or_default();
            path.rsplit('/')
                .next()
                .filter(|n| !n.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "attachment".to_string())
}

fn find<'a>(nodes: &'a [Node], name: &str) -> Option<&'a Node> {
    nodes.iter().find_map(|n| match n {
        Node::Element { name: n_name, .. } if n_name == name => Some(n),
        Node::Element { children, .. } => find(children, name),
        Node::Text(_) => None,
    })
}

fn find_attr(nodes: &[Node], key: &str) -> Option<String> {
    nodes.iter().find_map(|n| match n {
        Node::Element { children, .. } => n
            .attr(key)
            .map(str::to_string)
            .or_else(|| find_attr(children, key)),
        Node::Text(_) => None,
    })
}

/// The text in some nodes: as written inside `pre`, and otherwise with a
/// break as a new line and every other run of space as one.
fn text_of(nodes: &[Node], pre: bool) -> String {
    fn walk(nodes: &[Node], out: &mut String, depth: usize) {
        for n in nodes {
            match n {
                Node::Text(t) => out.push_str(t),
                Node::Element { name, .. } if name == "br" => out.push('\n'),
                Node::Element { children, .. } if depth < OPEN => walk(children, out, depth + 1),
                Node::Element { .. } => {}
            }
        }
    }
    let mut out = String::new();
    walk(nodes, &mut out, 0);
    if pre {
        out
    } else {
        out.lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Text into a line, its spaces folded as a browser folds them.
fn push_text(line: &mut Vec<Inline>, text: &str, marks: &Marks) {
    let mut folded = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                folded.push(' ');
                space = false;
            }
            folded.push(c);
        }
    }
    if space {
        folded.push(' ');
    }
    // No space at the start of a line, or twice across two runs.
    let after_space = match line.last() {
        None | Some(Inline::Break) => true,
        Some(Inline::Text { text, .. }) => text.ends_with(' '),
        Some(_) => false,
    };
    if after_space {
        folded = folded.trim_start().to_string();
    }
    if folded.is_empty() {
        return;
    }
    if let Some(Inline::Text {
        text,
        bold,
        italic,
        code,
        strike,
        underline,
        href,
    }) = line.last_mut()
        && (*bold, *italic, *code, *strike, *underline, href.as_ref())
            == (
                marks.bold,
                marks.italic,
                marks.code,
                marks.strike,
                marks.underline,
                marks.href.as_ref(),
            )
    {
        text.push_str(&folded);
        return;
    }
    line.push(Inline::Text {
        text: folded,
        bold: marks.bold,
        italic: marks.italic,
        code: marks.code,
        strike: marks.strike,
        underline: marks.underline,
        href: marks.href.clone(),
    });
}

/// A line without the space and breaks at its ends.
fn tidy(line: &mut Vec<Inline>) {
    while matches!(line.last(), Some(Inline::Break)) {
        line.pop();
    }
    while matches!(line.first(), Some(Inline::Break)) {
        line.remove(0);
    }
    if let Some(Inline::Text { text, .. }) = line.last_mut() {
        let trimmed = text.trim_end().len();
        text.truncate(trimmed);
    }
    if let Some(Inline::Text { text, .. }) = line.first_mut()
        && text.starts_with(char::is_whitespace)
    {
        *text = text.trim_start().to_string();
    }
    line.retain(|i| !matches!(i, Inline::Text { text, .. } if text.is_empty()));
}

/// The line so far as a paragraph, unless there is nothing in it.
fn flush(out: &mut Vec<Block>, line: &mut Vec<Inline>) {
    let mut content = std::mem::take(line);
    tidy(&mut content);
    if !content.is_empty() {
        out.push(Block::Paragraph { content });
    }
}

// ---------------------------------------------------------------- to HTML

/// A document as HTML: the tags every rich text editor takes.
pub fn to_html(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        render_block(b, &mut out);
    }
    out
}

fn render_block(block: &Block, out: &mut String) {
    match block {
        Block::Paragraph { content } => {
            out.push_str("<p>");
            render_inlines(content, out);
            out.push_str("</p>");
        }
        Block::Heading { level, content } => {
            let level = (*level).clamp(1, 6);
            out.push_str(&format!("<h{level}>"));
            render_inlines(content, out);
            out.push_str(&format!("</h{level}>"));
        }
        Block::List {
            ordered,
            start,
            items,
        } => {
            match (ordered, start) {
                (true, 1) => out.push_str("<ol>"),
                (true, n) => out.push_str(&format!("<ol start=\"{n}\">")),
                (false, _) => out.push_str("<ul>"),
            }
            for item in items {
                out.push_str("<li>");
                match item.checked {
                    Some(true) => out.push_str("[x] "),
                    Some(false) => out.push_str("[ ] "),
                    None => {}
                }
                // An item of one paragraph is that paragraph's text, as an
                // editor writes it, rather than a paragraph in an item.
                match item.content.as_slice() {
                    [Block::Paragraph { content }] => render_inlines(content, out),
                    blocks => {
                        for b in blocks {
                            render_block(b, out);
                        }
                    }
                }
                out.push_str("</li>");
            }
            out.push_str(if *ordered { "</ol>" } else { "</ul>" });
        }
        Block::Code { language, text } => {
            match language {
                Some(l) => out.push_str(&format!("<pre data-language=\"{}\">", escape(l))),
                None => out.push_str("<pre>"),
            }
            out.push_str(&escape(text));
            out.push_str("</pre>");
        }
        Block::Quote { content } => {
            out.push_str("<blockquote>");
            for b in content {
                render_block(b, out);
            }
            out.push_str("</blockquote>");
        }
        Block::Panel { content, .. } => {
            for b in content {
                render_block(b, out);
            }
        }
        Block::Expand { title, content } => {
            if !title.is_empty() {
                out.push_str(&format!("<p><strong>{}</strong></p>", escape(title)));
            }
            for b in content {
                render_block(b, out);
            }
        }
        Block::Rule => out.push_str("<hr>"),
        Block::Table { rows } => {
            out.push_str("<table>");
            for row in rows {
                out.push_str("<tr>");
                for cell in row {
                    let tag = if cell.header { "th" } else { "td" };
                    out.push_str(&format!("<{tag}>"));
                    for b in &cell.content {
                        render_block(b, out);
                    }
                    out.push_str(&format!("</{tag}>"));
                }
                out.push_str("</tr>");
            }
            out.push_str("</table>");
        }
        // Markdown written here has none, and one read from the site cannot
        // be sent back as anything but its name: said before saving.
        Block::Attachment { .. } => {}
        Block::Card { url } => {
            let url = escape(url);
            out.push_str(&format!("<p><a href=\"{url}\">{url}</a></p>"));
        }
    }
}

fn render_inlines(content: &[Inline], out: &mut String) {
    for i in content {
        match i {
            Inline::Text {
                text,
                bold,
                italic,
                code,
                strike,
                underline,
                href,
            } => {
                let mut s = escape(text);
                for (on, tag) in [
                    (*code, "code"),
                    (*strike, "s"),
                    (*underline, "u"),
                    (*italic, "em"),
                    (*bold, "strong"),
                ] {
                    if on {
                        s = format!("<{tag}>{s}</{tag}>");
                    }
                }
                if let Some(href) = href {
                    s = format!("<a href=\"{}\">{s}</a>", escape(href));
                }
                out.push_str(&s);
            }
            Inline::Mention { name } => out.push_str(&escape(&format!("@{name}"))),
            Inline::Emoji { text } | Inline::Status { text, .. } => out.push_str(&escape(text)),
            Inline::Date { at } => out.push_str(&super::markdown::iso_date(*at)),
            Inline::Break => out.push_str("<br>"),
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Inline {
        doc::text_run(t)
    }

    fn bold(t: &str) -> Inline {
        let Inline::Text { text, .. } = text(t) else {
            unreachable!()
        };
        Inline::Text {
            text,
            bold: true,
            italic: false,
            code: false,
            strike: false,
            underline: false,
            href: None,
        }
    }

    fn link(t: &str, href: &str) -> Inline {
        let Inline::Text { text, .. } = text(t) else {
            unreachable!()
        };
        Inline::Text {
            text,
            bold: false,
            italic: false,
            code: false,
            strike: false,
            underline: false,
            href: Some(href.into()),
        }
    }

    fn para(t: &str) -> Block {
        Block::Paragraph {
            content: vec![text(t)],
        }
    }

    #[test]
    fn actiontext_reads_as_paragraphs_with_their_marks_and_links() {
        let blocks = from_html(
            "<div class=\"action-text-content\">\n  <p>Keep <strong>it</strong>  short &amp; <a href=\"https://x.test/a?b=1&amp;c=2\">sweet</a>.</p>\n<p>Second<br>line</p></div>",
        );
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph {
                    content: vec![
                        text("Keep "),
                        bold("it"),
                        text(" short & "),
                        link("sweet", "https://x.test/a?b=1&c=2"),
                        text("."),
                    ]
                },
                Block::Paragraph {
                    content: vec![text("Second"), Inline::Break, text("line")]
                },
            ]
        );
    }

    #[test]
    fn a_trix_div_of_lines_is_a_paragraph_and_bare_text_is_too() {
        assert_eq!(from_html("Hello, World!"), vec![para("Hello, World!")]);
        assert_eq!(
            from_html("<div>one<br>two<br><br></div>"),
            vec![Block::Paragraph {
                content: vec![text("one"), Inline::Break, text("two")]
            }]
        );
        assert_eq!(from_html("  <div> </div>\n"), vec![]);
    }

    #[test]
    fn lists_code_quotes_headings_and_rules_keep_their_shape() {
        let blocks = from_html(
            "<h2>Plan</h2><ul><li>one</li><li>two<ol start=\"3\"><li>deep</li></ol></li></ul>\
             <pre data-language=\"rust\">let a = 1 &lt; 2;\n  b()</pre><blockquote><p>said</p></blockquote><hr>",
        );
        assert_eq!(
            blocks,
            vec![
                Block::Heading {
                    level: 2,
                    content: vec![text("Plan")]
                },
                Block::List {
                    ordered: false,
                    start: 1,
                    items: vec![
                        Item {
                            checked: None,
                            content: vec![para("one")]
                        },
                        Item {
                            checked: None,
                            content: vec![
                                para("two"),
                                Block::List {
                                    ordered: true,
                                    start: 3,
                                    items: vec![Item {
                                        checked: None,
                                        content: vec![para("deep")]
                                    }]
                                }
                            ]
                        },
                    ]
                },
                Block::Code {
                    language: Some("rust".into()),
                    text: "let a = 1 < 2;\n  b()".into()
                },
                Block::Quote {
                    content: vec![para("said")]
                },
                Block::Rule,
            ]
        );
    }

    #[test]
    fn nothing_dangerous_gets_through() {
        let blocks = from_html(
            "<p onclick=\"x()\">a<script>alert(1)</script><a href=\"javascript:alert(1)\">b</a><style>p{}</style><img src=x onerror=y></p>",
        );
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph {
                    content: vec![text("ab")]
                },
                Block::Attachment {
                    name: "x".into(),
                    image: None
                },
            ]
        );
    }

    #[test]
    fn attachments_are_named_and_mentions_are_people() {
        let blocks = from_html(
            "<p>Hi <action-text-attachment content-type=\"application/vnd.actiontext.mention\" sgid=\"s\"><img alt=\"Ada Lovelace\" src=\"/a.png\"></action-text-attachment>, see</p>\
             <action-text-attachment content-type=\"image/png\" filename=\"shot.png\" url=\"https://f.test/x/shot.png\"><figure class=\"attachment\"><img src=\"https://f.test/x/shot.png\"><figcaption>Shot</figcaption></figure></action-text-attachment>\
             <figure><img src=\"https://f.test/rails/blob/pic.jpg?x=1\"></figure>",
        );
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph {
                    content: vec![
                        text("Hi "),
                        Inline::Mention {
                            name: "Ada Lovelace".into()
                        },
                        text(", see"),
                    ]
                },
                Block::Attachment {
                    name: "shot.png".into(),
                    image: None
                },
                Block::Attachment {
                    name: "pic.jpg".into(),
                    image: None
                },
            ]
        );
    }

    #[test]
    fn broken_markup_still_reads() {
        assert_eq!(
            from_html("<p>open <b>bold</p><li>stray</li></div>a < b &bogus; &#x41;"),
            vec![
                Block::Paragraph {
                    content: vec![text("open "), bold("bold")]
                },
                para("stray"),
                para("a < b &bogus; A"),
            ]
        );
        // Deep enough to matter, and read without falling over.
        let deep = "<div>".repeat(10_000) + "x" + &"</div>".repeat(10_000);
        assert_eq!(from_html(&deep), vec![para("x")]);
    }

    #[test]
    fn markdown_goes_out_as_html_and_reads_back_the_same() {
        let md = "Keep it **short** & [sweet](https://x.test).\n\n- one\n- two\n\n```\na < b\n```";
        let html = from_markdown(md);
        assert_eq!(
            html,
            "<p>Keep it <strong>short</strong> &amp; <a href=\"https://x.test\">sweet</a>.</p>\
             <ul><li>one</li><li>two</li></ul><pre>a &lt; b</pre>"
        );
        assert_eq!(
            super::super::markdown::to_markdown(&from_html(&html)).text,
            super::super::markdown::to_markdown(&doc::from_adf(&super::super::markdown::to_adf(
                md
            )))
            .text
        );
    }
}
