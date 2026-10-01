//! Rich text from a tracker, as a tree a client can draw without trusting it.
//!
//! Jira Cloud writes descriptions and comments in the Atlassian Document
//! Format: a JSON tree of paragraphs, lists, code blocks, mentions and so on.
//! Jira will also hand back the same thing rendered as HTML, and that is the
//! version this deliberately does not use -- HTML from a tracker is markup
//! anyone who can comment on a ticket controls, and a webview that injects it
//! is a webview that runs it. A typed tree has no such door: every node is one
//! of the shapes below, a link is a link only if it is `http`, `https` or
//! `mailto`, and an image is its name.
//!
//! Read generously. ADF grows node types, and a description that used one this
//! does not know must still read: an unknown block keeps whatever text is
//! inside it, and an unknown inline keeps its text if it has any.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How deep a document may nest before the rest of it is dropped. Real ones
/// are a handful of levels -- a list in a panel in a table -- and a bound means
/// a hostile one cannot recurse this off the stack.
const DEPTH: usize = 24;

/// One block of a document.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "DocBlock"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Block {
    Paragraph {
        content: Vec<Inline>,
    },
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    /// Bulleted, numbered or a checklist. A checklist's items carry `checked`.
    List {
        ordered: bool,
        start: u32,
        items: Vec<Item>,
    },
    Code {
        language: Option<String>,
        text: String,
    },
    Quote {
        content: Vec<Block>,
    },
    /// An info, note, warning, error or success box.
    Panel {
        kind: String,
        content: Vec<Block>,
    },
    /// A collapsible section, drawn open: there is no reason to hide what the
    /// author wrote behind a click when the whole point is to read it here.
    Expand {
        title: String,
        content: Vec<Block>,
    },
    Rule,
    Table {
        rows: Vec<Vec<Cell>>,
    },
    /// An image or a file. Named rather than shown: fetching it needs the
    /// tracker's credentials, and the window loads nothing from the network.
    Attachment {
        name: String,
    },
    /// A link drawn as a card. Shown as the link.
    Card {
        url: String,
    },
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "DocItem"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// `Some` for a checklist item: whether it is done.
    pub checked: Option<bool>,
    pub content: Vec<Block>,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "DocCell"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub header: bool,
    pub content: Vec<Block>,
}

/// One run of text, or something inline that is not text.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "DocInline"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Inline {
    Text {
        text: String,
        bold: bool,
        italic: bool,
        code: bool,
        strike: bool,
        underline: bool,
        /// Only ever `http`, `https` or `mailto`. Anything else is dropped and
        /// the text stays as text.
        href: Option<String>,
    },
    /// `@Name`, as the author saw it.
    Mention {
        name: String,
    },
    Emoji {
        text: String,
    },
    /// A lozenge such as `IN REVIEW`, with Jira's colour name for it.
    Status {
        text: String,
        color: String,
    },
    /// A date, in epoch milliseconds, for the client to write in its own way.
    Date {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        at: i64,
    },
    Break,
}

/// The document an ADF value describes. Anything that is not one -- a `null`
/// description, a string from an older API -- reads as plain paragraphs or as
/// nothing.
pub fn from_adf(value: &Value) -> Vec<Block> {
    match value {
        Value::String(s) => plain(s),
        Value::Object(_) => blocks(value.get("content"), 0),
        _ => Vec::new(),
    }
}

/// Text with no markup, one paragraph per blank-line-separated run.
pub fn plain(text: &str) -> Vec<Block> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut content = Vec::new();
            for (i, line) in p.lines().enumerate() {
                if i > 0 {
                    content.push(Inline::Break);
                }
                content.push(text_run(line));
            }
            Block::Paragraph { content }
        })
        .collect()
}

fn text_run(text: &str) -> Inline {
    Inline::Text {
        text: text.to_string(),
        bold: false,
        italic: false,
        code: false,
        strike: false,
        underline: false,
        href: None,
    }
}

fn children(content: Option<&Value>) -> &[Value] {
    content
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn attr<'a>(node: &'a Value, key: &str) -> Option<&'a Value> {
    node.get("attrs").and_then(|a| a.get(key))
}

fn attr_str(node: &Value, key: &str) -> Option<String> {
    attr(node, key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn blocks(content: Option<&Value>, depth: usize) -> Vec<Block> {
    if depth > DEPTH {
        return Vec::new();
    }
    let mut out = Vec::new();
    for node in children(content) {
        block(node, depth, &mut out);
    }
    out
}

fn block(node: &Value, depth: usize, out: &mut Vec<Block>) {
    let kind = node.get("type").and_then(Value::as_str).unwrap_or_default();
    let content = node.get("content");
    let d = depth + 1;
    match kind {
        "paragraph" => {
            let content = inlines(content, d);
            if !content.is_empty() {
                out.push(Block::Paragraph { content });
            }
        }
        "heading" => out.push(Block::Heading {
            level: attr(node, "level")
                .and_then(Value::as_u64)
                .map_or(2, |l| l.clamp(1, 6) as u8),
            content: inlines(content, d),
        }),
        "bulletList" | "orderedList" => out.push(Block::List {
            ordered: kind == "orderedList",
            start: attr(node, "order")
                .and_then(Value::as_u64)
                .map_or(1, |n| n.min(u32::MAX as u64) as u32),
            items: children(content)
                .iter()
                .map(|item| Item {
                    checked: None,
                    content: blocks(item.get("content"), d),
                })
                .collect(),
        }),
        // A checklist's items hold inline content directly, not paragraphs.
        "taskList" | "decisionList" => out.push(Block::List {
            ordered: false,
            start: 1,
            items: children(content)
                .iter()
                .map(|item| Item {
                    checked: (kind == "taskList")
                        .then(|| attr_str(item, "state").as_deref() == Some("DONE")),
                    content: vec![Block::Paragraph {
                        content: inlines(item.get("content"), d),
                    }],
                })
                .collect(),
        }),
        "codeBlock" => out.push(Block::Code {
            language: attr_str(node, "language"),
            text: children(content)
                .iter()
                .filter_map(|t| t.get("text").and_then(Value::as_str))
                .collect(),
        }),
        "blockquote" => out.push(Block::Quote {
            content: blocks(content, d),
        }),
        "panel" => out.push(Block::Panel {
            kind: attr_str(node, "panelType").unwrap_or_else(|| "info".into()),
            content: blocks(content, d),
        }),
        "expand" | "nestedExpand" => out.push(Block::Expand {
            title: attr_str(node, "title").unwrap_or_default(),
            content: blocks(content, d),
        }),
        "rule" => out.push(Block::Rule),
        "table" => out.push(Block::Table {
            rows: children(content)
                .iter()
                .map(|row| {
                    children(row.get("content"))
                        .iter()
                        .map(|cell| Cell {
                            header: cell.get("type").and_then(Value::as_str) == Some("tableHeader"),
                            content: blocks(cell.get("content"), d),
                        })
                        .collect()
                })
                .collect(),
        }),
        "mediaSingle" | "mediaGroup" => {
            for media in children(content) {
                out.push(Block::Attachment {
                    name: attr_str(media, "alt")
                        .or_else(|| attr_str(media, "name"))
                        .unwrap_or_else(|| "attachment".into()),
                });
            }
        }
        "media" => out.push(Block::Attachment {
            name: attr_str(node, "alt").unwrap_or_else(|| "attachment".into()),
        }),
        "blockCard" | "embedCard" => {
            if let Some(url) = attr_str(node, "url").and_then(|u| safe(&u)) {
                out.push(Block::Card { url });
            }
        }
        // Something this does not know. Its blocks if it has any, else its
        // text as a paragraph, else nothing -- never an error.
        _ => {
            let inner = blocks(content, d);
            if !inner.is_empty() {
                out.extend(inner);
            } else {
                let content = inlines(content, d);
                if !content.is_empty() {
                    out.push(Block::Paragraph { content });
                }
            }
        }
    }
}

fn inlines(content: Option<&Value>, depth: usize) -> Vec<Inline> {
    if depth > DEPTH {
        return Vec::new();
    }
    let mut out = Vec::new();
    for node in children(content) {
        let kind = node.get("type").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "text" => {
                let text = node.get("text").and_then(Value::as_str).unwrap_or_default();
                if text.is_empty() {
                    continue;
                }
                let mut run = Inline::Text {
                    text: text.to_string(),
                    bold: false,
                    italic: false,
                    code: false,
                    strike: false,
                    underline: false,
                    href: None,
                };
                if let Inline::Text {
                    bold,
                    italic,
                    code,
                    strike,
                    underline,
                    href,
                    ..
                } = &mut run
                {
                    for mark in children(node.get("marks")) {
                        match mark.get("type").and_then(Value::as_str) {
                            Some("strong") => *bold = true,
                            Some("em") => *italic = true,
                            Some("code") => *code = true,
                            Some("strike") => *strike = true,
                            Some("underline") => *underline = true,
                            Some("link") => *href = attr_str(mark, "href").and_then(|u| safe(&u)),
                            _ => {}
                        }
                    }
                }
                out.push(run);
            }
            "hardBreak" => out.push(Inline::Break),
            "mention" => out.push(Inline::Mention {
                name: attr_str(node, "text")
                    .map(|t| t.trim_start_matches('@').to_string())
                    .unwrap_or_else(|| "someone".into()),
            }),
            "emoji" => out.push(Inline::Emoji {
                text: attr_str(node, "text")
                    .or_else(|| attr_str(node, "shortName"))
                    .unwrap_or_default(),
            }),
            "status" => out.push(Inline::Status {
                text: attr_str(node, "text").unwrap_or_default(),
                color: attr_str(node, "color").unwrap_or_else(|| "neutral".into()),
            }),
            "date" => {
                // A string of milliseconds, in practice; a number is accepted
                // too.
                let at = attr(node, "timestamp").and_then(|t| {
                    t.as_i64()
                        .or_else(|| t.as_str().and_then(|s| s.parse().ok()))
                });
                if let Some(at) = at {
                    out.push(Inline::Date { at });
                }
            }
            "inlineCard" => {
                if let Some(url) = attr_str(node, "url").and_then(|u| safe(&u)) {
                    out.push(Inline::Text {
                        text: url.clone(),
                        bold: false,
                        italic: false,
                        code: false,
                        strike: false,
                        underline: false,
                        href: Some(url),
                    });
                }
            }
            _ => {
                if let Some(text) = node.get("text").and_then(Value::as_str) {
                    out.push(text_run(text));
                } else {
                    out.extend(inlines(node.get("content"), depth + 1));
                }
            }
        }
    }
    out
}

/// A URL a link may point at. `javascript:` and `data:` are the reasons this
/// exists; anything that is not plainly a web or mail address goes with them.
fn safe(url: &str) -> Option<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:"))
        .then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(t: &str) -> Inline {
        text_run(t)
    }

    #[test]
    fn paragraphs_marks_and_links_come_through() {
        let doc = json!({ "type": "doc", "version": 1, "content": [
            { "type": "paragraph", "content": [
                { "type": "text", "text": "Run " },
                { "type": "text", "text": "cargo test", "marks": [{ "type": "code" }] },
                { "type": "text", "text": " and see ", "marks": [{ "type": "strong" }] },
                { "type": "text", "text": "the docs", "marks": [
                    { "type": "link", "attrs": { "href": "https://docs.rs/tokio" } }
                ] },
            ] },
        ] });
        let blocks = from_adf(&doc);
        let [Block::Paragraph { content }] = blocks.as_slice() else {
            panic!("one paragraph");
        };
        assert_eq!(content[0], text("Run "));
        assert!(matches!(&content[1], Inline::Text { code: true, .. }));
        assert!(matches!(
            &content[2],
            Inline::Text {
                bold: true,
                href: None,
                ..
            }
        ));
        assert!(
            matches!(&content[3], Inline::Text { href: Some(h), .. } if h == "https://docs.rs/tokio")
        );
    }

    /// The reason the tree exists rather than Jira's HTML: a link anyone can
    /// write into a comment must not be a script.
    #[test]
    fn a_link_that_is_not_a_web_address_is_only_text() {
        for href in [
            "javascript:alert(1)",
            "data:text/html,x",
            "JaVaScRiPt:x",
            "file:///etc",
        ] {
            let doc = json!({ "type": "doc", "content": [{ "type": "paragraph", "content": [
                { "type": "text", "text": "click", "marks": [{ "type": "link", "attrs": { "href": href } }] }
            ] }] });
            let blocks = from_adf(&doc);
            let [Block::Paragraph { content }] = blocks.as_slice() else {
                panic!()
            };
            assert!(
                matches!(&content[0], Inline::Text { href: None, .. }),
                "{href}"
            );
        }
        let card = json!({ "type": "doc", "content": [
            { "type": "blockCard", "attrs": { "url": "javascript:x" } }
        ] });
        assert!(from_adf(&card).is_empty());
    }

    #[test]
    fn lists_checklists_code_and_tables() {
        let doc = json!({ "type": "doc", "content": [
            { "type": "orderedList", "attrs": { "order": 3 }, "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] },
            ] },
            { "type": "taskList", "content": [
                { "type": "taskItem", "attrs": { "state": "DONE" }, "content": [{ "type": "text", "text": "done" }] },
                { "type": "taskItem", "attrs": { "state": "TODO" }, "content": [{ "type": "text", "text": "not" }] },
            ] },
            { "type": "codeBlock", "attrs": { "language": "rust" }, "content": [{ "type": "text", "text": "fn main() {}" }] },
            { "type": "table", "content": [
                { "type": "tableRow", "content": [
                    { "type": "tableHeader", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "h" }] }] },
                    { "type": "tableCell", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "c" }] }] },
                ] },
            ] },
        ] });
        let blocks = from_adf(&doc);
        assert!(
            matches!(&blocks[0], Block::List { ordered: true, start: 3, items } if items.len() == 1)
        );
        let Block::List { items, .. } = &blocks[1] else {
            panic!()
        };
        assert_eq!(items[0].checked, Some(true));
        assert_eq!(items[1].checked, Some(false));
        assert_eq!(
            blocks[2],
            Block::Code {
                language: Some("rust".into()),
                text: "fn main() {}".into()
            }
        );
        let Block::Table { rows } = &blocks[3] else {
            panic!()
        };
        assert!(rows[0][0].header && !rows[0][1].header);
    }

    #[test]
    fn mentions_media_and_dates_are_named_not_fetched() {
        let doc = json!({ "type": "doc", "content": [
            { "type": "paragraph", "content": [
                { "type": "mention", "attrs": { "id": "x", "text": "@Ada Lovelace" } },
                { "type": "date", "attrs": { "timestamp": "1790812800000" } },
            ] },
            { "type": "mediaSingle", "content": [{ "type": "media", "attrs": { "alt": "screenshot.png", "id": "abc" } }] },
        ] });
        let blocks = from_adf(&doc);
        let Block::Paragraph { content } = &blocks[0] else {
            panic!()
        };
        assert_eq!(
            content[0],
            Inline::Mention {
                name: "Ada Lovelace".into()
            }
        );
        assert_eq!(
            content[1],
            Inline::Date {
                at: 1_790_812_800_000
            }
        );
        assert_eq!(
            blocks[1],
            Block::Attachment {
                name: "screenshot.png".into()
            }
        );
    }

    /// A node this has never heard of still reads, as its text.
    #[test]
    fn an_unknown_node_keeps_its_text() {
        let doc = json!({ "type": "doc", "content": [
            { "type": "somethingNew", "content": [{ "type": "text", "text": "still here" }] },
            { "type": "paragraph", "content": [{ "type": "newInline", "text": "and here" }] },
        ] });
        let blocks = from_adf(&doc);
        assert_eq!(
            blocks[0],
            Block::Paragraph {
                content: vec![text("still here")]
            }
        );
        assert_eq!(
            blocks[1],
            Block::Paragraph {
                content: vec![text("and here")]
            }
        );
    }

    #[test]
    fn a_document_nested_past_the_bound_is_cut_rather_than_followed() {
        let mut node =
            json!({ "type": "paragraph", "content": [{ "type": "text", "text": "deep" }] });
        for _ in 0..200 {
            node = json!({ "type": "blockquote", "content": [node] });
        }
        let doc = json!({ "type": "doc", "content": [node] });
        // Returns at all, which is the point; and the depth it reached is bounded.
        let mut blocks = from_adf(&doc);
        let mut depth = 0;
        while let Some(Block::Quote { content }) = blocks.first().cloned() {
            depth += 1;
            blocks = content;
        }
        assert!(depth <= DEPTH + 1, "{depth}");
    }

    #[test]
    fn plain_text_and_null_read_as_what_they_are() {
        assert_eq!(
            from_adf(&json!("one\ntwo\n\nthree")),
            vec![
                Block::Paragraph {
                    content: vec![text("one"), Inline::Break, text("two")]
                },
                Block::Paragraph {
                    content: vec![text("three")]
                },
            ]
        );
        assert!(from_adf(&Value::Null).is_empty());
    }
}
