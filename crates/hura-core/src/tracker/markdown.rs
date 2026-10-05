//! Rich text as something a person can edit: Markdown out, Jira's document
//! format back in.
//!
//! The window reads a description as a typed tree ([`super::doc`]) and draws
//! it; editing one needs text, and Markdown is the text people already write
//! tickets in. So a description or a comment is turned into Markdown to edit
//! and the Markdown into ADF to save.
//!
//! **The round trip is lossy, and says so.** Markdown has paragraphs,
//! headings, lists and checklists, code, quotes, rules, bold, italics,
//! strike-through, inline code and links -- and those survive both ways.
//! ADF also has tables, panels, collapsible sections, mentions, status
//! lozenges, dates and images, and Markdown has none of them: they become
//! their text, or nothing. [`Markdown::lost`] names each kind that was in the
//! original, so the editor can say what saving will flatten before anybody
//! presses save.
//!
//! The Markdown read back is a deliberately small dialect: `*` for emphasis
//! but not `_`, because `snake_case` in a ticket is a word and not the start
//! of italics; no raw HTML; no reference links. Anything it does not
//! recognise is text.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::doc::{Block, Inline};

/// A document as Markdown, and what of it Markdown could not hold.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Markdown {
    pub text: String,
    /// What saving this will flatten, in words: "tables", "mentions (kept as
    /// text)". Empty when the round trip is exact.
    pub lost: Vec<String>,
}

/// A document as Markdown.
pub fn to_markdown(blocks: &[Block]) -> Markdown {
    let mut lost = Lost::default();
    let text = render_blocks(blocks, &mut lost);
    Markdown { text, lost: lost.0 }
}

/// Markdown as an ADF document.
pub fn to_adf(markdown: &str) -> Value {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut ids = 0;
    json!({ "type": "doc", "version": 1, "content": parse_blocks(&lines, &mut ids) })
}

// ------------------------------------------------------------ to Markdown

/// Each kind of loss once, in the order it was met.
#[derive(Default)]
struct Lost(Vec<String>);

impl Lost {
    fn note(&mut self, what: &str) {
        if !self.0.iter().any(|w| w == what) {
            self.0.push(what.to_string());
        }
    }
}

fn render_blocks(blocks: &[Block], lost: &mut Lost) -> String {
    blocks
        .iter()
        .map(|b| render_block(b, lost))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_block(block: &Block, lost: &mut Lost) -> String {
    match block {
        Block::Paragraph { content } => guard_lines(&render_inlines(content, lost)),
        Block::Heading { level, content } => {
            format!(
                "{} {}",
                "#".repeat((*level).clamp(1, 6) as usize),
                render_inlines(content, lost)
            )
        }
        Block::List {
            ordered,
            start,
            items,
        } => items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let marker = match (ordered, item.checked) {
                    (_, Some(true)) => "- [x]".to_string(),
                    (_, Some(false)) => "- [ ]".to_string(),
                    (true, None) => format!("{}.", *start as usize + i),
                    (false, None) => "-".to_string(),
                };
                // An item's blocks on lines of their own, indented under the
                // marker: one newline between them rather than a blank line,
                // which keeps the list tight -- a second paragraph in an item
                // reads back as a line break within the first.
                let indent = " ".repeat(marker.len() + 1);
                let body = item
                    .content
                    .iter()
                    .map(|b| render_block(b, lost))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut lines = body.lines();
                let first = lines.next().unwrap_or_default();
                let mut out = format!("{marker} {first}");
                for line in lines {
                    out.push('\n');
                    if !line.is_empty() {
                        out.push_str(&indent);
                        out.push_str(line);
                    }
                }
                out.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Code { language, text } => {
            format!(
                "```{}\n{}\n```",
                language.as_deref().unwrap_or_default(),
                text.trim_end_matches('\n')
            )
        }
        Block::Quote { content } => render_blocks(content, lost)
            .lines()
            .map(|l| {
                if l.is_empty() {
                    ">".to_string()
                } else {
                    format!("> {l}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Panel { content, .. } => {
            lost.note("panels (their text is kept)");
            render_blocks(content, lost)
        }
        Block::Expand { title, content } => {
            lost.note("collapsible sections (their text is kept)");
            let body = render_blocks(content, lost);
            if title.is_empty() {
                body
            } else {
                format!("**{}**\n\n{body}", escape(title))
            }
        }
        Block::Rule => "---".to_string(),
        Block::Table { rows } => {
            lost.note("tables (their text is kept, a row a line)");
            rows.iter()
                .map(|row| {
                    let cells: Vec<String> = row
                        .iter()
                        .map(|c| render_blocks(&c.content, lost).replace('\n', " "))
                        .collect();
                    guard_lines(&cells.join(" | "))
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        }
        Block::Attachment { .. } => {
            lost.note("images and attachments");
            String::new()
        }
        Block::Card { url } => format!("[{}]({})", escape(url), link_target(url)),
    }
}

fn render_inlines(content: &[Inline], lost: &mut Lost) -> String {
    let mut out = String::new();
    for inline in content {
        match inline {
            Inline::Text {
                text,
                bold,
                italic,
                code,
                strike,
                underline,
                href,
            } => {
                if *underline {
                    lost.note("underlining");
                }
                // Markdown's delimiters do not open or close against a space,
                // so the run's own leading and trailing spaces go outside them.
                let core = text.trim();
                if core.is_empty() {
                    out.push_str(text);
                    continue;
                }
                let lead = &text[..text.len() - text.trim_start().len()];
                let trail = &text[text.trim_end().len()..];
                let mut run = if *code { code_span(core) } else { escape(core) };
                if *strike {
                    run = format!("~~{run}~~");
                }
                if *italic {
                    run = format!("*{run}*");
                }
                if *bold {
                    run = format!("**{run}**");
                }
                if let Some(href) = href {
                    run = format!("[{run}]({})", link_target(href));
                }
                out.push_str(lead);
                out.push_str(&run);
                out.push_str(trail);
            }
            Inline::Mention { name } => {
                lost.note("mentions (kept as text)");
                out.push('@');
                out.push_str(&escape(name));
            }
            Inline::Emoji { text } => out.push_str(&escape(text)),
            Inline::Status { text, .. } => {
                lost.note("status lozenges (kept as text)");
                out.push_str(&escape(text));
            }
            Inline::Date { at } => {
                lost.note("dates (kept as text)");
                out.push_str(&iso_date(*at));
            }
            Inline::Break => out.push('\n'),
        }
    }
    out
}

/// Text that will read back as text: every character this dialect gives a
/// meaning, escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '`' | '[' | ']' | '~') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escape what would start a block at the head of a line of a paragraph: a
/// paragraph that happens to begin `# ` or `1. ` must not come back a heading
/// or a list.
fn guard_lines(text: &str) -> String {
    text.lines()
        .map(|line| {
            let t = line.trim_start();
            let starts_block = t.starts_with('#')
                || t.starts_with('>')
                || t.starts_with("- ")
                || t.starts_with("+ ")
                || t == "-"
                || t.starts_with("---")
                || t.starts_with("___");
            if starts_block {
                return format!("\\{t}");
            }
            // `12. ` -- escape the dot, since a backslash before a digit is a
            // backslash.
            let digits = t.chars().take_while(char::is_ascii_digit).count();
            if digits > 0 && t[digits..].starts_with(". ") {
                return format!("{}\\{}", &t[..digits], &t[digits..]);
            }
            t.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn code_span(text: &str) -> String {
    if text.contains('`') {
        format!("`` {text} ``")
    } else {
        format!("`{text}`")
    }
}

/// A link's target, with the one character that would end it early encoded.
fn link_target(url: &str) -> String {
    url.replace(' ', "%20")
        .replace('(', "%28")
        .replace(')', "%29")
}

/// Epoch milliseconds as `YYYY-MM-DD`, in UTC.
fn iso_date(ms: i64) -> String {
    // Howard Hinnant's days-to-civil.
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

// ------------------------------------------------------------- to ADF

fn parse_blocks(lines: &[&str], ids: &mut u32) -> Vec<Value> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        if t.is_empty() {
            i += 1;
            continue;
        }
        if let Some(lang) = t.strip_prefix("```") {
            let fence_indent = line.len() - t.len();
            let mut text = Vec::new();
            i += 1;
            while i < lines.len() && lines[i].trim() != "```" {
                let l = lines[i];
                let strip = l.len() - l.trim_start().len();
                text.push(&l[strip.min(fence_indent)..]);
                i += 1;
            }
            i += 1; // the closing fence, or the end
            let text = text.join("\n");
            let mut node = json!({ "type": "codeBlock", "content": [] });
            if !lang.trim().is_empty() {
                node["attrs"] = json!({ "language": lang.trim() });
            }
            if !text.is_empty() {
                node["content"] = json!([{ "type": "text", "text": text }]);
            }
            out.push(node);
            continue;
        }
        if let Some((level, rest)) = heading(t) {
            out.push(json!({
                "type": "heading",
                "attrs": { "level": level },
                "content": parse_inline(rest),
            }));
            i += 1;
            continue;
        }
        if is_rule(t) {
            out.push(json!({ "type": "rule" }));
            i += 1;
            continue;
        }
        if t.starts_with('>') {
            let mut inner = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let l = lines[i].trim_start()[1..].to_string();
                inner.push(l.strip_prefix(' ').map(str::to_string).unwrap_or(l));
                i += 1;
            }
            let inner: Vec<&str> = inner.iter().map(String::as_str).collect();
            let content = parse_blocks(&inner, ids);
            if !content.is_empty() {
                out.push(json!({ "type": "blockquote", "content": content }));
            }
            continue;
        }
        if marker(line).is_some() {
            let (node, next) = parse_list(lines, i, ids);
            out.push(node);
            i = next;
            continue;
        }
        // A paragraph: lines until a blank one or one that starts a block,
        // each line after the first a hard break.
        let mut content: Vec<Value> = Vec::new();
        let start = i;
        while i < lines.len() {
            let l = lines[i].trim();
            if l.is_empty() {
                break;
            }
            if i > start && starts_block(lines[i]) {
                break;
            }
            if i > start {
                content.push(json!({ "type": "hardBreak" }));
            }
            content.extend(parse_inline(l));
            i += 1;
        }
        out.push(json!({ "type": "paragraph", "content": content }));
    }
    out
}

fn starts_block(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```")
        || heading(t).is_some()
        || is_rule(t)
        || t.starts_with('>')
        || marker(line).is_some()
}

fn heading(t: &str) -> Option<(usize, &str)> {
    let hashes = t.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &t[hashes..];
    if rest.is_empty() {
        return Some((hashes, ""));
    }
    rest.strip_prefix(' ').map(|r| (hashes, r.trim()))
}

fn is_rule(t: &str) -> bool {
    let t = t.trim();
    t.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|c| t.chars().all(|x| x.to_string() == *c))
}

/// A list item's marker: its indent, whether it is numbered (and from what),
/// and where the item's text starts.
#[derive(Debug, Clone, Copy)]
struct Marker {
    indent: usize,
    ordered: Option<u32>,
    text_at: usize,
}

fn marker(line: &str) -> Option<Marker> {
    let indent = line.len() - line.trim_start().len();
    let t = &line[indent..];
    if let Some(rest) = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .or_else(|| t.strip_prefix("+ "))
    {
        return Some(Marker {
            indent,
            ordered: None,
            text_at: line.len() - rest.len(),
        });
    }
    if t == "-" || t == "*" || t == "+" {
        return Some(Marker {
            indent,
            ordered: None,
            text_at: line.len(),
        });
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits)
        && let Some(rest) = t[digits..].strip_prefix(". ")
    {
        return Some(Marker {
            indent,
            ordered: t[..digits].parse().ok(),
            text_at: line.len() - rest.len(),
        });
    }
    None
}

/// A run of items with the first one's indent and kind, from `start`; the
/// list, and the line after it.
fn parse_list(lines: &[&str], start: usize, ids: &mut u32) -> (Value, usize) {
    let first = marker(lines[start]).expect("a list starts at a marker");
    let ordered = first.ordered.is_some();
    let mut items: Vec<Vec<String>> = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            // A blank line ends the list unless the next line carries on with
            // another item or with something indented under this one.
            let next = lines.get(i + 1).copied().unwrap_or_default();
            let carries_on = marker(next)
                .is_some_and(|m| m.indent == first.indent && m.ordered.is_some() == ordered)
                || indent_of(next) > first.indent && !next.trim().is_empty();
            if !carries_on {
                break;
            }
            if let Some(item) = items.last_mut() {
                item.push(String::new())
            }
            i += 1;
            continue;
        }
        match marker(line) {
            Some(m) if m.indent == first.indent => {
                if m.ordered.is_some() != ordered {
                    break;
                }
                items.push(vec![line[m.text_at..].to_string()]);
            }
            Some(m) if m.indent < first.indent => break,
            _ if indent_of(line) <= first.indent && starts_block(line) => break,
            _ => {
                // Indented under the item, or a lazy continuation of its text.
                let strip = indent_of(line).min(first.text_at);
                let Some(item) = items.last_mut() else { break };
                item.push(line[strip..].to_string());
            }
        }
        i += 1;
    }

    // A checklist when every item says whether it is done.
    let checks: Vec<Option<bool>> = items
        .iter()
        .map(|item| {
            let head = item.first().map(String::as_str).unwrap_or_default();
            if head.starts_with("[ ] ") || head == "[ ]" {
                Some(false)
            } else if head.starts_with("[x] ") || head.starts_with("[X] ") || head == "[x]" {
                Some(true)
            } else {
                None
            }
        })
        .collect();
    if !ordered && !checks.is_empty() && checks.iter().all(Option::is_some) {
        *ids += 1;
        let list_id = format!("hura-list-{ids}");
        let content: Vec<Value> = items
            .iter()
            .zip(&checks)
            .map(|(item, done)| {
                *ids += 1;
                // A checklist item holds inline content only: its lines, as
                // one run with breaks.
                let mut text = item.clone();
                text[0] = text[0].get(3..).unwrap_or_default().trim_start().to_string();
                let mut content = Vec::new();
                for (n, l) in text.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).enumerate() {
                    if n > 0 {
                        content.push(json!({ "type": "hardBreak" }));
                    }
                    content.extend(parse_inline(l));
                }
                json!({
                    "type": "taskItem",
                    "attrs": { "localId": format!("hura-task-{ids}"), "state": if done.unwrap_or(false) { "DONE" } else { "TODO" } },
                    "content": content,
                })
            })
            .collect();
        return (
            json!({ "type": "taskList", "attrs": { "localId": list_id }, "content": content }),
            i,
        );
    }

    let content: Vec<Value> = items
        .iter()
        .map(|item| {
            let lines: Vec<&str> = item.iter().map(String::as_str).collect();
            let mut blocks = parse_blocks(&lines, ids);
            // An item opens with a paragraph or a code block; one that opens
            // with anything else gets an empty paragraph in front, which is
            // how Jira's own editor writes it.
            let opens = blocks
                .first()
                .and_then(|b| b.get("type"))
                .and_then(Value::as_str);
            if !matches!(opens, Some("paragraph" | "codeBlock")) {
                blocks.insert(0, json!({ "type": "paragraph", "content": [] }));
            }
            json!({ "type": "listItem", "content": blocks })
        })
        .collect();
    let mut node = json!({
        "type": if ordered { "orderedList" } else { "bulletList" },
        "content": content,
    });
    if let Some(n) = first.ordered.filter(|&n| n != 1) {
        node["attrs"] = json!({ "order": n });
    }
    (node, i)
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The marks a run of text carries, in the order ADF lists them.
#[derive(Debug, Clone, Default, PartialEq)]
struct Marks {
    link: Option<String>,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
}

impl Marks {
    fn to_json(&self) -> Vec<Value> {
        let mut out = Vec::new();
        if let Some(href) = &self.link {
            out.push(json!({ "type": "link", "attrs": { "href": href } }));
        }
        // Code takes no other mark but a link, in ADF's schema; a code span
        // inside bold text is code.
        if self.code {
            out.push(json!({ "type": "code" }));
            return out;
        }
        if self.bold {
            out.push(json!({ "type": "strong" }));
        }
        if self.italic {
            out.push(json!({ "type": "em" }));
        }
        if self.strike {
            out.push(json!({ "type": "strike" }));
        }
        out
    }
}

fn parse_inline(text: &str) -> Vec<Value> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs: Vec<(String, Marks)> = Vec::new();
    inline(&chars, &Marks::default(), &mut runs);

    // Neighbouring runs with the same marks are one text node.
    let mut merged: Vec<(String, Marks)> = Vec::new();
    for (t, m) in runs {
        if t.is_empty() {
            continue;
        }
        match merged.last_mut() {
            Some((last, lm)) if *lm == m => last.push_str(&t),
            _ => merged.push((t, m)),
        }
    }
    merged
        .into_iter()
        .map(|(t, m)| {
            let marks = m.to_json();
            if marks.is_empty() {
                json!({ "type": "text", "text": t })
            } else {
                json!({ "type": "text", "text": t, "marks": marks })
            }
        })
        .collect()
}

fn inline(s: &[char], marks: &Marks, out: &mut Vec<(String, Marks)>) {
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut Vec<(String, Marks)>| {
        if !plain.is_empty() {
            out.push((std::mem::take(plain), marks.clone()));
        }
    };
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c == '\\' && i + 1 < s.len() && s[i + 1].is_ascii_punctuation() {
            plain.push(s[i + 1]);
            i += 2;
            continue;
        }
        if c == '`' {
            let ticks = s[i..].iter().take_while(|&&c| c == '`').count();
            if let Some(end) = find_ticks(s, i + ticks, ticks) {
                flush(&mut plain, out);
                let mut code: String = s[i + ticks..end].iter().collect();
                if code.len() >= 2 && code.starts_with(' ') && code.ends_with(' ') {
                    code = code[1..code.len() - 1].to_string();
                }
                out.push((
                    code,
                    Marks {
                        code: true,
                        ..marks.clone()
                    },
                ));
                i = end + ticks;
                continue;
            }
            plain.extend(&s[i..i + ticks]);
            i += ticks;
            continue;
        }
        if let Some((delim, apply)) = delimiter(s, i)
            && let Some(end) = find_closing(s, i + delim.len(), delim)
        {
            flush(&mut plain, out);
            let mut inner = marks.clone();
            apply(&mut inner);
            inline(&s[i + delim.len()..end], &inner, out);
            i = end + delim.len();
            continue;
        }
        if c == '['
            && let Some((label_end, url, after)) = link(s, i)
        {
            flush(&mut plain, out);
            let inner = Marks {
                link: Some(url),
                ..marks.clone()
            };
            inline(&s[i + 1..label_end], &inner, out);
            i = after;
            continue;
        }
        plain.push(c);
        i += 1;
    }
    flush(&mut plain, out);
}

type Apply = fn(&mut Marks);

/// The emphasis delimiter opening at `i`, if one does: followed by something
/// that is not a space, as Markdown requires -- `2 * 3` is arithmetic.
fn delimiter(s: &[char], i: usize) -> Option<(&'static [char], Apply)> {
    const BOLD: &[char] = &['*', '*'];
    const STRIKE: &[char] = &['~', '~'];
    const ITALIC: &[char] = &['*'];
    let opens = |d: &[char]| {
        s[i..].starts_with(d) && s.get(i + d.len()).is_some_and(|c| !c.is_whitespace())
    };
    if opens(BOLD) {
        return Some((BOLD, |m| m.bold = true));
    }
    if opens(STRIKE) {
        return Some((STRIKE, |m| m.strike = true));
    }
    if opens(ITALIC) {
        return Some((ITALIC, |m| m.italic = true));
    }
    None
}

/// Where `delim` closes, from `from`: not escaped, not inside a code span,
/// after something that is not a space -- and for a single `*`, not half of a
/// `**`.
fn find_closing(s: &[char], from: usize, delim: &[char]) -> Option<usize> {
    let mut j = from;
    while j < s.len() {
        match s[j] {
            '\\' => {
                j += 2;
                continue;
            }
            '`' => {
                let ticks = s[j..].iter().take_while(|&&c| c == '`').count();
                j = find_ticks(s, j + ticks, ticks).map_or(j + ticks, |e| e + ticks);
                continue;
            }
            _ => {}
        }
        if j > from && s[j..].starts_with(delim) && !s[j - 1].is_whitespace() {
            if delim == ['*'] {
                // `**` inside italics opens bold; skip over it whole.
                if s.get(j + 1) == Some(&'*')
                    && let Some(end) = find_closing(s, j + 2, &['*', '*'])
                {
                    j = end + 2;
                    continue;
                }
            }
            return Some(j);
        }
        j += 1;
    }
    None
}

fn find_ticks(s: &[char], from: usize, ticks: usize) -> Option<usize> {
    let mut j = from;
    while j < s.len() {
        if s[j] == '`' {
            let run = s[j..].iter().take_while(|&&c| c == '`').count();
            if run == ticks {
                return Some(j);
            }
            j += run;
        } else {
            j += 1;
        }
    }
    None
}

/// `[label](url)` at `i`: where the label ends, the url, and the index after
/// the closing parenthesis. Only a web or mail address is a link, the same
/// rule the reader applies -- anything else stays text.
fn link(s: &[char], i: usize) -> Option<(usize, String, usize)> {
    let mut depth = 0;
    let mut j = i;
    let label_end = loop {
        match s.get(j)? {
            '\\' => j += 1,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break j;
                }
            }
            _ => {}
        }
        j += 1;
    };
    if s.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let close = s[label_end + 2..].iter().position(|&c| c == ')')? + label_end + 2;
    let url: String = s[label_end + 2..close].iter().collect();
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let safe = lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:");
    (safe && label_end > i + 1).then(|| (label_end, url.to_string(), close + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::doc::from_adf;

    /// What goes out as Markdown and comes back as ADF reads back as the same
    /// document -- for everything Markdown can say.
    fn round_trip(md: &str) -> String {
        to_markdown(&from_adf(&to_adf(md))).text
    }

    #[test]
    fn what_markdown_can_say_survives_the_round_trip() {
        for md in [
            "plain words",
            "# A heading\n\nand a paragraph",
            "**bold**, *italic*, ~~gone~~ and `code`",
            "a [link](https://example.com/x) in text",
            "line one\nline two",
            "- one\n- two\n  - nested\n- three",
            "3. three\n4. four",
            "- [ ] to do\n- [x] done",
            "```rust\nfn main() {}\n```",
            "> quoted\n>\n> twice",
            "above\n\n---\n\nbelow",
            "snake_case stays a word",
            "a literal \\* and \\[brackets\\] and a \\`tick",
        ] {
            assert_eq!(round_trip(md), md, "{md}");
        }
    }

    /// A paragraph that happens to start like a block is still a paragraph
    /// after the trip.
    #[test]
    fn text_that_looks_like_markdown_is_escaped_and_stays_text() {
        let blocks = vec![Block::Paragraph {
            content: vec![Inline::Text {
                text: "# not a heading".into(),
                bold: false,
                italic: false,
                code: false,
                strike: false,
                underline: false,
                href: None,
            }],
        }];
        let md = to_markdown(&blocks).text;
        assert_eq!(md, "\\# not a heading");
        assert_eq!(from_adf(&to_adf(&md)), blocks);

        for line in [
            "1. not a list",
            "- not a list",
            "> not a quote",
            "2 * 3 * 4",
        ] {
            assert_eq!(
                to_adf(
                    &to_markdown(&from_adf(&json!({ "type": "doc", "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": line }] }
                ] })))
                    .text
                )["content"][0]["type"],
                "paragraph",
                "{line}"
            );
        }
    }

    /// What Markdown cannot hold is named, so the editor can say so before
    /// saving.
    #[test]
    fn what_cannot_be_kept_is_named() {
        let adf = json!({ "type": "doc", "content": [
            { "type": "table", "content": [{ "type": "tableRow", "content": [
                { "type": "tableCell", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "a" }] }] },
                { "type": "tableCell", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "b" }] }] }
            ] }] },
            { "type": "paragraph", "content": [
                { "type": "text", "text": "hi " },
                { "type": "mention", "attrs": { "id": "x", "text": "@Ada" } }
            ] },
            { "type": "mediaSingle", "content": [{ "type": "media", "attrs": { "alt": "shot.png" } }] }
        ] });
        let md = to_markdown(&from_adf(&adf));
        assert_eq!(md.text, "a | b\n\nhi @Ada");
        assert_eq!(md.lost.len(), 3, "{:?}", md.lost);
        assert!(md.lost[0].starts_with("tables"));
        assert!(md.lost[1].starts_with("mentions"));
        assert!(md.lost[2].starts_with("images"));

        assert!(
            to_markdown(&from_adf(&to_adf("just **text**")))
                .lost
                .is_empty()
        );
    }

    #[test]
    fn the_adf_written_is_what_jira_expects() {
        let adf = to_adf("**[bold link](https://x.io)** and `a`");
        let content = &adf["content"][0]["content"];
        assert_eq!(content[0]["text"], "bold link");
        assert_eq!(content[0]["marks"][0]["type"], "link");
        assert_eq!(content[0]["marks"][1]["type"], "strong");
        assert_eq!(content[2]["marks"], json!([{ "type": "code" }]));
        assert_eq!(adf["version"], 1);

        // A link to anything but the web or mail is text.
        let adf = to_adf("[x](javascript:alert(1))");
        assert!(adf["content"][0]["content"][0].get("marks").is_none());

        // A checklist's items carry ids and a state.
        let adf = to_adf("- [x] done");
        let item = &adf["content"][0]["content"][0];
        assert_eq!(item["type"], "taskItem");
        assert_eq!(item["attrs"]["state"], "DONE");

        // An empty document is one with no content, not one empty paragraph.
        assert_eq!(to_adf("  \n\n")["content"], json!([]));
    }

    #[test]
    fn a_date_is_written_as_a_day() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(1_791_331_200_000), "2026-10-07");
    }
}
