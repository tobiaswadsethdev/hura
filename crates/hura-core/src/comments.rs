//! Review comments on a session's diff, kept until they are sent to the agent.
//!
//! A review is written a line at a time and delivered in one go: telling the
//! agent about each remark as it is written would interrupt it six times to say
//! six things that belong in one message, and the second interruption lands
//! while it is acting on the first.
//!
//! Kept on the **server**, per session, beside the events feed and for the same
//! reason: a client is a window onto a session, and a review half-written when
//! the window closes is work. It also means the review is the session's rather
//! than the window's, so a second client sees it and the agent is told once
//! whichever one sends it.
//!
//! What is stored is what the person wrote plus enough to say where they wrote
//! it. Not a line *identity*: the working copy moves under a review -- the agent
//! is still going -- and a comment that tried to follow a line through later
//! edits would either be wrong or need a diff of the diff. The excerpt is kept
//! verbatim instead, so the message quotes what was actually being looked at
//! even when the file has moved on.
//!
//! **A pull request's reviewers are heard the same way.** What they said on
//! the forge goes to the session that owns the pull request as one message in
//! this format, so an agent reads a reviewer's remark exactly as it reads
//! yours. Those are not kept here: they are kept by the forge, and the client
//! that read the pull request hands them over ([`pull_remarks`]). What is kept
//! is which of them have been sent, by the forge's own ids, because the same
//! comments come back with every read and an agent told twice does the work
//! twice. Asked for by a button and by a timer alike, so that either can run
//! as often as it likes and the agent hears each remark once.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::tracker::{IssuePerson as Person, PullDetail, PullVerdict};

/// One remark against one line of a diff.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// Unique within a session, and the handle for removing one. Assigned from
    /// the highest already present rather than from the count, so removing the
    /// middle of a review cannot make the next comment reuse an id.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: u64,
    /// Path as the diff gives it, without the `a/` or `b/` prefix.
    pub file: String,
    /// Line number in the file, as the diff's hunk header counts it. Zero for a
    /// comment against a file rather than a line -- an untracked file has no
    /// line numbers to point at.
    pub line: u32,
    /// The diff line being commented on, verbatim, `+`/`-`/space and all.
    pub excerpt: String,
    pub body: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub at: u64,
}

/// A comment a client is asking to add.
///
/// The id and the timestamp are the server's to assign: two clients writing at
/// once would otherwise agree on an id and one of them would vanish.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewComment {
    pub file: String,
    pub line: u32,
    pub excerpt: String,
    pub body: String,
}

/// One entry in a pull request's conversation, as it goes to the agent: a
/// thread on a line, a comment on the pull request, or a review that said
/// something.
///
/// Made by the client from what the forge answered, and sent to the server
/// whole: the server cannot read the forge (the trackers' tokens are the
/// account's, not the server's), and what it does with this is the same
/// either way.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRemark {
    /// Path in the repository, without the `/` Azure DevOps starts it with.
    /// Empty for a remark on the pull request as a whole.
    pub file: String,
    /// Zero for one that is not on a line.
    pub line: u32,
    /// The code it was about has changed since it was written.
    pub outdated: bool,
    /// The thread, oldest first: replies are sent with what they reply to.
    pub said: Vec<PullSaid>,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullSaid {
    /// Unique across every pull request: the tracker, the repository, the
    /// pull request and the forge's id for the comment. What is remembered
    /// once it has been sent.
    pub id: String,
    /// Who said it, and what they decided when it came with a review.
    pub who: String,
    /// Markdown, as they wrote it.
    pub body: String,
}

/// Where a session's unsent review lives.
fn path(session: &str) -> PathBuf {
    // Beside the events feed, under a directory of its own so a session name can
    // never collide with `sessions.json`.
    dir().join(format!("{session}.jsonl"))
}

/// Where the ids of the pull request comments already sent to a session's
/// agent are.
fn sent_path(session: &str) -> PathBuf {
    dir().join(format!("{session}.sent"))
}

fn dir() -> PathBuf {
    crate::store::Store::default_path().with_file_name("comments")
}

/// The unsent review, oldest first -- which is reading order for a diff.
pub fn list(session: &str) -> Vec<Comment> {
    list_at(&path(session))
}

/// Each of these has a twin taking the file, for the reason
/// [`crate::store::Store::load_from`] does: a review lives in the config
/// directory, and a test that had to move `XDG_CONFIG_HOME` to reach it would
/// be a test that races every other test doing the same.
pub fn list_at(path: &Path) -> Vec<Comment> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        // A line that will not parse is dropped rather than failing the read:
        // one corrupt record should cost that comment, not the review.
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn write(path: &Path, comments: &[Comment]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let mut text = String::new();
    for c in comments {
        let line = serde_json::to_string(c).map_err(|e| e.to_string())?;
        text.push_str(&line);
        text.push('\n');
    }
    fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Add one, and answer with the review as it now stands.
pub fn add(session: &str, new: NewComment) -> Result<Vec<Comment>, String> {
    add_at(&path(session), new)
}

pub fn add_at(path: &Path, new: NewComment) -> Result<Vec<Comment>, String> {
    if new.body.trim().is_empty() {
        return Err("a comment with nothing in it".into());
    }
    let mut all = list_at(path);
    let id = all.iter().map(|c| c.id).max().unwrap_or(0) + 1;
    all.push(Comment {
        id,
        file: new.file,
        line: new.line,
        excerpt: new.excerpt,
        body: new.body,
        at: now(),
    });
    write(path, &all)?;
    Ok(all)
}

/// Remove one by id. Removing one that is not there is not an error: two
/// clients deleting the same comment should both end up with it gone.
pub fn remove(session: &str, id: u64) -> Result<Vec<Comment>, String> {
    remove_at(&path(session), id)
}

pub fn remove_at(path: &Path, id: u64) -> Result<Vec<Comment>, String> {
    let mut all = list_at(path);
    all.retain(|c| c.id != id);
    write(path, &all)?;
    Ok(all)
}

/// Drop the whole review. Called once it has been delivered.
pub fn clear(session: &str) -> Result<(), String> {
    let path = path(session);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not clear {}: {e}", path.display())),
    }
}

/// Drop a destroyed session's unsent review.
///
/// The same file as [`clear`] and deliberately not the same function: that one
/// is the last step of delivering a review and its failure is worth returning,
/// this one is part of removing a session and its failure is not -- the session
/// has gone either way. Mirrors [`crate::events::forget_kept`], which is the
/// other thing a session's name owns after the session itself has stopped
/// existing.
///
/// Left behind, this outlived the session: a new session taking the name
/// inherited the old one's comments, and the next review sent handed its agent
/// notes on a diff it had never produced. The record of what its pull request's
/// reviewers have been heard saying goes with it, for the opposite reason: a
/// new session on the same pull request has been told none of it.
pub fn forget(session: &str) {
    forget_at(&path(session));
    forget_at(&sent_path(session));
}

pub fn forget_at(path: &Path) {
    let _ = fs::remove_file(path);
}

/// What a pull request's reviewers have said that is still for its author:
/// every thread that is not resolved, and has something in it from someone
/// other than the pull request's author.
///
/// The author's own words are left out on their own, because they are not a
/// review: a note to self, or "done" under a thread. Under a reviewer's
/// remark they are kept, because a reply is read with what it replies to.
/// Which side is the author is the forge's to say: you, when you opened it,
/// and otherwise whoever did, by name.
///
/// A review's verdict with nothing written goes nowhere: "changes
/// requested" says nothing an agent could act on, and the threads that came
/// with it are entries of their own.
pub fn pull_remarks(pull: &PullDetail) -> Vec<PullRemark> {
    let by_author = |p: &Option<Person>| match p {
        Some(p) if pull.mine => p.me,
        Some(p) => pull.author.as_ref().is_some_and(|a| a.name == p.name),
        None => false,
    };
    pull.conversation
        .iter()
        .filter(|t| !t.resolved)
        .filter_map(|t| {
            let said: Vec<PullSaid> = t
                .comments
                .iter()
                .filter(|c| !c.body.trim().is_empty())
                .map(|c| {
                    let name = c.author.as_ref().map_or("someone", |a| a.name.as_str());
                    PullSaid {
                        id: format!(
                            "{}/{}/{}/{}",
                            pull.tracker,
                            pull.repo,
                            pull.id,
                            // A cloud from before comments had ids: when it
                            // was said, and by whom, is what is left.
                            if c.id.is_empty() {
                                format!("{}:{name}", c.created.as_deref().unwrap_or(""))
                            } else {
                                c.id.clone()
                            }
                        ),
                        who: match t.verdict.and_then(decided) {
                            Some(verdict) => format!("{name}, {verdict}"),
                            None => name.to_string(),
                        },
                        body: c.body.trim().to_string(),
                    }
                })
                .collect();
            let reviewed = t
                .comments
                .iter()
                .any(|c| !c.body.trim().is_empty() && !by_author(&c.author));
            (reviewed && !said.is_empty()).then(|| PullRemark {
                file: t
                    .path
                    .as_deref()
                    .unwrap_or("")
                    .trim_start_matches('/')
                    .to_string(),
                line: t.line.unwrap_or(0),
                outdated: t.outdated,
                said,
            })
        })
        .collect()
}

/// What a review decided, in the words a message says it with. `None` for
/// the ones that decide nothing.
fn decided(verdict: PullVerdict) -> Option<&'static str> {
    Some(match verdict {
        PullVerdict::Approved => "approving",
        PullVerdict::Suggestions => "approving with suggestions",
        PullVerdict::ChangesRequested => "asking for changes",
        PullVerdict::Waiting => "waiting for the author",
        PullVerdict::Rejected => "rejecting",
        PullVerdict::Commented | PullVerdict::Dismissed | PullVerdict::Pending => return None,
    })
}

/// The ids of the pull request comments already sent to a session's agent.
pub fn sent(session: &str) -> BTreeSet<String> {
    sent_at(&sent_path(session))
}

pub fn sent_at(path: &Path) -> BTreeSet<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// Remember that these have been sent. Added to the end of the file, so
/// two sends at once cost at worst a line written twice, not one lost.
pub fn mark_sent(session: &str, remarks: &[PullRemark]) -> Result<(), String> {
    mark_sent_at(&sent_path(session), remarks)
}

pub fn mark_sent_at(path: &Path, remarks: &[PullRemark]) -> Result<(), String> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let text: String = remarks
        .iter()
        .flat_map(|r| &r.said)
        .map(|s| format!("{}\n", s.id))
        .collect();
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// The remarks with something in them the agent has not been sent.
///
/// A thread with a new reply is sent whole, the old with the new: a reply
/// saying "not that one, the other" means nothing without what it answers.
pub fn unsent(remarks: Vec<PullRemark>, sent: &BTreeSet<String>) -> Vec<PullRemark> {
    remarks
        .into_iter()
        .filter(|r| r.said.iter().any(|s| !sent.contains(&s.id)))
        .collect()
}

/// A pull request's remarks as one message for the agent, in the format of
/// [`message`]: by file and line, each quoted under where it was made. What
/// was said about the pull request as a whole comes first, under a heading
/// of its own in place of a file's name.
///
/// `key` is the forge's name for the pull request: `#58`, `!26056`.
pub fn pull_message(key: &str, remarks: &[PullRemark]) -> String {
    let entries: Vec<Comment> = remarks
        .iter()
        .enumerate()
        .map(|(i, r)| Comment {
            id: i as u64,
            file: r.file.clone(),
            line: r.line,
            excerpt: if r.outdated {
                "(the code has changed since)".into()
            } else {
                String::new()
            },
            body: r
                .said
                .iter()
                .map(|s| {
                    let mut lines = s.body.lines();
                    let first = lines.next().unwrap_or("");
                    // Lines after the first sit under the name, so where one
                    // person stops and the next starts is plain.
                    let rest: String = lines.map(|l| format!("\n  {l}")).collect();
                    format!("{}: {first}{rest}", s.who)
                })
                .collect::<Vec<_>>()
                .join("\n"),
            at: 0,
        })
        .collect();
    render(
        &format!("on pull request {key}"),
        "The pull request as a whole",
        &entries,
    )
}

/// The review as one message for the agent.
///
/// Grouped by file and in line order, because that is how it will be acted on;
/// the excerpt is quoted so the agent does not have to trust that the line
/// numbers still mean what they meant when the review was written.
///
/// Plain text with no markup of its own. Whatever is on the other end is a
/// terminal agent reading a paste, and a format it has to parse is a format it
/// can get wrong.
pub fn message(comments: &[Comment]) -> String {
    render("on the diff", "", comments)
}

/// [`message`], for comments `on` something, with `general` as the heading
/// of the ones that are on no file.
fn render(on: &str, general: &str, comments: &[Comment]) -> String {
    let mut sorted: Vec<&Comment> = comments.iter().collect();
    sorted.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.id.cmp(&b.id))
    });

    let count = sorted.len();
    let mut out = format!(
        "Here {} {} review comment{} {on}:\n",
        if count == 1 { "is" } else { "are" },
        count,
        if count == 1 { "" } else { "s" },
    );

    let mut current = None;
    for c in sorted {
        if current != Some(&c.file) {
            let heading = if c.file.is_empty() { general } else { &c.file };
            out.push_str(&format!("\n{heading}\n"));
            current = Some(&c.file);
        }
        let excerpt = c.excerpt.trim_end();
        if c.line > 0 {
            out.push_str(format!("  line {}: {excerpt}", c.line).trim_end());
            out.push('\n');
        } else if !excerpt.is_empty() {
            out.push_str(&format!("  {excerpt}\n"));
        }
        for line in c.body.trim().lines() {
            out.push_str(&format!("    {line}\n"));
        }
    }
    out
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(id: u64, file: &str, line: u32, excerpt: &str, body: &str) -> Comment {
        Comment {
            id,
            file: file.into(),
            line,
            excerpt: excerpt.into(),
            body: body.into(),
            at: 0,
        }
    }

    /// Grouped by file and in line order, whatever order they were written in:
    /// a review is acted on file by file, not in the order someone happened to
    /// notice things.
    #[test]
    fn the_message_is_grouped_by_file_and_in_line_order() {
        let msg = message(&[
            comment(1, "src/b.rs", 10, "+    let x = 1;", "unused"),
            comment(2, "src/a.rs", 30, "+    bar()", "second"),
            comment(3, "src/a.rs", 4, "+    foo()", "first"),
        ]);
        let a = msg.find("src/a.rs").unwrap();
        let b = msg.find("src/b.rs").unwrap();
        assert!(a < b, "{msg}");
        assert!(
            msg.find("first").unwrap() < msg.find("second").unwrap(),
            "{msg}"
        );
        assert!(msg.starts_with("Here are 3 review comments"), "{msg}");
    }

    /// One comment reads as one, not as "1 comments".
    #[test]
    fn one_comment_reads_as_one() {
        let msg = message(&[comment(1, "a.rs", 1, "+x", "no")]);
        assert!(
            msg.starts_with("Here is 1 review comment on the diff:"),
            "{msg}"
        );
    }

    /// A comment against a file rather than a line -- an untracked file has no
    /// line numbers -- says so instead of claiming line zero.
    #[test]
    fn a_comment_with_no_line_does_not_claim_line_zero() {
        let msg = message(&[comment(1, "notes.md", 0, "notes.md", "why is this here")]);
        assert!(!msg.contains("line 0"), "{msg}");
        assert!(msg.contains("why is this here"), "{msg}");
    }

    /// A body of several lines stays several lines, indented under its comment,
    /// so the agent can tell where one remark ends and the next begins.
    #[test]
    fn a_multi_line_body_stays_indented_under_its_comment() {
        let msg = message(&[comment(1, "a.rs", 2, "+x", "one\ntwo")]);
        assert!(msg.contains("    one\n    two\n"), "{msg}");
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hura-comments-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("s.jsonl")
    }

    fn draft(file: &str, line: u32, body: &str) -> NewComment {
        NewComment {
            file: file.into(),
            line,
            excerpt: "+ something".into(),
            body: body.into(),
        }
    }

    /// Forgetting is what destroying a session does with the review it never
    /// sent. Left behind, the next session to take the name inherited it.
    #[test]
    fn a_forgotten_review_leaves_nothing_for_the_next_session_of_that_name() {
        let path = scratch("forget");
        add_at(&path, draft("a.rs", 1, "not sent")).unwrap();
        assert_eq!(list_at(&path).len(), 1);

        forget_at(&path);

        assert!(list_at(&path).is_empty());
        assert!(!path.exists(), "the file goes, not just its contents");
        // Twice is the same as once: two clients removing one session should
        // both end up with it gone.
        forget_at(&path);

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// A review survives being written and read back, which is the whole reason
    /// it is on the server rather than in the window.
    #[test]
    fn a_review_round_trips() {
        let path = scratch("roundtrip");
        assert!(list_at(&path).is_empty());
        add_at(&path, draft("a.rs", 1, "first")).unwrap();
        let all = add_at(&path, draft("b.rs", 2, "second")).unwrap();
        assert_eq!(all.len(), 2);
        let read = list_at(&path);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].body, "first");
        assert_eq!(read[1].file, "b.rs");
    }

    /// Ids come from the highest already present, not from the count: removing
    /// the middle of a review must not make the next comment reuse an id, or
    /// deleting one would delete two.
    #[test]
    fn removing_the_middle_does_not_let_an_id_be_reused() {
        let path = scratch("ids");
        add_at(&path, draft("a.rs", 1, "one")).unwrap();
        add_at(&path, draft("a.rs", 2, "two")).unwrap();
        add_at(&path, draft("a.rs", 3, "three")).unwrap();
        remove_at(&path, 2).unwrap();
        let all = add_at(&path, draft("a.rs", 4, "four")).unwrap();
        let ids: Vec<u64> = all.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![1, 3, 4]);
    }

    /// Removing one that has already gone is not an error: two windows on the
    /// same session may both be told to remove it.
    #[test]
    fn removing_one_that_is_not_there_is_not_an_error() {
        let path = scratch("gone");
        add_at(&path, draft("a.rs", 1, "one")).unwrap();
        let all = remove_at(&path, 99).unwrap();
        assert_eq!(all.len(), 1);
    }

    /// An empty body is refused where it is written rather than turning into a
    /// blank line in the message the agent is sent.
    #[test]
    fn a_comment_with_nothing_in_it_is_refused() {
        let path = scratch("empty");
        assert!(add_at(&path, draft("a.rs", 1, "   ")).is_err());
        assert!(list_at(&path).is_empty());
    }

    use crate::tracker::{IssuePerson, Kind, PullComment, PullState, PullThread};

    fn person(name: &str, me: bool) -> Option<IssuePerson> {
        Some(IssuePerson {
            name: name.into(),
            me,
        })
    }

    fn said(id: &str, who: Option<IssuePerson>, body: &str) -> PullComment {
        PullComment {
            id: id.into(),
            author: who,
            body: body.into(),
            created: None,
        }
    }

    fn thread(path: Option<&str>, line: Option<u32>, comments: Vec<PullComment>) -> PullThread {
        PullThread {
            id: String::new(),
            path: path.map(String::from),
            line,
            resolved: false,
            outdated: false,
            verdict: None,
            author: comments.first().and_then(|c| c.author.clone()),
            at: None,
            comments,
            can_reply: true,
        }
    }

    /// Yours, with Ada reviewing it.
    fn pull(conversation: Vec<PullThread>) -> PullDetail {
        PullDetail {
            tracker: "gh".into(),
            kind: Kind::GitHub,
            id: "58".into(),
            key: "#58".into(),
            repo: "o/r".into(),
            title: String::new(),
            url: String::new(),
            state: PullState::Open,
            draft: false,
            conflicts: false,
            fork: false,
            author: person("Tobias", true),
            branch: String::new(),
            base: String::new(),
            created: None,
            updated: None,
            body: String::new(),
            additions: None,
            deletions: None,
            changed_files: None,
            checks: Vec::new(),
            reviewers: Vec::new(),
            conversation,
            older: 0,
            offers: Vec::new(),
            my_vote: None,
            mine: true,
            merging: None,
            warnings: Vec::new(),
        }
    }

    /// What is sent is what is still for the author: not a resolved thread,
    /// not the author's own note to self, and not a verdict with nothing
    /// written. A reply of the author's under a reviewer's remark goes with
    /// it, since it is read with what it answers.
    #[test]
    fn only_what_reviewers_said_that_is_still_open_is_sent() {
        let mut resolved = thread(
            Some("a.rs"),
            Some(1),
            vec![said("t1", person("Ada", false), "fixed already")],
        );
        resolved.resolved = true;
        let mut verdict = thread(None, None, Vec::new());
        verdict.verdict = Some(PullVerdict::Approved);
        let p = pull(vec![
            resolved,
            thread(
                None,
                None,
                vec![said("c1", person("Tobias", true), "note to self")],
            ),
            verdict,
            thread(
                Some("/src/b.rs"),
                Some(7),
                vec![
                    said("t2", person("Ada", false), "Why three?"),
                    said("t3", person("Tobias", true), "Measured."),
                ],
            ),
        ]);

        let remarks = pull_remarks(&p);

        assert_eq!(remarks.len(), 1, "{remarks:?}");
        let r = &remarks[0];
        assert_eq!(r.file, "src/b.rs", "without Azure DevOps' leading slash");
        assert_eq!(r.line, 7);
        let ids: Vec<&str> = r.said.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["gh/o/r/58/t2", "gh/o/r/58/t3"]);
    }

    /// On somebody else's pull request the author is them, by name, and you
    /// are one of its reviewers.
    #[test]
    fn on_somebody_elses_pull_request_their_words_are_the_authors() {
        let mut p = pull(vec![
            thread(None, None, vec![said("c1", person("Sam", false), "ready")]),
            thread(
                None,
                None,
                vec![said("c2", person("Tobias", true), "not yet")],
            ),
        ]);
        p.mine = false;
        p.author = person("Sam", false);

        let remarks = pull_remarks(&p);

        assert_eq!(remarks.len(), 1);
        assert_eq!(remarks[0].said[0].body, "not yet");
    }

    /// The message is the review's: grouped by file, in line order, with what
    /// was said about the whole pull request first and a verdict said with
    /// its review.
    #[test]
    fn a_pull_requests_message_reads_like_a_review() {
        let mut review = thread(
            None,
            None,
            vec![said(
                "r1",
                person("Ada", false),
                "One thing first.\nAnd a second.",
            )],
        );
        review.verdict = Some(PullVerdict::ChangesRequested);
        let mut old = thread(
            Some("a.rs"),
            Some(42),
            vec![said("t1", person("Ada", false), "Why three?")],
        );
        old.outdated = true;
        let p = pull(vec![
            thread(
                Some("b.rs"),
                Some(3),
                vec![said("t2", person("Sam", false), "typo")],
            ),
            old,
            review,
        ]);

        let msg = pull_message(&p.key, &pull_remarks(&p));

        assert!(
            msg.starts_with("Here are 3 review comments on pull request #58:\n"),
            "{msg}"
        );
        assert!(
            msg.contains(
                "\nThe pull request as a whole\n    Ada, asking for changes: One thing first.\n      And a second.\n"
            ),
            "{msg}"
        );
        assert!(
            msg.contains("\na.rs\n  line 42: (the code has changed since)\n    Ada: Why three?\n"),
            "{msg}"
        );
        assert!(msg.contains("\nb.rs\n  line 3:\n    Sam: typo\n"), "{msg}");
        let whole = msg.find("The pull request as a whole").unwrap();
        assert!(whole < msg.find("a.rs").unwrap(), "{msg}");
        assert!(
            msg.find("a.rs").unwrap() < msg.find("b.rs").unwrap(),
            "{msg}"
        );
    }

    /// Each remark is sent once. A thread with a new reply goes again, whole,
    /// because the reply means nothing without what it answers.
    #[test]
    fn what_has_been_sent_is_not_sent_again_until_a_thread_moves() {
        let path = scratch("sent");
        let first = pull(vec![thread(
            Some("a.rs"),
            Some(1),
            vec![said("t1", person("Ada", false), "Why?")],
        )]);
        let remarks = unsent(pull_remarks(&first), &sent_at(&path));
        assert_eq!(remarks.len(), 1);
        mark_sent_at(&path, &remarks).unwrap();

        assert!(unsent(pull_remarks(&first), &sent_at(&path)).is_empty());

        let replied = pull(vec![thread(
            Some("a.rs"),
            Some(1),
            vec![
                said("t1", person("Ada", false), "Why?"),
                said("t2", person("Ada", false), "Ping."),
            ],
        )]);
        let again = unsent(pull_remarks(&replied), &sent_at(&path));
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].said.len(), 2, "sent whole");
    }

    /// One unreadable line costs that comment, not the review around it.
    #[test]
    fn a_corrupt_line_does_not_lose_the_rest_of_the_review() {
        let path = scratch("corrupt");
        add_at(&path, draft("a.rs", 1, "kept")).unwrap();
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("{not json at all\n");
        fs::write(&path, text).unwrap();
        let all = list_at(&path);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].body, "kept");
    }
}
