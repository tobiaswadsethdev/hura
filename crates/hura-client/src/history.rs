//! Every conversation this machine has seen, kept on it, and searching them.
//!
//! **A copy, kept by the client, of what passed through it.** The transcript
//! is the host's, inside the sandbox, and it goes with the sandbox: a removed
//! session's conversations are nowhere after that, and an ended one's are
//! forgotten by the host when it ends. The window already has every entry in
//! hand as it arrives -- its chat cache is built from them -- so this keeps the
//! same entries on disk as they pass, and reads them back to search. Indexing
//! on the server instead was the other choice; it would see conversations no
//! window watched, but it would also be one more thing `hurad` stores about a
//! sandbox outside it, which is the arrangement [`hura_core::store`] exists to
//! keep disposable. What a window never watched is caught up by asking the
//! host for what came after the last entry kept -- see `history_catch_up` in
//! the desktop's `main.rs` -- which it does before a session is removed or a
//! conversation ended, so that both are still here afterwards.
//!
//! **One file of entries per conversation, appended to.** Under
//! `history/<server>/<session>/<conv>.jsonl` in the state directory, private
//! like everything else there: a transcript is the code, and whatever the agent
//! read. Pictures are taken out before an entry is written, and a tool's result
//! cut to its start, since between them they are most of a transcript's size
//! and none of what anybody searches for. A session that
//! is removed has its directory renamed `<session>@<when>`, and a conversation
//! that ends, or whose host started its transcript again, its file -- so a
//! later one under the same name is a different one here too.
//!
//! **Searched by reading, not by an index file.** What is searched is a few
//! lines of text per entry -- what was said, the tool used and the files it
//! touched -- and that is kept in memory once read, each file re-read from
//! where it was last read to when it grows. Thousands of conversations are a
//! few megabytes of that, which a linear scan answers faster than a person
//! types. An inverted index would be a second copy of the same text to keep
//! consistent with the first, for a speed nobody would notice.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead as _, Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};

use hura_core::chat::{ChatEntry, ChatEvent};
use hura_core::state;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How many results one search answers with, newest first.
pub const LIMIT: usize = 200;

/// What a session was called, kept beside its conversations because the
/// session list will not say once it is gone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub title: Option<String>,
    pub project: Option<String>,
    /// Epoch milliseconds, once it was removed.
    #[serde(default)]
    pub removed: Option<u64>,
}

/// One entry that matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hit {
    pub server: String,
    /// The session's name, which a removed one shares with whatever was made
    /// later under it.
    pub session: String,
    /// Which of the sessions by that name: what [`History::transcript`] takes.
    pub session_key: String,
    pub title: Option<String>,
    pub project: Option<String>,
    /// The session has been removed, so the conversation is only here.
    pub removed: bool,
    pub conv: String,
    pub conv_key: String,
    /// The conversation has ended, or been started again, and is only here.
    pub ended: bool,
    pub seq: u64,
    pub at: u64,
    pub role: Role,
    /// The entry's text around the first thing searched for, on one line.
    pub snippet: String,
    pub tools: Vec<String>,
    pub files: Vec<String>,
}

/// The answer to a search.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Found {
    pub hits: Vec<Hit>,
    /// There were more than [`LIMIT`].
    pub more: bool,
}

/// Who an entry is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    You,
    Claude,
    /// A subagent, inside a call of Claude's.
    Agent,
    /// A `!` command.
    Shell,
    Error,
}

/// What of an entry is searched.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Doc {
    seq: u64,
    at: u64,
    role: Role,
    text: String,
    tools: Vec<String>,
    files: Vec<String>,
    /// Everything above, lower-cased, which is what a term is looked for in.
    haystack: String,
}

/// The searchable part of an entry, or `None` for one with nothing in it a
/// person would look for: a read receipt, a tool's result, the end of a turn.
///
/// A tool's result is left out on purpose. It is the file read or the command's
/// output, so every word in the repository would match in the conversation
/// that happened to read it; the call itself says which file, and that is
/// what is searched.
fn doc(entry: &ChatEntry) -> Option<Doc> {
    let (role, text, tools, files) = match &entry.ev {
        ChatEvent::Prompt { text, .. } => (Role::You, text.clone(), vec![], vec![]),
        ChatEvent::Shell { command, .. } => {
            (Role::Shell, command.clone(), vec!["shell".into()], vec![])
        }
        ChatEvent::Error { message } => (Role::Error, message.clone(), vec![], vec![]),
        ChatEvent::Sdk { msg } => assistant(msg)?,
        _ => return None,
    };
    if text.trim().is_empty() && tools.is_empty() {
        return None;
    }
    let haystack = [text.as_str(), &tools.join(" "), &files.join(" ")]
        .join("\n")
        .to_lowercase();
    Some(Doc {
        seq: entry.seq,
        at: entry.at,
        role,
        text,
        tools,
        files,
        haystack,
    })
}

/// What Claude said and did in one assistant message: its text, and for each
/// tool call the tool's name, the files it names and a line saying what it was
/// asked -- `cargo test`, the pattern, the URL.
fn assistant(msg: &Value) -> Option<(Role, String, Vec<String>, Vec<String>)> {
    if msg.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let role = if msg.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
        Role::Agent
    } else {
        Role::Claude
    };
    let mut text = Vec::new();
    let mut tools = Vec::new();
    let mut files = Vec::new();
    for block in msg.pointer("/message/content")?.as_array()? {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    text.push(t.to_string());
                }
            }
            Some("tool_use") => {
                let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                if !tools.iter().any(|t| t == name) {
                    tools.push(name.to_string());
                }
                let input = block.get("input");
                let field = |k: &str| input.and_then(|i| i.get(k)).and_then(Value::as_str);
                for k in ["file_path", "notebook_path", "path"] {
                    if let Some(p) = field(k)
                        && !files.iter().any(|f| f == p)
                    {
                        files.push(p.to_string());
                    }
                }
                // Every one it has: a command is remembered by what it ran and
                // by what it was for, "Run the search tests".
                let said: Vec<&str> =
                    ["command", "pattern", "url", "query", "description", "skill"]
                        .into_iter()
                        .filter_map(field)
                        .collect();
                if !said.is_empty() {
                    text.push(format!("{name}: {}", said.join(" · ")));
                }
            }
            _ => {}
        }
    }
    Some((role, text.join("\n"), tools, files))
}

/// How much of a tool's result is kept: the start of a file the agent read,
/// or of a command's output, which is enough to read the conversation by and
/// none of what is searched.
const RESULT_CHARS: usize = 4_000;

/// An entry as it is kept: a picture's bytes taken out -- an image block's
/// `source.data`, wherever in an SDK message it is, and a sent file's `data` --
/// and a tool's result cut to its start.
fn trimmed(entry: &ChatEntry) -> ChatEntry {
    fn cut(text: &mut String) {
        if let Some((at, _)) = text.char_indices().nth(RESULT_CHARS) {
            text.truncate(at);
            text.push_str("\n…");
        }
    }
    fn strip(v: &mut Value, in_result: bool) {
        match v {
            Value::Object(map) => {
                let kind = map.get("type").and_then(Value::as_str).map(str::to_string);
                let kind = kind.as_deref();
                if kind == Some("image")
                    && let Some(Value::Object(source)) = map.get_mut("source")
                {
                    source.remove("data");
                }
                let in_result = in_result || kind == Some("tool_result");
                for (k, v) in map.iter_mut() {
                    match v {
                        Value::String(text) if in_result && (k == "content" || k == "text") => {
                            cut(text)
                        }
                        _ => strip(v, in_result),
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|v| strip(v, in_result)),
            _ => {}
        }
    }
    let mut entry = entry.clone();
    match &mut entry.ev {
        ChatEvent::Sdk { msg } => {
            // `tool_use_result` is the same result again, as Claude Code
            // structured it for itself.
            if let Some(m) = msg.as_object_mut() {
                m.remove("tool_use_result");
            }
            strip(msg, false)
        }
        ChatEvent::Prompt { files, .. } => files.iter_mut().for_each(|f| f.data = None),
        _ => {}
    }
    entry
}

/// A query: words that must all be there, and `file:` and `tool:` to say
/// where. A phrase in double quotes is one word.
#[derive(Debug, Default, PartialEq, Eq)]
struct Query {
    words: Vec<String>,
    files: Vec<String>,
    tools: Vec<String>,
}

impl Query {
    fn parse(text: &str) -> Query {
        let mut q = Query::default();
        let mut chars = text.chars().peekable();
        loop {
            while chars.next_if(|c| c.is_whitespace()).is_some() {}
            if chars.peek().is_none() {
                break;
            }
            let mut word = String::new();
            let mut quoted = false;
            while let Some(&c) = chars.peek() {
                if c == '"' {
                    quoted = !quoted;
                } else if c.is_whitespace() && !quoted {
                    break;
                } else {
                    word.push(c);
                }
                chars.next();
            }
            let word = word.to_lowercase();
            if let Some(f) = word.strip_prefix("file:").filter(|f| !f.is_empty()) {
                q.files.push(f.to_string());
            } else if let Some(t) = word.strip_prefix("tool:").filter(|t| !t.is_empty()) {
                q.tools.push(t.to_string());
            } else if !word.is_empty() {
                q.words.push(word);
            }
        }
        q
    }

    fn is_empty(&self) -> bool {
        self.words.is_empty() && self.files.is_empty() && self.tools.is_empty()
    }

    fn matches(&self, d: &Doc) -> bool {
        let has =
            |list: &[String], want: &str| list.iter().any(|x| x.to_lowercase().contains(want));
        self.words.iter().all(|w| d.haystack.contains(w.as_str()))
            && self.files.iter().all(|f| has(&d.files, f))
            && self.tools.iter().all(|t| has(&d.tools, t))
    }
}

/// The text around the first word looked for, on one line, cut at about a
/// line's length.
fn snippet(text: &str, words: &[String]) -> String {
    const BEFORE: usize = 50;
    const LENGTH: usize = 180;
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = flat.chars().collect();
    let lower: Vec<char> = flat.to_lowercase().chars().collect();
    // Lower-casing can change how many characters there are -- `İ` is two --
    // and then a position in one is not one in the other; the start will do.
    let at = if lower.len() == chars.len() {
        words
            .iter()
            .filter_map(|w| {
                let w: Vec<char> = w.chars().collect();
                lower
                    .windows(w.len().max(1))
                    .position(|win| win == w.as_slice())
            })
            .min()
            .unwrap_or(0)
    } else {
        0
    };
    let start = at.saturating_sub(BEFORE);
    let end = (start + LENGTH).min(chars.len());
    let mut out: String = chars[start..end].iter().collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// A name made safe to be a file's: what a path may not hold, and `@`, which
/// says when something was put away, are written as `%xx`.
fn safe(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.') && !(out.is_empty() && b == b'.')
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02x}"));
        }
    }
    out
}

fn unsafe_name(file: &str) -> String {
    let bytes = file.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = file
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A name with when it was put away taken off: `chat-2@1731400000000` is
/// `chat-2`, put away.
fn kept_name(key: &str) -> (String, bool) {
    match key.split_once('@') {
        Some((name, _)) => (unsafe_name(name), true),
        None => (unsafe_name(key), false),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// What has been read of one file: how far, and what it said.
#[derive(Default)]
struct Read {
    offset: u64,
    /// The last entry written, which is where a host is asked from.
    last: u64,
    /// When each entry was, by seq, in order: what tells one transcript's
    /// entry 1 from another's.
    stamps: Vec<(u64, u64)>,
    docs: Vec<Doc>,
}

impl Read {
    fn at(&self, seq: u64) -> Option<u64> {
        let i = self.stamps.binary_search_by_key(&seq, |&(s, _)| s).ok()?;
        Some(self.stamps[i].1)
    }
}

/// Every conversation kept on this machine.
pub struct History {
    dir: PathBuf,
    read: HashMap<PathBuf, Read>,
    metas: HashMap<PathBuf, Meta>,
}

impl History {
    /// The one in the state directory.
    pub fn open() -> History {
        History::at(state::dir().join("history"))
    }

    pub fn at(dir: PathBuf) -> History {
        History {
            dir,
            read: HashMap::new(),
            metas: HashMap::new(),
        }
    }

    fn session_dir(&self, server: &str, session: &str) -> PathBuf {
        self.dir.join(safe(server)).join(safe(session))
    }

    fn conv_file(&self, server: &str, session: &str, conv: &str) -> PathBuf {
        self.session_dir(server, session)
            .join(format!("{}.jsonl", safe(conv)))
    }

    /// Bring what is known of one file up to what is in it.
    fn refresh(&mut self, path: &Path) -> io::Result<&Read> {
        let len = match fs::metadata(path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e),
        };
        let read = self.read.entry(path.to_path_buf()).or_default();
        if len < read.offset {
            *read = Read::default();
        }
        if len > read.offset {
            let mut file = fs::File::open(path)?;
            file.seek(io::SeekFrom::Start(read.offset))?;
            let mut text = String::new();
            file.take(len - read.offset).read_to_string(&mut text)?;
            // Only whole lines: one being written as this reads is read once
            // it is finished.
            let whole = text.rfind('\n').map_or(0, |i| i + 1);
            for line in text[..whole].lines() {
                // A line that does not parse is skipped rather than ending the
                // file: an entry kind a newer window wrote, say.
                if let Ok(entry) = serde_json::from_str::<ChatEntry>(line) {
                    read.last = read.last.max(entry.seq);
                    read.stamps.push((entry.seq, entry.at));
                    read.docs.extend(doc(&entry));
                }
            }
            read.offset += whole as u64;
        }
        Ok(read)
    }

    /// The last entry kept of a conversation, which is what to ask its host
    /// for what came after. `0` for one never seen.
    pub fn last(&mut self, server: &str, session: &str, conv: &str) -> u64 {
        let path = self.conv_file(server, session, conv);
        self.refresh(&path).map(|r| r.last).unwrap_or(0)
    }

    /// A replay from a host: what it has after `since`, the entry it was
    /// asked from, in frames of which `seq` is each one's last.
    ///
    /// A host whose transcript started again is put away as an ended
    /// conversation first, and is told two ways. It has less than it was
    /// asked from, which is how a pane tells; or an entry it sends again is
    /// not the one kept under that number, which is how a replay from the
    /// start tells -- a window opened after the host started again asks from
    /// nothing, and every entry it is sent is then one already "kept".
    pub fn replay(
        &mut self,
        (server, session, conv): (&str, &str, &str),
        since: u64,
        seq: u64,
        entries: &[ChatEntry],
    ) -> io::Result<()> {
        let path = self.conv_file(server, session, conv);
        let read = self.refresh(&path)?;
        let another = entries
            .iter()
            .filter(|e| e.seq <= read.last)
            .any(|e| read.at(e.seq).is_some_and(|at| at != e.at));
        if seq < since || another {
            self.ended(server, session, conv)?;
        }
        self.keep(server, session, conv, entries)
    }

    /// Entries, as they arrive. Any already kept are skipped, which is what
    /// makes a replay that overlaps, or two windows' worth of the same entry,
    /// harmless.
    pub fn keep(
        &mut self,
        server: &str,
        session: &str,
        conv: &str,
        entries: &[ChatEntry],
    ) -> io::Result<()> {
        let last = self.last(server, session, conv);
        let mut lines = String::new();
        let mut newest = last;
        for e in entries {
            if e.seq > newest {
                newest = e.seq;
                lines.push_str(&serde_json::to_string(&trimmed(e)).map_err(io::Error::other)?);
                lines.push('\n');
            }
        }
        if lines.is_empty() {
            return Ok(());
        }
        let path = self.conv_file(server, session, conv);
        if let Some(parent) = path.parent() {
            state::private_dir(&self.dir)?;
            state::private_dir(parent)?;
        }
        let mut file = append_private(&path)?;
        file.write_all(lines.as_bytes())
    }

    /// What a session is called now, kept for when it is gone. Written only
    /// when it changed, and only for a session with something kept.
    pub fn name(
        &mut self,
        server: &str,
        session: &str,
        title: Option<&str>,
        project: Option<&str>,
    ) -> io::Result<()> {
        let dir = self.session_dir(server, session);
        let meta = Meta {
            title: title.map(str::to_string),
            project: project.map(str::to_string),
            removed: None,
        };
        if self.metas.get(&dir) == Some(&meta) || !dir.is_dir() {
            return Ok(());
        }
        state::write_private(
            &dir.join("meta.json"),
            &serde_json::to_string(&meta).map_err(io::Error::other)?,
        )?;
        self.metas.insert(dir, meta);
        Ok(())
    }

    /// A conversation that has ended. Put away under the time it ended, so the
    /// next one by its name starts with nothing.
    pub fn ended(&mut self, server: &str, session: &str, conv: &str) -> io::Result<()> {
        let path = self.conv_file(server, session, conv);
        if !path.exists() {
            return Ok(());
        }
        let to = path.with_file_name(format!("{}@{}.jsonl", safe(conv), now()));
        fs::rename(&path, &to)?;
        self.read.remove(&path);
        Ok(())
    }

    /// A session that has been removed: its conversations put away together,
    /// with when.
    pub fn removed(&mut self, server: &str, session: &str) -> io::Result<()> {
        let dir = self.session_dir(server, session);
        if !dir.is_dir() {
            return Ok(());
        }
        let when = now();
        let mut meta = self.meta(&dir);
        meta.removed = Some(when);
        state::write_private(
            &dir.join("meta.json"),
            &serde_json::to_string(&meta).map_err(io::Error::other)?,
        )?;
        let to = dir.with_file_name(format!("{}@{when}", safe(session)));
        fs::rename(&dir, &to)?;
        self.metas.remove(&dir);
        self.read.retain(|p, _| !p.starts_with(&dir));
        Ok(())
    }

    fn meta(&mut self, dir: &Path) -> Meta {
        if let Some(m) = self.metas.get(dir) {
            return m.clone();
        }
        let m: Meta = fs::read_to_string(dir.join("meta.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        self.metas.insert(dir.to_path_buf(), m.clone());
        m
    }

    /// Everything kept of one conversation, for reading it once it exists
    /// nowhere else.
    pub fn transcript(
        &self,
        server: &str,
        session_key: &str,
        conv_key: &str,
    ) -> io::Result<Vec<ChatEntry>> {
        // Keys are names this answered with, made safe again; nothing in one
        // can step outside the directory.
        let path = self
            .dir
            .join(safe(server))
            .join(safe_key(session_key))
            .join(format!("{}.jsonl", safe_key(conv_key)));
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        Ok(io::BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect())
    }

    /// Read everything kept, so the first search does not: the window's frames
    /// wait on this while it holds the lock, and they would rather wait at
    /// start than in the middle of a conversation.
    pub fn warm(&mut self) {
        let _ = self.search("\u{0}");
    }

    /// Every entry that matches, newest first.
    pub fn search(&mut self, text: &str) -> io::Result<Found> {
        let query = Query::parse(text);
        if query.is_empty() {
            return Ok(Found::default());
        }
        let mut hits = Vec::new();
        for server_dir in dirs(&self.dir)? {
            let server = unsafe_name(&file_name(&server_dir));
            for session_dir in dirs(&server_dir)? {
                let session_key = file_name(&session_dir);
                let (session, _) = kept_name(&session_key);
                let meta = self.meta(&session_dir);
                for path in fs::read_dir(&session_dir)?.flatten().map(|e| e.path()) {
                    if path.extension().is_none_or(|x| x != "jsonl") {
                        continue;
                    }
                    let conv_key = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let (conv, ended) = kept_name(&conv_key);
                    for d in self
                        .refresh(&path)?
                        .docs
                        .iter()
                        .filter(|d| query.matches(d))
                    {
                        hits.push(Hit {
                            server: server.clone(),
                            session: session.clone(),
                            session_key: session_key.clone(),
                            title: meta.title.clone(),
                            project: meta.project.clone(),
                            removed: meta.removed.is_some(),
                            conv: conv.clone(),
                            conv_key: conv_key.clone(),
                            ended,
                            seq: d.seq,
                            at: d.at,
                            role: d.role,
                            snippet: snippet(&d.text, &query.words),
                            tools: d.tools.clone(),
                            files: d.files.clone(),
                        });
                    }
                }
            }
        }
        hits.sort_by(|a, b| b.at.cmp(&a.at).then(b.seq.cmp(&a.seq)));
        let more = hits.len() > LIMIT;
        hits.truncate(LIMIT);
        Ok(Found { hits, more })
    }
}

/// A key this module made, made safe again: `name@when` is the name made safe
/// and the time, which is digits.
fn safe_key(key: &str) -> String {
    match key.split_once('@') {
        Some((name, when)) if when.bytes().all(|b| b.is_ascii_digit()) => {
            format!("{}@{when}", safe(&unsafe_name(name)))
        }
        _ => safe(&unsafe_name(key)),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn dirs(path: &Path) -> io::Result<Vec<PathBuf>> {
    match fs::read_dir(path) {
        Ok(read) => Ok(read
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn append_private(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)
}

#[cfg(windows)]
fn append_private(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new().append(true).create(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch() -> History {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "hura-history-test-{}-{n}-{}",
            std::process::id(),
            now()
        ));
        let _ = fs::remove_dir_all(&dir);
        History::at(dir)
    }

    fn entry(seq: u64, ev: ChatEvent) -> ChatEntry {
        ChatEntry {
            seq,
            at: 1_000 + seq,
            ev,
        }
    }

    fn prompt(seq: u64, text: &str) -> ChatEntry {
        entry(
            seq,
            ChatEvent::Prompt {
                text: text.into(),
                uuid: format!("u{seq}"),
                files: vec![],
                receipt: false,
            },
        )
    }

    fn sdk(seq: u64, msg: Value) -> ChatEntry {
        entry(seq, ChatEvent::Sdk { msg })
    }

    fn edit(seq: u64, path: &str) -> ChatEntry {
        sdk(
            seq,
            json!({"type": "assistant", "parent_tool_use_id": null, "message": {"content": [
                {"type": "text", "text": "Fixing the token refresh."},
                {"type": "tool_use", "id": "t1", "name": "Edit", "input": {"file_path": path, "old_string": "a", "new_string": "b"}}
            ]}}),
        )
    }

    fn found(h: &mut History, q: &str) -> Vec<(String, u64)> {
        h.search(q)
            .unwrap()
            .hits
            .into_iter()
            .map(|h| (h.conv, h.seq))
            .collect()
    }

    #[test]
    fn words_are_all_looked_for_whatever_their_case() {
        let mut h = scratch();
        h.keep(
            "wsl",
            "s",
            "agent",
            &[
                prompt(1, "Make the Login test pass"),
                prompt(2, "the login page"),
            ],
        )
        .unwrap();
        assert_eq!(found(&mut h, "login TEST"), vec![("agent".into(), 1)]);
        assert_eq!(found(&mut h, "\"login page\""), vec![("agent".into(), 2)]);
        assert_eq!(found(&mut h, "login").len(), 2);
        assert!(found(&mut h, "").is_empty());
    }

    #[test]
    fn a_tool_call_is_found_by_its_tool_and_its_file() {
        let mut h = scratch();
        h.keep(
            "wsl",
            "s",
            "agent",
            &[prompt(1, "fix auth"), edit(2, "/sandbox/repo/src/auth.rs")],
        )
        .unwrap();
        assert_eq!(found(&mut h, "file:auth.rs"), vec![("agent".into(), 2)]);
        assert_eq!(found(&mut h, "tool:edit"), vec![("agent".into(), 2)]);
        assert_eq!(found(&mut h, "tool:edit token"), vec![("agent".into(), 2)]);
        assert!(found(&mut h, "tool:bash").is_empty());
        // A plain word finds a file name too.
        assert_eq!(found(&mut h, "auth.rs"), vec![("agent".into(), 2)]);
    }

    #[test]
    fn a_tools_result_is_not_searched() {
        let mut h = scratch();
        let result = sdk(
            2,
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "fn secret_function() {}"}
            ]}}),
        );
        h.keep("wsl", "s", "agent", &[prompt(1, "read it"), result])
            .unwrap();
        assert!(found(&mut h, "secret_function").is_empty());
    }

    #[test]
    fn a_subagent_and_a_shell_command_say_who_they_were() {
        let mut h = scratch();
        let sub = sdk(
            2,
            json!({"type": "assistant", "parent_tool_use_id": "t9", "message": {"content": [
                {"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "cargo test -p hura-core", "description": "Run the core tests"}}
            ]}}),
        );
        let shell = entry(
            3,
            ChatEvent::Shell {
                uuid: "x".into(),
                command: "git log".into(),
            },
        );
        h.keep("wsl", "s", "agent", &[sub, shell]).unwrap();
        let hits = h.search("cargo").unwrap().hits;
        assert_eq!(hits[0].role, Role::Agent);
        assert_eq!(
            hits[0].snippet,
            "Bash: cargo test -p hura-core · Run the core tests"
        );
        assert_eq!(h.search("core tests").unwrap().hits.len(), 1);
        assert_eq!(h.search("tool:shell").unwrap().hits[0].role, Role::Shell);
    }

    #[test]
    fn entries_already_kept_are_not_kept_twice() {
        let mut h = scratch();
        h.keep("wsl", "s", "agent", &[prompt(1, "one"), prompt(2, "two")])
            .unwrap();
        h.replay(
            ("wsl", "s", "agent"),
            1,
            3,
            &[prompt(2, "two"), prompt(3, "three")],
        )
        .unwrap();
        assert_eq!(h.last("wsl", "s", "agent"), 3);
        assert_eq!(h.transcript("wsl", "s", "agent").unwrap().len(), 3);
        assert_eq!(found(&mut h, "two").len(), 1);
    }

    #[test]
    fn a_host_that_started_again_leaves_the_old_transcript_ended() {
        let mut h = scratch();
        h.keep(
            "wsl",
            "s",
            "agent",
            &[prompt(1, "old one"), prompt(2, "old two")],
        )
        .unwrap();
        // Asked from 2, and the host's last is 1: a pane's way of telling.
        h.replay(("wsl", "s", "agent"), 2, 1, &[]).unwrap();
        assert_eq!(h.last("wsl", "s", "agent"), 0);
        h.replay(("wsl", "s", "agent"), 0, 1, &[prompt(1, "new one")])
            .unwrap();
        let hits = h.search("one").unwrap().hits;
        assert_eq!(hits.len(), 2);
        let old = hits.iter().find(|x| x.snippet == "old one").unwrap();
        assert!(old.ended);
        assert_eq!(old.conv, "agent");
        assert_eq!(h.transcript("wsl", "s", &old.conv_key).unwrap().len(), 2);
        assert_eq!(h.last("wsl", "s", "agent"), 1);
    }

    #[test]
    fn a_replay_from_the_start_of_another_transcript_is_told_by_its_entries() {
        let mut h = scratch();
        h.keep(
            "wsl",
            "s",
            "agent",
            &[prompt(1, "old one"), prompt(2, "old two")],
        )
        .unwrap();
        // The same entries again, as a window opened later asks for them: kept
        // once, and nothing put away.
        h.replay(
            ("wsl", "s", "agent"),
            0,
            2,
            &[prompt(1, "old one"), prompt(2, "old two")],
        )
        .unwrap();
        assert_eq!(h.search("old").unwrap().hits.len(), 2);
        assert!(h.search("old").unwrap().hits.iter().all(|x| !x.ended));

        // Entry 1 again, but not the one kept: another transcript.
        let mut new = prompt(1, "new one");
        new.at = 9_999;
        h.replay(("wsl", "s", "agent"), 0, 1, &[new]).unwrap();
        let hits = h.search("one").unwrap().hits;
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().find(|x| x.snippet == "old one").unwrap().ended);
        assert!(!hits.iter().find(|x| x.snippet == "new one").unwrap().ended);
    }

    #[test]
    fn a_removed_session_is_still_found_and_its_name_starts_again() {
        let mut h = scratch();
        h.keep("wsl", "fix-login", "agent", &[prompt(1, "make login work")])
            .unwrap();
        h.name("wsl", "fix-login", Some("Fix the login"), Some("web"))
            .unwrap();
        h.removed("wsl", "fix-login").unwrap();
        h.keep("wsl", "fix-login", "agent", &[prompt(1, "another login")])
            .unwrap();

        let hits = h.search("login").unwrap().hits;
        assert_eq!(hits.len(), 2);
        let gone = hits.iter().find(|x| x.removed).unwrap();
        assert_eq!(gone.session, "fix-login");
        assert_eq!(gone.title.as_deref(), Some("Fix the login"));
        assert_eq!(gone.project.as_deref(), Some("web"));
        assert_eq!(
            h.transcript("wsl", &gone.session_key, &gone.conv_key)
                .unwrap()[0]
                .seq,
            1
        );
        let live = hits.iter().find(|x| !x.removed).unwrap();
        assert_eq!(live.snippet, "another login");
    }

    #[test]
    fn an_ended_conversation_is_still_found() {
        let mut h = scratch();
        h.keep("wsl", "s", "chat-2", &[prompt(1, "a side question")])
            .unwrap();
        h.ended("wsl", "s", "chat-2").unwrap();
        let hits = h.search("side").unwrap().hits;
        assert_eq!((hits[0].conv.as_str(), hits[0].ended), ("chat-2", true));
        assert_eq!(h.last("wsl", "s", "chat-2"), 0);
    }

    #[test]
    fn pictures_are_not_kept() {
        let mut h = scratch();
        let shot = sdk(
            1,
            json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t", "content": [
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}}
            ]}]}}),
        );
        h.keep("wsl", "s", "agent", &[shot]).unwrap();
        let text = fs::read_to_string(h.conv_file("wsl", "s", "agent")).unwrap();
        assert!(!text.contains("AAAA"), "{text}");
        assert!(text.contains("image/png"));
    }

    #[test]
    fn a_long_result_is_kept_as_its_start() {
        let mut h = scratch();
        let long = "x".repeat(RESULT_CHARS * 3);
        let read = sdk(
            1,
            json!({"type": "user", "tool_use_result": {"file": {"content": long}}, "message": {"content": [
                {"type": "tool_result", "tool_use_id": "t", "content": long},
                {"type": "tool_result", "tool_use_id": "u", "content": [{"type": "text", "text": long}]}
            ]}}),
        );
        h.keep("wsl", "s", "agent", &[read]).unwrap();
        let text = fs::read_to_string(h.conv_file("wsl", "s", "agent")).unwrap();
        assert!(text.len() < RESULT_CHARS * 3, "{}", text.len());
        assert!(!text.contains("tool_use_result"));
    }

    #[test]
    fn what_is_appended_later_is_found_without_reading_it_all_again() {
        let mut h = scratch();
        h.keep("wsl", "s", "agent", &[prompt(1, "first")]).unwrap();
        assert_eq!(found(&mut h, "first").len(), 1);
        h.keep("wsl", "s", "agent", &[prompt(2, "second")]).unwrap();
        assert_eq!(found(&mut h, "second").len(), 1);
        assert_eq!(found(&mut h, "first").len(), 1);
    }

    #[test]
    fn newest_first_and_bounded() {
        let mut h = scratch();
        let many: Vec<_> = (1..=LIMIT as u64 + 5)
            .map(|i| prompt(i, "same words"))
            .collect();
        h.keep("wsl", "s", "agent", &many).unwrap();
        let f = h.search("same").unwrap();
        assert!(f.more);
        assert_eq!(f.hits.len(), LIMIT);
        assert_eq!(f.hits[0].seq, LIMIT as u64 + 5);
    }

    #[test]
    fn names_that_are_not_paths_stay_inside_the_directory() {
        assert_eq!(safe("../etc"), "%2e.%2fetc");
        assert_eq!(unsafe_name(&safe("my server/x@y")), "my server/x@y");
        assert_eq!(safe_key("..%2f..@123"), "%2e.%2f..@123");
        assert_eq!(safe_key("a@../b"), "a%40..%2fb");
        let mut h = scratch();
        h.keep("my server", "s", "agent", &[prompt(1, "hello")])
            .unwrap();
        assert_eq!(h.search("hello").unwrap().hits[0].server, "my server");
    }

    #[test]
    fn a_snippet_is_cut_around_the_word() {
        let long = format!("{} needle {}", "a ".repeat(100), "b ".repeat(100));
        let s = snippet(&long, &["needle".into()]);
        assert!(s.starts_with('…') && s.ends_with('…'), "{s}");
        assert!(s.contains("needle"));
        assert_eq!(snippet("short\n\nline", &[]), "short line");
    }

    #[test]
    fn a_query_is_words_and_qualifiers() {
        let q = Query::parse(r#"  Token file:Auth.rs tool:Edit "two words" file: "#);
        assert_eq!(q.words, vec!["token", "two words", "file:"]);
        assert_eq!(q.files, vec!["auth.rs"]);
        assert_eq!(q.tools, vec!["edit"]);
    }
}
