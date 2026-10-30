//! One pull request, read in full, and what a reviewer does to it from here.
//!
//! The sidebar's row says where a pull request stands. This says what it is:
//! the description, every check by name, who has reviewed it and how, the
//! conversation, and the files it changes with both sides of each. It also
//! does the four things a reviewer does without needing the forge's page:
//! comment, reply to a thread, approve, and ask for changes.
//!
//! Read and written with the tracker's own token, like the rest of this
//! module. GitHub is one GraphQL request for the pull request and REST for the
//! files and the writes, which GraphQL makes no shorter. Azure DevOps has no
//! request that answers more than one of these questions, so it is one each.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::issue::Person;
use super::{AzureMe, JSON, Kind, Source, Stored, azure_auth, azure_me, get, post, send, string};
use crate::git::{Change, FileDiff};

/// A pull request, read in full.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullDetail {
    pub tracker: String,
    pub kind: Kind,
    pub id: String,
    /// `#58` on GitHub, `!26056` on Azure DevOps.
    pub key: String,
    /// `owner/name` on GitHub, the repository's name on Azure DevOps.
    pub repo: String,
    pub title: String,
    pub url: String,
    pub state: PullState,
    pub draft: bool,
    pub conflicts: bool,
    /// Opened from another repository, whose branch a worktree here cannot
    /// check out or push to.
    pub fork: bool,
    pub author: Option<Person>,
    /// The branch it merges from, and the one it merges into.
    pub branch: String,
    pub base: String,
    pub created: Option<String>,
    pub updated: Option<String>,
    /// Markdown, as its author wrote it.
    pub body: String,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub changed_files: Option<u32>,
    pub checks: Vec<PullCheck>,
    pub reviewers: Vec<PullReviewer>,
    /// Comments, reviews and threads on lines, oldest first.
    pub conversation: Vec<PullThread>,
    /// How many entries are older than the ones in `conversation`, and so only
    /// on the forge's page.
    pub older: u32,
    /// The verdicts you can give it from here. Empty when there are none: on
    /// GitHub your own pull request, which GitHub will not let you review, and
    /// anywhere a pull request that is no longer open.
    pub offers: Vec<PullVerdict>,
    /// Where your own review stands.
    pub my_vote: Option<PullVerdict>,
    /// Whether you opened it.
    pub mine: bool,
    /// Parts that could not be read, in words. The checks on Azure DevOps are
    /// two requests of their own, and a pull request without them is still
    /// worth showing.
    pub warnings: Vec<String>,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PullState {
    Open,
    Merged,
    Closed,
}

/// One check, by name.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullCheck {
    pub name: String,
    pub state: PullCheckState,
    /// Where its log or its page is.
    pub url: Option<String>,
    /// What it said about itself, when it said anything.
    pub detail: Option<String>,
    /// Whether the pull request cannot merge without it.
    pub required: bool,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PullCheckState {
    Passing,
    Failing,
    Pending,
    /// Ran and said neither yes nor no, or was cancelled.
    Neutral,
    Skipped,
}

/// Somebody asked to review it, or who did.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullReviewer {
    pub who: Person,
    pub verdict: PullVerdict,
    /// Required by a branch policy, or a code owner on GitHub.
    pub required: bool,
    /// A team or a group rather than a person.
    pub group: bool,
}

/// Where one review stands, or the one you give.
///
/// The two forges do not mean the same things and are not made to: GitHub has
/// approve and request changes, Azure DevOps has a five-step vote. Each side
/// uses the words it has, and [`PullDetail::offers`] says which apply.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PullVerdict {
    Approved,
    /// Azure DevOps: approved with suggestions.
    Suggestions,
    /// GitHub: changes requested.
    ChangesRequested,
    /// Azure DevOps: waiting for the author.
    Waiting,
    /// Azure DevOps: rejected.
    Rejected,
    /// GitHub: a review that only commented.
    Commented,
    /// GitHub: a review somebody dismissed.
    Dismissed,
    /// Asked for and not given. Given as a vote on Azure DevOps, it takes
    /// yours back.
    Pending,
}

/// One entry in the conversation: a comment, a review, or a thread on a line.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullThread {
    /// What a reply is addressed to: on GitHub the thread's first comment, on
    /// Azure DevOps the thread. Empty when it is not a thread.
    pub id: String,
    /// The file and line it is about, for a thread on the code.
    pub path: Option<String>,
    pub line: Option<u32>,
    pub resolved: bool,
    /// The code it was about has changed since.
    pub outdated: bool,
    /// When this is a review, what it decided.
    pub verdict: Option<PullVerdict>,
    /// Who gave the verdict, which a review with nothing written has no
    /// comment to say.
    pub author: Option<Person>,
    pub at: Option<String>,
    pub comments: Vec<PullComment>,
    pub can_reply: bool,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullComment {
    pub author: Option<Person>,
    /// Markdown.
    pub body: String,
    pub created: Option<String>,
}

/// The files a pull request changes, and the two commits it is measured
/// between.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullFiles {
    /// How the repository is addressed for the files' contents: `owner/name`
    /// on GitHub, the repository's id on Azure DevOps. Handed back to
    /// [`pull_file`].
    pub repo: String,
    /// Where the branch left its base, which is what a pull request's diff is
    /// measured from. Not the base branch's tip: what landed there since is
    /// not this pull request's change.
    pub base: String,
    pub head: String,
    pub files: Vec<PullFile>,
    /// There are more files than the forge lists in one answer.
    pub truncated: bool,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullFile {
    pub path: String,
    /// Where a renamed file was before.
    pub old_path: Option<String>,
    pub change: Change,
    /// Lines added and removed. `None` on Azure DevOps, whose list does not
    /// count them.
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

/// Read one pull request.
pub fn pull(stored: &Stored, repo: &str, id: &str) -> Result<PullDetail, String> {
    let (source, token) = connect(stored, id)?;
    match source.kind {
        Kind::GitHub => {
            let (owner, name) = github_repo(repo)?;
            let data = graphql(
                token,
                GITHUB_PULL,
                json!({ "owner": owner, "name": name, "number": number(id)? }),
            )?;
            parse_github(&data, source)
        }
        Kind::AzureDevOps => {
            let me = azure_me(source, token)?;
            let auth = azure_auth(token);
            let pr = azure_pull(source, &auth, id)?;
            let at = AzureAt::of(source, &pr)?;
            let threads = get(&at.pull_url(id, "threads", ""), &auth, &[])?;
            let mut warnings = Vec::new();
            let statuses = get(&at.pull_url(id, "statuses", ""), &auth, &[])
                .map_err(|e| warnings.push(format!("statuses: {e}")))
                .ok();
            let evaluations = get(&at.evaluations_url(id), &auth, &[])
                .map_err(|e| warnings.push(format!("policies: {e}")))
                .ok();
            let mut detail = parse_azure(
                &pr,
                &threads,
                statuses.as_ref(),
                evaluations.as_ref(),
                source,
                &me,
            )?;
            if !warnings.is_empty() {
                detail.warnings.push(format!(
                    "its checks could not all be read: {}",
                    warnings.join("; ")
                ));
            }
            Ok(detail)
        }
        Kind::Jira => unreachable!("connect refuses Jira"),
    }
}

/// The files a pull request changes.
pub fn pull_files(stored: &Stored, repo: &str, id: &str) -> Result<PullFiles, String> {
    let (source, token) = connect(stored, id)?;
    match source.kind {
        Kind::GitHub => {
            let (owner, name) = github_repo(repo)?;
            let auth = format!("Bearer {token}");
            let pr = get(
                &format!(
                    "https://api.github.com/repos/{owner}/{name}/pulls/{}",
                    number(id)?
                ),
                &auth,
                &[],
            )?;
            let sha = |side: &str| {
                pr.get(side)
                    .map(|s| string(s, "sha"))
                    .filter(|s| is_sha(s))
                    .ok_or_else(|| format!("github did not say where the pull request's {side} is"))
            };
            let (base, head) = (sha("base")?, sha("head")?);
            // Three dots: from where the branch left its base, the way the
            // pull request's own page measures it.
            let compare = get(
                &format!("https://api.github.com/repos/{owner}/{name}/compare/{base}...{head}"),
                &auth,
                &[],
            )?;
            let changed = pr.get("changed_files").and_then(Value::as_u64);
            parse_github_files(&compare, repo, changed)
        }
        Kind::AzureDevOps => {
            let auth = azure_auth(token);
            let pr = azure_pull(source, &auth, id)?;
            let at = AzureAt::of(source, &pr)?;
            let iterations = get(&at.pull_url(id, "iterations", ""), &auth, &[])?;
            let last = iterations
                .get("value")
                .and_then(Value::as_array)
                .and_then(|i| i.last())
                .ok_or("azure devops listed no pushes to this pull request")?;
            let iteration = last
                .get("id")
                .and_then(Value::as_u64)
                .ok_or("azure devops did not number its last push")?;
            let changes = get(
                &at.pull_url(
                    id,
                    &format!("iterations/{iteration}/changes"),
                    "&$top=2000&$compareTo=0",
                ),
                &auth,
                &[],
            )?;
            parse_azure_files(last, &changes, &at.repo)
        }
        Kind::Jira => unreachable!("connect refuses Jira"),
    }
}

/// Both sides of one file, for the diff editor.
///
/// `repo`, `base` and `head` are what [`pull_files`] answered with. A side
/// the file does not exist on (an added file's base, a deleted one's head) is
/// empty rather than asked for.
pub fn pull_file(
    stored: &Stored,
    repo: &str,
    base: &str,
    head: &str,
    file: &PullFile,
) -> Result<FileDiff, String> {
    let source = &stored.source;
    let token = token_of(stored)?;
    if !is_sha(base) || !is_sha(head) {
        return Err("those are not commits".into());
    }
    let before_path = file.old_path.as_deref().unwrap_or(&file.path);
    let wants_before = file.change != Change::Added;
    let wants_after = file.change != Change::Deleted;
    let (original, modified, binary) = match source.kind {
        Kind::GitHub => {
            let (owner, name) = github_repo(repo)?;
            let data = graphql(
                token,
                GITHUB_BLOBS,
                json!({
                    "owner": owner,
                    "name": name,
                    "before": format!("{base}:{before_path}"),
                    "after": format!("{head}:{}", file.path),
                }),
            )?;
            let blob = |side: &str, wanted: bool| -> (String, bool) {
                let b = data.get("repository").and_then(|r| r.get(side));
                match b {
                    Some(b) if wanted && !b.is_null() => {
                        let binary = b.get("isBinary").and_then(Value::as_bool).unwrap_or(false)
                            || b.get("isTruncated")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                        (string(b, "text"), binary)
                    }
                    _ => (String::new(), false),
                }
            };
            let (original, a) = blob("before", wants_before);
            let (modified, b) = blob("after", wants_after);
            (original, modified, a || b)
        }
        Kind::AzureDevOps => {
            if !is_guid(repo) {
                return Err(format!("`{repo}` is not an Azure DevOps repository id"));
            }
            let auth = azure_auth(token);
            let item = |path: &str, commit: &str| -> Result<(String, bool), String> {
                let at = AzureAt {
                    org: source.org.clone().unwrap_or_default(),
                    project: source.project.clone().unwrap_or_default(),
                    project_id: String::new(),
                    repo: repo.to_string(),
                };
                let body = get(&at.item_url(path, commit), &auth, &[])?;
                let binary = body
                    .get("contentMetadata")
                    .and_then(|m| m.get("isBinary"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                Ok((
                    if binary {
                        String::new()
                    } else {
                        string(&body, "content")
                    },
                    binary,
                ))
            };
            let (original, a) = if wants_before {
                item(before_path, base)?
            } else {
                (String::new(), false)
            };
            let (modified, b) = if wants_after {
                item(&file.path, head)?
            } else {
                (String::new(), false)
            };
            (original, modified, a || b)
        }
        Kind::Jira => return Err("Jira has no pull requests".into()),
    };
    Ok(FileDiff {
        path: file.path.clone(),
        original,
        modified,
        original_label: if wants_before {
            format!("{before_path} at {}", short(base))
        } else {
            "not there before".into()
        },
        modified_label: if wants_after {
            format!("{} at {}", file.path, short(head))
        } else {
            "deleted".into()
        },
        binary,
    })
}

/// Comment on a pull request, or reply to one of its threads.
///
/// `thread` is a [`PullThread::id`]. Markdown on both forges, which is what
/// both of them store.
pub fn pull_comment(
    stored: &Stored,
    repo: &str,
    id: &str,
    body: &str,
    thread: Option<&str>,
) -> Result<(), String> {
    if body.trim().is_empty() {
        return Err("that comment is empty".into());
    }
    let (source, token) = connect(stored, id)?;
    if let Some(t) = thread
        && !is_number(t)
    {
        return Err(format!("`{t}` is not a thread"));
    }
    match source.kind {
        Kind::GitHub => {
            let (owner, name) = github_repo(repo)?;
            let n = number(id)?;
            let url = match thread {
                Some(t) => {
                    format!(
                        "https://api.github.com/repos/{owner}/{name}/pulls/{n}/comments/{t}/replies"
                    )
                }
                // A pull request is an issue to GitHub, and its conversation is
                // the issue's comments.
                None => format!("https://api.github.com/repos/{owner}/{name}/issues/{n}/comments"),
            };
            send(
                "POST",
                &url,
                &format!("Bearer {token}"),
                Some(&json!({ "body": body })),
            )?;
        }
        Kind::AzureDevOps => {
            let auth = azure_auth(token);
            let pr = azure_pull(source, &auth, id)?;
            let at = AzureAt::of(source, &pr)?;
            azure_comment(&at, &auth, id, body, thread)?;
        }
        Kind::Jira => unreachable!("connect refuses Jira"),
    }
    Ok(())
}

/// Give a review: approve, ask for changes, or one of Azure DevOps' votes.
///
/// `body` goes with it. GitHub refuses a request for changes without one,
/// and on Azure DevOps it is posted as a comment beside the vote.
pub fn pull_vote(
    stored: &Stored,
    repo: &str,
    id: &str,
    verdict: PullVerdict,
    body: &str,
) -> Result<(), String> {
    let (source, token) = connect(stored, id)?;
    let body = body.trim();
    match source.kind {
        Kind::GitHub => {
            let (owner, name) = github_repo(repo)?;
            let event = match verdict {
                PullVerdict::Approved => "APPROVE",
                PullVerdict::ChangesRequested => {
                    if body.is_empty() {
                        return Err("say what should change: GitHub wants a comment with a request for changes".into());
                    }
                    "REQUEST_CHANGES"
                }
                other => return Err(format!("GitHub has no `{}` review", verdict_word(other))),
            };
            let mut review = json!({ "event": event });
            if !body.is_empty() {
                review["body"] = json!(body);
            }
            send(
                "POST",
                &format!(
                    "https://api.github.com/repos/{owner}/{name}/pulls/{}/reviews",
                    number(id)?
                ),
                &format!("Bearer {token}"),
                Some(&review),
            )?;
        }
        Kind::AzureDevOps => {
            let vote = match verdict {
                PullVerdict::Approved => 10,
                PullVerdict::Suggestions => 5,
                PullVerdict::Pending => 0,
                PullVerdict::Waiting => -5,
                PullVerdict::Rejected => -10,
                other => {
                    return Err(format!(
                        "Azure DevOps has no `{}` vote",
                        verdict_word(other)
                    ));
                }
            };
            let me = azure_me(source, token)?;
            let auth = azure_auth(token);
            let pr = azure_pull(source, &auth, id)?;
            let at = AzureAt::of(source, &pr)?;
            if !body.is_empty() {
                azure_comment(&at, &auth, id, body, None)?;
            }
            // Adds you as a reviewer if you were not one, which is what voting
            // on Azure DevOps' own page does too.
            send(
                "PUT",
                &at.pull_url(id, &format!("reviewers/{}", me.id), ""),
                &auth,
                Some(&json!({ "vote": vote })),
            )?;
        }
        Kind::Jira => unreachable!("connect refuses Jira"),
    }
    Ok(())
}

// ------------------------------------------------------------------ shared

/// The tracker and its token, for a pull request numbered `id`.
fn connect<'a>(stored: &'a Stored, id: &str) -> Result<(&'a Source, &'a str), String> {
    number(id)?;
    let token = token_of(stored)?;
    Ok((&stored.source, token))
}

fn token_of(stored: &Stored) -> Result<&str, String> {
    let source = &stored.source;
    if source.kind == Kind::Jira {
        return Err("Jira has no pull requests".into());
    }
    if let Some(problem) = source.problem() {
        return Err(problem);
    }
    stored
        .token
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| format!("{}: no token yet", source.name))
}

/// A pull request's number. It goes into URL paths, so it is digits and
/// nothing else.
fn number(id: &str) -> Result<u64, String> {
    if !is_number(id) {
        return Err(format!("`{id}` is not a pull request number"));
    }
    id.parse()
        .map_err(|_| format!("`{id}` is not a pull request number"))
}

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.len() <= 12 && s.bytes().all(|b| b.is_ascii_digit())
}

/// A commit, as a forge names one.
fn is_sha(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_guid(s: &str) -> bool {
    s.len() == 36 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

/// `owner/name`, checked, since both go into URL paths.
fn github_repo(repo: &str) -> Result<(&str, &str), String> {
    let ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && s != "."
            && s != ".."
    };
    match repo.split_once('/') {
        Some((owner, name)) if ok(owner) && ok(name) => Ok((owner, name)),
        _ => Err(format!("`{repo}` is not a GitHub repository")),
    }
}

fn verdict_word(v: PullVerdict) -> &'static str {
    match v {
        PullVerdict::Approved => "approve",
        PullVerdict::Suggestions => "approve with suggestions",
        PullVerdict::ChangesRequested => "request changes",
        PullVerdict::Waiting => "wait for author",
        PullVerdict::Rejected => "reject",
        PullVerdict::Commented => "comment",
        PullVerdict::Dismissed => "dismiss",
        PullVerdict::Pending => "reset",
    }
}

fn u32_of(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(Value::as_u64).map(|n| n as u32)
}

fn text_of(v: &Value, key: &str) -> Option<String> {
    Some(string(v, key)).filter(|s| !s.is_empty())
}

// ------------------------------------------------------------------ GitHub

/// The pull request and everything said about it, in one request.
///
/// `last` on the conversation, because the newest of a long one is what is
/// worth reading here; the oldest are a click away on GitHub.
const GITHUB_PULL: &str = r#"query($owner: String!, $name: String!, $number: Int!) {
  viewer { login }
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      number title url body state isDraft mergeable
      createdAt updatedAt headRefName baseRefName isCrossRepository viewerDidAuthor
      additions deletions changedFiles
      repository { nameWithOwner }
      author { login ... on User { name } }
      reviewRequests(first: 30) { nodes { asCodeOwner requestedReviewer { __typename ... on User { login name } ... on Team { name } ... on Bot { login } ... on Mannequin { login } } } }
      latestReviews(first: 30) { nodes { state author { login ... on User { name } } } }
      commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes {
        __typename
        ... on CheckRun { name status conclusion detailsUrl isRequired(pullRequestNumber: $number) checkSuite { workflowRun { workflow { name } } } }
        ... on StatusContext { context state targetUrl description isRequired(pullRequestNumber: $number) }
      } } } } } }
      comments(last: 100) { totalCount nodes { databaseId author { login ... on User { name } } body createdAt } }
      reviews(last: 100) { totalCount nodes { state body submittedAt author { login ... on User { name } } } }
      reviewThreads(last: 100) { totalCount nodes { isResolved isOutdated path line originalLine viewerCanReply
        comments(first: 50) { nodes { databaseId author { login ... on User { name } } body createdAt } } } }
    }
  }
}"#;

/// Both sides of one file. `isTruncated` is a file too large for GraphQL to
/// hand over whole, which is shown the way a binary one is.
const GITHUB_BLOBS: &str = r#"query($owner: String!, $name: String!, $before: String!, $after: String!) {
  repository(owner: $owner, name: $name) {
    before: object(expression: $before) { ... on Blob { text isBinary isTruncated } }
    after: object(expression: $after) { ... on Blob { text isBinary isTruncated } }
  }
}"#;

/// One GraphQL request, and its `data`, or what GitHub objected to.
fn graphql(token: &str, query: &str, variables: Value) -> Result<Value, String> {
    let body = json!({ "query": query, "variables": variables });
    let answer = post(
        "https://api.github.com/graphql",
        &format!("Bearer {token}"),
        &body.to_string(),
        JSON,
    )?;
    if let Some(errors) = answer.get("errors").and_then(Value::as_array)
        && !errors.is_empty()
    {
        let said: Vec<String> = errors
            .iter()
            .map(|e| string(e, "message"))
            .filter(|m| !m.is_empty())
            .collect();
        return Err(format!("github: {}", said.join("; ")));
    }
    answer
        .get("data")
        .cloned()
        .ok_or_else(|| "github answered with no data".to_string())
}

/// A GitHub account as a person: their name when they have given one, and
/// their login when not.
fn github_person(actor: &Value, viewer: &str) -> Option<Person> {
    if actor.is_null() {
        return None;
    }
    let login = string(actor, "login");
    let name = text_of(actor, "name").unwrap_or_else(|| login.clone());
    if name.is_empty() {
        return None;
    }
    Some(Person {
        name,
        me: !login.is_empty() && login.eq_ignore_ascii_case(viewer),
    })
}

fn github_verdict(state: &str) -> Option<PullVerdict> {
    Some(match state {
        "APPROVED" => PullVerdict::Approved,
        "CHANGES_REQUESTED" => PullVerdict::ChangesRequested,
        "COMMENTED" => PullVerdict::Commented,
        "DISMISSED" => PullVerdict::Dismissed,
        // `PENDING` is a review still being written, which only its author
        // can see and nobody has been given.
        _ => return None,
    })
}

fn nodes<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(|c| c.get("nodes"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn total(v: &Value, key: &str) -> u32 {
    v.get(key)
        .and_then(|c| u32_of(c, "totalCount"))
        .unwrap_or(0)
}

fn parse_github(data: &Value, source: &Source) -> Result<PullDetail, String> {
    let viewer = data
        .get("viewer")
        .map(|v| string(v, "login"))
        .unwrap_or_default();
    let p = data
        .get("repository")
        .and_then(|r| r.get("pullRequest"))
        .filter(|p| !p.is_null())
        .ok_or("github has no such pull request, or this token cannot see it")?;
    let n = p
        .get("number")
        .and_then(Value::as_u64)
        .ok_or("github did not number the pull request")?;
    let state = match string(p, "state").as_str() {
        "MERGED" => PullState::Merged,
        "CLOSED" => PullState::Closed,
        _ => PullState::Open,
    };
    let mine = p
        .get("viewerDidAuthor")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let contexts = p
        .get("commits")
        .and_then(|c| c.get("nodes"))
        .and_then(Value::as_array)
        .and_then(|n| n.last())
        .and_then(|n| n.get("commit"))
        .and_then(|c| c.get("statusCheckRollup"))
        .filter(|r| !r.is_null())
        .map(|r| nodes(r, "contexts"))
        .unwrap_or_default();
    let checks = contexts.iter().filter_map(github_check).collect();

    let mut reviewers: Vec<PullReviewer> = Vec::new();
    let mut my_vote = None;
    for r in nodes(p, "latestReviews") {
        let Some(verdict) = github_verdict(&string(r, "state")) else {
            continue;
        };
        let Some(who) = r.get("author").and_then(|a| github_person(a, &viewer)) else {
            continue;
        };
        if who.me {
            my_vote = Some(verdict);
        }
        reviewers.push(PullReviewer {
            who,
            verdict,
            required: false,
            group: false,
        });
    }
    for r in nodes(p, "reviewRequests") {
        // A team this token cannot see comes back as `null`, which is still
        // somebody being waited on.
        let asked = r.get("requestedReviewer").unwrap_or(&Value::Null);
        let group = string(asked, "__typename") == "Team" || asked.is_null();
        let who = if asked.is_null() {
            Person {
                name: "a team".into(),
                me: false,
            }
        } else {
            match github_person(asked, &viewer) {
                Some(who) => who,
                None => continue,
            }
        };
        let required = r
            .get("asCodeOwner")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Asked again after reviewing: the old verdict no longer stands.
        if let Some(existing) = reviewers.iter_mut().find(|e| e.who == who && !group) {
            existing.verdict = PullVerdict::Pending;
            existing.required |= required;
            continue;
        }
        if who.me {
            my_vote = Some(PullVerdict::Pending);
        }
        reviewers.push(PullReviewer {
            who,
            verdict: PullVerdict::Pending,
            required,
            group,
        });
    }

    let comment = |c: &Value| PullComment {
        author: c.get("author").and_then(|a| github_person(a, &viewer)),
        body: string(c, "body"),
        created: text_of(c, "createdAt"),
    };
    let mut conversation: Vec<PullThread> = Vec::new();
    for c in nodes(p, "comments") {
        let c = comment(c);
        conversation.push(PullThread {
            id: String::new(),
            path: None,
            line: None,
            resolved: false,
            outdated: false,
            verdict: None,
            author: c.author.clone(),
            at: c.created.clone(),
            comments: vec![c],
            can_reply: false,
        });
    }
    for r in nodes(p, "reviews") {
        let state = string(r, "state");
        let body = string(r, "body");
        // A review that only commented, with nothing of its own, is the
        // envelope its threads came in: the threads are below.
        if (state == "COMMENTED" || state == "PENDING") && body.trim().is_empty() {
            continue;
        }
        let author = r.get("author").and_then(|a| github_person(a, &viewer));
        let at = text_of(r, "submittedAt");
        conversation.push(PullThread {
            id: String::new(),
            path: None,
            line: None,
            resolved: false,
            outdated: false,
            verdict: github_verdict(&state),
            author: author.clone(),
            at: at.clone(),
            comments: if body.trim().is_empty() {
                Vec::new()
            } else {
                vec![PullComment {
                    author,
                    body,
                    created: at,
                }]
            },
            can_reply: false,
        });
    }
    for t in nodes(p, "reviewThreads") {
        let comments: Vec<&Value> = nodes(t, "comments").iter().collect();
        let Some(first) = comments.first() else {
            continue;
        };
        let first_id = first
            .get("databaseId")
            .and_then(Value::as_u64)
            .map(|id| id.to_string())
            .unwrap_or_default();
        let comments: Vec<PullComment> = comments.into_iter().map(comment).collect();
        conversation.push(PullThread {
            can_reply: !first_id.is_empty()
                && t.get("viewerCanReply")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            id: first_id,
            path: text_of(t, "path"),
            line: u32_of(t, "line").or_else(|| u32_of(t, "originalLine")),
            resolved: t
                .get("isResolved")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            outdated: t
                .get("isOutdated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            verdict: None,
            author: comments[0].author.clone(),
            at: comments[0].created.clone(),
            comments,
        });
    }
    // RFC 3339 in UTC throughout, so the text sorts as the time does.
    conversation.sort_by(|a, b| a.at.cmp(&b.at));
    // Only the lists GitHub cut short. Reviews with nothing in them were left
    // out on purpose, so they are not "older".
    let older = [("comments", 100), ("reviewThreads", 100)]
        .iter()
        .map(|(k, cap)| total(p, k).saturating_sub(*cap))
        .sum::<u32>();

    let open = state == PullState::Open;
    Ok(PullDetail {
        tracker: source.name.clone(),
        kind: Kind::GitHub,
        id: n.to_string(),
        key: format!("#{n}"),
        repo: p
            .get("repository")
            .map(|r| string(r, "nameWithOwner"))
            .unwrap_or_default(),
        title: string(p, "title"),
        url: string(p, "url"),
        state,
        draft: p.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        conflicts: string(p, "mergeable") == "CONFLICTING",
        fork: p
            .get("isCrossRepository")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        author: p.get("author").and_then(|a| github_person(a, &viewer)),
        branch: string(p, "headRefName"),
        base: string(p, "baseRefName"),
        created: text_of(p, "createdAt"),
        updated: text_of(p, "updatedAt"),
        body: string(p, "body"),
        additions: u32_of(p, "additions"),
        deletions: u32_of(p, "deletions"),
        changed_files: u32_of(p, "changedFiles"),
        checks,
        reviewers,
        conversation,
        older,
        offers: if open && !mine {
            vec![PullVerdict::Approved, PullVerdict::ChangesRequested]
        } else {
            Vec::new()
        },
        my_vote,
        mine,
        warnings: Vec::new(),
    })
}

/// One of the contexts in a commit's check rollup: an Actions job or another
/// app's check run, or a commit status from something older.
fn github_check(c: &Value) -> Option<PullCheck> {
    let required = c
        .get("isRequired")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match string(c, "__typename").as_str() {
        "CheckRun" => {
            let name = string(c, "name");
            // `CI / test`, the way GitHub's own list names a job, because
            // two workflows can each have a job called `test`.
            let workflow = c
                .get("checkSuite")
                .and_then(|s| s.get("workflowRun"))
                .and_then(|r| r.get("workflow"))
                .map(|w| string(w, "name"))
                .filter(|w| !w.is_empty());
            let state = if string(c, "status") != "COMPLETED" {
                PullCheckState::Pending
            } else {
                match string(c, "conclusion").as_str() {
                    "SUCCESS" => PullCheckState::Passing,
                    "SKIPPED" => PullCheckState::Skipped,
                    "NEUTRAL" | "STALE" | "CANCELLED" => PullCheckState::Neutral,
                    _ => PullCheckState::Failing,
                }
            };
            Some(PullCheck {
                name: match workflow {
                    Some(w) => format!("{w} / {name}"),
                    None => name,
                },
                state,
                url: text_of(c, "detailsUrl"),
                detail: None,
                required,
            })
        }
        "StatusContext" => Some(PullCheck {
            name: string(c, "context"),
            state: match string(c, "state").as_str() {
                "SUCCESS" => PullCheckState::Passing,
                "PENDING" | "EXPECTED" => PullCheckState::Pending,
                _ => PullCheckState::Failing,
            },
            url: text_of(c, "targetUrl"),
            detail: text_of(c, "description"),
            required,
        }),
        _ => None,
    }
}

/// GitHub's comparison of the two commits, as a list of files.
fn parse_github_files(
    compare: &Value,
    repo: &str,
    changed: Option<u64>,
) -> Result<PullFiles, String> {
    let base = compare
        .get("merge_base_commit")
        .map(|c| string(c, "sha"))
        .filter(|s| is_sha(s))
        .ok_or("github did not say where the branch left its base")?;
    let head = compare
        .get("commits")
        .and_then(Value::as_array)
        .and_then(|c| c.last())
        .map(|c| string(c, "sha"))
        .filter(|s| is_sha(s))
        .ok_or("github did not say which commit the branch is at")?;
    let files: Vec<PullFile> = compare
        .get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(|f| {
                    let path = text_of(f, "filename")?;
                    let change = match string(f, "status").as_str() {
                        "added" => Change::Added,
                        "removed" => Change::Deleted,
                        "renamed" => Change::Renamed,
                        "unchanged" => return None,
                        _ => Change::Modified,
                    };
                    Some(PullFile {
                        old_path: text_of(f, "previous_filename")
                            .filter(|_| change == Change::Renamed),
                        path,
                        change,
                        additions: u32_of(f, "additions"),
                        deletions: u32_of(f, "deletions"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(PullFiles {
        repo: repo.to_string(),
        truncated: changed.is_some_and(|c| c > files.len() as u64),
        base,
        head,
        files,
    })
}

// ---------------------------------------------------------- Azure DevOps

/// Where one pull request lives on Azure DevOps: the organisation and project
/// the tracker names, and the repository and project ids its own record
/// gives, which is what the per-repository requests are addressed by.
struct AzureAt {
    org: String,
    project: String,
    project_id: String,
    repo: String,
}

impl AzureAt {
    fn of(source: &Source, pr: &Value) -> Result<AzureAt, String> {
        let repository = pr
            .get("repository")
            .ok_or("azure devops did not say which repository")?;
        let repo = string(repository, "id");
        if !is_guid(&repo) {
            return Err("azure devops did not say which repository".into());
        }
        let project_id = repository
            .get("project")
            .map(|p| string(p, "id"))
            .filter(|p| is_guid(p))
            .unwrap_or_default();
        Ok(AzureAt {
            org: source.org.clone().unwrap_or_default(),
            project: source.project.clone().unwrap_or_default(),
            project_id,
            repo,
        })
    }

    /// `…/pullRequests/{id}/{what}`, with `extra` query parameters after the
    /// API version.
    fn pull_url(&self, id: &str, what: &str, extra: &str) -> String {
        format!(
            "https://dev.azure.com/{}/{}/_apis/git/repositories/{}/pullRequests/{id}/{what}?api-version=7.1{extra}",
            self.org, self.project, self.repo
        )
    }

    /// The branch policies' verdicts on this pull request: builds, required
    /// reviewers, linked work items. Addressed by an artifact id that names
    /// the project by its id.
    fn evaluations_url(&self, id: &str) -> String {
        let artifact = format!("vstfs:///CodeReview/CodeReviewId/{}/{id}", self.project_id);
        format!(
            "https://dev.azure.com/{}/{}/_apis/policy/evaluations?artifactId={}&api-version=7.1-preview.1",
            self.org,
            self.project,
            super::urlencode(&artifact)
        )
    }

    fn item_url(&self, path: &str, commit: &str) -> String {
        format!(
            "https://dev.azure.com/{}/{}/_apis/git/repositories/{}/items?path={}&versionDescriptor.version={commit}&versionDescriptor.versionType=commit&includeContent=true&$format=json&api-version=7.1",
            self.org,
            self.project,
            self.repo,
            super::urlencode(path)
        )
    }
}

/// The pull request's own record, by its number. Asked of the project rather
/// than a repository, because the sidebar knows the number and the
/// repository's name, and every other request wants the repository's id.
fn azure_pull(source: &Source, auth: &str, id: &str) -> Result<Value, String> {
    let org = source.org.as_deref().unwrap_or_default();
    let project = source.project.as_deref().unwrap_or_default();
    get(
        &format!(
            "https://dev.azure.com/{org}/{project}/_apis/git/pullrequests/{id}?api-version=7.1"
        ),
        auth,
        &[],
    )
    .map_err(|e| super::pulls::azure_refusal(&e))
}

fn azure_comment(
    at: &AzureAt,
    auth: &str,
    id: &str,
    body: &str,
    thread: Option<&str>,
) -> Result<(), String> {
    match thread {
        // Under the thread's first comment, which is what Azure DevOps' own
        // reply box does.
        Some(t) => send(
            "POST",
            &at.pull_url(id, &format!("threads/{t}/comments"), ""),
            auth,
            Some(&json!({ "content": body, "parentCommentId": 1, "commentType": 1 })),
        )?,
        None => send(
            "POST",
            &at.pull_url(id, "threads", ""),
            auth,
            Some(&json!({
                "comments": [{ "content": body, "parentCommentId": 0, "commentType": 1 }],
                "status": 1,
            })),
        )?,
    };
    Ok(())
}

fn azure_verdict(vote: i64) -> PullVerdict {
    match vote {
        v if v >= 10 => PullVerdict::Approved,
        v if v > 0 => PullVerdict::Suggestions,
        0 => PullVerdict::Pending,
        v if v > -10 => PullVerdict::Waiting,
        _ => PullVerdict::Rejected,
    }
}

fn azure_person(identity: &Value, me: &AzureMe) -> Option<Person> {
    let name = text_of(identity, "displayName")?;
    Some(Person {
        name,
        me: me.is(identity),
    })
}

/// A field Azure DevOps answers with as a name over REST and as a number from
/// the SDKs that wrap it, as both: `("active", 1)`.
fn azure_enum(v: &Value, key: &str) -> (String, Option<i64>) {
    match v.get(key) {
        Some(Value::String(s)) => (s.to_ascii_lowercase(), None),
        Some(Value::Number(n)) => (String::new(), n.as_i64()),
        _ => (String::new(), None),
    }
}

fn parse_azure(
    pr: &Value,
    threads: &Value,
    statuses: Option<&Value>,
    evaluations: Option<&Value>,
    source: &Source,
    me: &AzureMe,
) -> Result<PullDetail, String> {
    let id = pr
        .get("pullRequestId")
        .and_then(Value::as_u64)
        .ok_or("azure devops did not number the pull request")?;
    let repository = pr.get("repository").unwrap_or(&Value::Null);
    let repo = string(repository, "name");
    let repo_url = text_of(repository, "webUrl").unwrap_or_else(|| {
        format!(
            "https://dev.azure.com/{}/{}/_git/{repo}",
            source.org.as_deref().unwrap_or_default(),
            source.project.as_deref().unwrap_or_default()
        )
    });
    let state = match azure_enum(pr, "status") {
        (s, _) if s == "completed" => PullState::Merged,
        (_, Some(3)) => PullState::Merged,
        (s, _) if s == "abandoned" => PullState::Closed,
        (_, Some(2)) => PullState::Closed,
        _ => PullState::Open,
    };
    let conflicts = match pr.get("mergeStatus") {
        Some(Value::String(s)) => s == "conflicts",
        Some(Value::Number(n)) => n.as_u64() == Some(2),
        _ => false,
    };
    let refs = |key: &str| {
        let name = string(pr, key);
        name.strip_prefix("refs/heads/")
            .map(str::to_string)
            .unwrap_or(name)
    };
    let author = pr.get("createdBy").and_then(|c| azure_person(c, me));
    let mine = author.as_ref().is_some_and(|a| a.me);

    let mut my_vote = None;
    let reviewers = pr
        .get("reviewers")
        .and_then(Value::as_array)
        .map(|rs| {
            rs.iter()
                .filter_map(|r| {
                    let who = azure_person(r, me)?;
                    let verdict = azure_verdict(r.get("vote").and_then(Value::as_i64).unwrap_or(0));
                    if who.me {
                        my_vote = Some(verdict);
                    }
                    Some(PullReviewer {
                        who,
                        verdict,
                        required: r
                            .get("isRequired")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        group: r
                            .get("isContainer")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let mut conversation = Vec::new();
    for t in threads
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if t.get("isDeleted").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        if let Some(entry) = azure_thread(t, me) {
            conversation.push(entry);
        }
    }
    conversation.sort_by(|a, b| a.at.cmp(&b.at));

    let mut checks: Vec<PullCheck> = Vec::new();
    if let Some(evaluations) = evaluations {
        checks.extend(azure_evaluations(evaluations, source));
    }
    if let Some(statuses) = statuses {
        checks.extend(azure_statuses(statuses));
    }

    let open = state == PullState::Open;
    Ok(PullDetail {
        tracker: source.name.clone(),
        kind: Kind::AzureDevOps,
        id: id.to_string(),
        key: format!("!{id}"),
        url: format!("{repo_url}/pullrequest/{id}"),
        repo,
        title: string(pr, "title"),
        state,
        draft: pr.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        conflicts,
        fork: pr.get("forkSource").is_some_and(|f| !f.is_null()),
        author,
        branch: refs("sourceRefName"),
        base: refs("targetRefName"),
        created: text_of(pr, "creationDate"),
        updated: text_of(pr, "closedDate"),
        body: string(pr, "description"),
        additions: None,
        deletions: None,
        changed_files: None,
        checks,
        reviewers,
        conversation,
        older: 0,
        offers: if open {
            vec![
                PullVerdict::Approved,
                PullVerdict::Suggestions,
                PullVerdict::Waiting,
                PullVerdict::Rejected,
                PullVerdict::Pending,
            ]
        } else {
            Vec::new()
        },
        my_vote,
        mine,
        warnings: Vec::new(),
    })
}

/// One thread, as an entry in the conversation, or `None` for the notices
/// Azure DevOps keeps there: a reviewer added, a branch updated. A vote is
/// kept, as the review it is.
fn azure_thread(t: &Value, me: &AzureMe) -> Option<PullThread> {
    let comments: Vec<&Value> = t
        .get("comments")
        .and_then(Value::as_array)?
        .iter()
        .filter(|c| !c.get("isDeleted").and_then(Value::as_bool).unwrap_or(false))
        .collect();
    let system = |c: &Value| match azure_enum(c, "commentType") {
        (s, _) if s == "system" => true,
        (_, Some(3)) => true,
        _ => false,
    };
    let at = text_of(t, "publishedDate");
    let kind = t
        .get("properties")
        .and_then(|p| p.get("CodeReviewThreadType"))
        .and_then(|k| k.get("$value"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind == "VoteUpdate" {
        let vote = t
            .get("properties")
            .and_then(|p| p.get("CodeReviewVoteResult"))
            .and_then(|v| v.get("$value"))
            .and_then(|v| match v {
                Value::String(s) => s.parse::<i64>().ok(),
                other => other.as_i64(),
            })?;
        // The voter is in the thread's identities, by the number the
        // property names; the comment's own author is the service.
        let voter = t
            .get("properties")
            .and_then(|p| p.get("CodeReviewVotedByIdentity"))
            .and_then(|v| v.get("$value"))
            .and_then(Value::as_str)
            .and_then(|n| t.get("identities").and_then(|i| i.get(n)))
            .and_then(|i| azure_person(i, me));
        return Some(PullThread {
            id: String::new(),
            path: None,
            line: None,
            resolved: false,
            outdated: false,
            verdict: Some(azure_verdict(vote)),
            author: voter,
            at,
            comments: Vec::new(),
            can_reply: false,
        });
    }
    let comments: Vec<PullComment> = comments
        .into_iter()
        .filter(|c| !system(c))
        .map(|c| PullComment {
            author: c.get("author").and_then(|a| azure_person(a, me)),
            body: string(c, "content"),
            created: text_of(c, "publishedDate"),
        })
        .collect();
    if comments.is_empty() {
        return None;
    }
    let context = t
        .get("threadContext")
        .filter(|c| !c.is_null())
        .or_else(|| t.get("pullRequestThreadContext").filter(|c| !c.is_null()));
    let path = context.and_then(|c| text_of(c, "filePath"));
    let line = context.and_then(|c| {
        c.get("rightFileStart")
            .filter(|s| !s.is_null())
            .or_else(|| c.get("leftFileStart").filter(|s| !s.is_null()))
            .and_then(|s| u32_of(s, "line"))
    });
    // `active` and `pending` are open; `fixed`, `wontFix`, `closed` and
    // `byDesign` are each a way of being done with it.
    let resolved = match azure_enum(t, "status") {
        (s, _) if !s.is_empty() => !matches!(s.as_str(), "active" | "pending" | "unknown"),
        (_, Some(n)) => (2..=5).contains(&n),
        _ => false,
    };
    let id = t.get("id").and_then(Value::as_u64)?.to_string();
    Some(PullThread {
        id,
        path,
        line,
        resolved,
        outdated: false,
        verdict: None,
        author: comments[0].author.clone(),
        at: comments[0].created.clone().or(at),
        comments,
        can_reply: true,
    })
}

/// The branch policies, as checks. A policy with nothing to say about this
/// pull request is left out rather than shown as skipped: a project's
/// policies cover every branch, and most of them are about other ones.
fn azure_evaluations(body: &Value, source: &Source) -> Vec<PullCheck> {
    body.get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let config = e.get("configuration")?;
            if config.get("isEnabled").and_then(Value::as_bool) == Some(false) {
                return None;
            }
            let kind = config
                .get("type")
                .map(|t| string(t, "displayName"))
                .unwrap_or_default();
            let named = config
                .get("settings")
                .map(|s| string(s, "displayName"))
                .filter(|n| !n.is_empty());
            let state = match string(e, "status").as_str() {
                "approved" => PullCheckState::Passing,
                "rejected" | "broken" => PullCheckState::Failing,
                "running" | "queued" => PullCheckState::Pending,
                _ => return None,
            };
            let build = e
                .get("context")
                .and_then(|c| c.get("buildId"))
                .and_then(Value::as_u64);
            Some(PullCheck {
                name: match named {
                    Some(n) if !kind.is_empty() && n != kind => format!("{kind} / {n}"),
                    Some(n) => n,
                    None => kind,
                },
                state,
                url: build.map(|b| {
                    format!(
                        "https://dev.azure.com/{}/{}/_build/results?buildId={b}",
                        source.org.as_deref().unwrap_or_default(),
                        source.project.as_deref().unwrap_or_default()
                    )
                }),
                detail: None,
                required: config
                    .get("isBlocking")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// Statuses other services posted: one per push each, so only the newest of
/// each name is the one that stands.
fn azure_statuses(body: &Value) -> Vec<PullCheck> {
    let mut newest: Vec<(String, u64, PullCheck)> = Vec::new();
    for s in body
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(context) = s.get("context") else {
            continue;
        };
        let name = [string(context, "genre"), string(context, "name")]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        if name.is_empty() {
            continue;
        }
        let state = match azure_enum(s, "state") {
            (s, _) if s == "succeeded" => PullCheckState::Passing,
            (s, _) if s == "failed" || s == "error" => PullCheckState::Failing,
            (s, _) if s == "pending" => PullCheckState::Pending,
            (s, _) if s == "notapplicable" => PullCheckState::Skipped,
            (_, Some(2)) => PullCheckState::Passing,
            (_, Some(3) | Some(4)) => PullCheckState::Failing,
            (_, Some(1)) => PullCheckState::Pending,
            _ => PullCheckState::Neutral,
        };
        let id = s.get("id").and_then(Value::as_u64).unwrap_or(0);
        let check = PullCheck {
            name: name.clone(),
            state,
            url: text_of(s, "targetUrl"),
            detail: text_of(s, "description"),
            required: false,
        };
        match newest.iter_mut().find(|(n, _, _)| *n == name) {
            Some(entry) if entry.1 < id => *entry = (name, id, check),
            Some(_) => {}
            None => newest.push((name, id, check)),
        }
    }
    newest.into_iter().map(|(_, _, c)| c).collect()
}

/// The last push's changes, measured from where the branch left its base.
fn parse_azure_files(iteration: &Value, changes: &Value, repo: &str) -> Result<PullFiles, String> {
    let commit = |key: &str| {
        iteration
            .get(key)
            .map(|c| string(c, "commitId"))
            .filter(|s| is_sha(s))
    };
    let base = commit("commonRefCommit")
        .or_else(|| commit("targetRefCommit"))
        .ok_or("azure devops did not say where the branch left its base")?;
    let head = commit("sourceRefCommit")
        .ok_or("azure devops did not say which commit the branch is at")?;
    let entries = changes
        .get("changeEntries")
        .and_then(Value::as_array)
        .ok_or("azure devops did not list the changes")?;
    let files = entries
        .iter()
        .filter_map(|c| {
            let item = c.get("item")?;
            if item
                .get("isFolder")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || string(item, "gitObjectType") == "tree"
            {
                return None;
            }
            let path = text_of(item, "path")?;
            let (kind, flags) = azure_enum(c, "changeType");
            // A set of flags: `edit, rename` is both.
            let has =
                |word: &str, bit: i64| kind.contains(word) || flags.is_some_and(|f| f & bit != 0);
            let change = if has("delete", 16) {
                Change::Deleted
            } else if has("add", 1) {
                Change::Added
            } else if has("rename", 8) {
                Change::Renamed
            } else {
                Change::Modified
            };
            Some(PullFile {
                old_path: text_of(c, "originalPath")
                    .or_else(|| text_of(c, "sourceServerItem"))
                    .filter(|o| change == Change::Renamed && *o != path),
                path,
                change,
                additions: None,
                deletions: None,
            })
        })
        .collect();
    Ok(PullFiles {
        repo: repo.to_string(),
        base,
        head,
        files,
        truncated: changes.get("nextSkip").and_then(Value::as_u64).unwrap_or(0) > 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github_source() -> Source {
        Source {
            kind: Kind::GitHub,
            name: "github".into(),
            ..Default::default()
        }
    }

    fn azure_source() -> Source {
        Source {
            kind: Kind::AzureDevOps,
            name: "work".into(),
            org: Some("contoso".into()),
            project: Some("tools".into()),
            ..Default::default()
        }
    }

    fn me() -> AzureMe {
        AzureMe {
            id: "me-1".into(),
            account: Some("you@example.com".into()),
        }
    }

    /// GitHub's answer to [`GITHUB_PULL`], measured against real ones
    /// (`cli/cli#10513` for the threads, `cli/cli#14440` for an unseen team)
    /// and trimmed.
    const GITHUB_DETAIL: &str = r#"{
      "viewer": { "login": "tobias" },
      "repository": { "pullRequest": {
        "number": 61, "title": "Retry uploads", "url": "https://github.com/o/r/pull/61",
        "body": "Fixes the **retry**.", "state": "OPEN", "isDraft": false, "mergeable": "CONFLICTING",
        "createdAt": "2026-10-01T10:00:00Z", "updatedAt": "2026-10-03T10:00:00Z",
        "headRefName": "sam/retry", "baseRefName": "main", "isCrossRepository": false,
        "viewerDidAuthor": false, "additions": 40, "deletions": 3, "changedFiles": 2,
        "repository": { "nameWithOwner": "o/r" },
        "author": { "login": "sam", "name": "Sam Rivera" },
        "reviewRequests": { "nodes": [
          { "asCodeOwner": false, "requestedReviewer": { "__typename": "User", "login": "tobias", "name": null } },
          { "asCodeOwner": true, "requestedReviewer": null }
        ] },
        "latestReviews": { "nodes": [
          { "state": "CHANGES_REQUESTED", "author": { "login": "ada", "name": "Ada Lovelace" } },
          { "state": "COMMENTED", "author": { "login": "copilot-pull-request-reviewer" } }
        ] },
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "contexts": { "nodes": [
          { "__typename": "CheckRun", "name": "test", "status": "COMPLETED", "conclusion": "FAILURE",
            "detailsUrl": "https://github.com/o/r/actions/runs/1/job/2", "isRequired": true,
            "checkSuite": { "workflowRun": { "workflow": { "name": "CI" } } } },
          { "__typename": "CheckRun", "name": "lint", "status": "IN_PROGRESS", "conclusion": null,
            "detailsUrl": null, "isRequired": false, "checkSuite": { "workflowRun": null } },
          { "__typename": "StatusContext", "context": "netlify", "state": "SUCCESS",
            "targetUrl": "https://netlify.example/1", "description": "Deploy ready", "isRequired": false }
        ] } } } }] },
        "comments": { "totalCount": 1, "nodes": [
          { "databaseId": 900, "author": { "login": "tobias", "name": "Tobias" }, "body": "Looks close.", "createdAt": "2026-10-02T09:00:00Z" }
        ] },
        "reviews": { "totalCount": 3, "nodes": [
          { "state": "COMMENTED", "body": "", "submittedAt": "2026-10-01T12:00:00Z", "author": { "login": "ada", "name": "Ada Lovelace" } },
          { "state": "CHANGES_REQUESTED", "body": "One thing first.", "submittedAt": "2026-10-02T12:00:00Z", "author": { "login": "ada", "name": "Ada Lovelace" } }
        ] },
        "reviewThreads": { "totalCount": 1, "nodes": [
          { "isResolved": false, "isOutdated": true, "path": "src/upload.rs", "line": null, "originalLine": 42, "viewerCanReply": true,
            "comments": { "nodes": [
              { "databaseId": 1974078574, "author": { "login": "ada", "name": "Ada Lovelace" }, "body": "Why three?", "createdAt": "2026-10-01T12:00:00Z" },
              { "databaseId": 1974078575, "author": { "login": "sam", "name": "Sam Rivera" }, "body": "Measured.", "createdAt": "2026-10-01T13:00:00Z" }
            ] } }
        ] }
      } }
    }"#;

    fn github_detail() -> PullDetail {
        parse_github(
            &serde_json::from_str(GITHUB_DETAIL).unwrap(),
            &github_source(),
        )
        .unwrap()
    }

    #[test]
    fn a_github_pull_request_reads_its_state_and_sides() {
        let d = github_detail();
        assert_eq!(d.key, "#61");
        assert_eq!(d.repo, "o/r");
        assert_eq!(d.state, PullState::Open);
        assert!(d.conflicts);
        assert!(!d.fork);
        assert_eq!((d.branch.as_str(), d.base.as_str()), ("sam/retry", "main"));
        assert_eq!(
            d.author.as_ref().map(|a| a.name.as_str()),
            Some("Sam Rivera")
        );
        assert_eq!(
            (d.additions, d.deletions, d.changed_files),
            (Some(40), Some(3), Some(2))
        );
    }

    #[test]
    fn github_checks_are_named_after_their_workflow_and_say_how_they_went() {
        let d = github_detail();
        let names: Vec<_> = d
            .checks
            .iter()
            .map(|c| (c.name.as_str(), c.state))
            .collect();
        assert_eq!(
            names,
            [
                ("CI / test", PullCheckState::Failing),
                ("lint", PullCheckState::Pending),
                ("netlify", PullCheckState::Passing),
            ]
        );
        assert!(d.checks[0].required);
        assert_eq!(d.checks[2].detail.as_deref(), Some("Deploy ready"));
    }

    #[test]
    fn github_reviewers_are_who_reviewed_and_who_is_still_asked() {
        let d = github_detail();
        let who: Vec<_> = d
            .reviewers
            .iter()
            .map(|r| (r.who.name.as_str(), r.verdict, r.group))
            .collect();
        assert_eq!(
            who,
            [
                ("Ada Lovelace", PullVerdict::ChangesRequested, false),
                (
                    "copilot-pull-request-reviewer",
                    PullVerdict::Commented,
                    false
                ),
                ("tobias", PullVerdict::Pending, false),
                // A team this token cannot see is still a review being waited on.
                ("a team", PullVerdict::Pending, true),
            ]
        );
        assert!(d.reviewers[2].who.me);
        assert_eq!(d.my_vote, Some(PullVerdict::Pending));
        assert_eq!(
            d.offers,
            [PullVerdict::Approved, PullVerdict::ChangesRequested]
        );
    }

    #[test]
    fn the_github_conversation_is_in_time_order_without_empty_reviews() {
        let d = github_detail();
        let entries: Vec<_> = d
            .conversation
            .iter()
            .map(|t| {
                (
                    t.at.as_deref().unwrap_or(""),
                    t.path.as_deref(),
                    t.verdict,
                    t.comments.len(),
                )
            })
            .collect();
        assert_eq!(
            entries,
            [
                ("2026-10-01T12:00:00Z", Some("src/upload.rs"), None, 2),
                ("2026-10-02T09:00:00Z", None, None, 1),
                (
                    "2026-10-02T12:00:00Z",
                    None,
                    Some(PullVerdict::ChangesRequested),
                    1
                ),
            ]
        );
        let thread = &d.conversation[0];
        assert_eq!(
            thread.id, "1974078574",
            "a reply goes under the first comment"
        );
        assert_eq!(
            thread.line,
            Some(42),
            "an outdated thread keeps its original line"
        );
        assert!(thread.outdated && thread.can_reply && !thread.resolved);
        assert!(d.conversation[1].comments[0].author.as_ref().unwrap().me);
    }

    #[test]
    fn your_own_github_pull_request_offers_no_review() {
        let mut v: Value = serde_json::from_str(GITHUB_DETAIL).unwrap();
        v["repository"]["pullRequest"]["viewerDidAuthor"] = json!(true);
        let d = parse_github(&v, &github_source()).unwrap();
        assert!(d.mine);
        assert!(d.offers.is_empty(), "GitHub refuses a review of your own");
    }

    #[test]
    fn a_closed_github_pull_request_offers_no_review() {
        let mut v: Value = serde_json::from_str(GITHUB_DETAIL).unwrap();
        v["repository"]["pullRequest"]["state"] = json!("MERGED");
        let d = parse_github(&v, &github_source()).unwrap();
        assert_eq!(d.state, PullState::Merged);
        assert!(d.offers.is_empty());
    }

    #[test]
    fn a_github_pull_request_the_token_cannot_see_says_so() {
        let v = json!({ "viewer": { "login": "x" }, "repository": { "pullRequest": null } });
        assert!(
            parse_github(&v, &github_source())
                .unwrap_err()
                .contains("no such pull request")
        );
    }

    #[test]
    fn github_files_are_measured_from_the_merge_base() {
        let compare = json!({
            "merge_base_commit": { "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
            "commits": [{ "sha": "1111111" }, { "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" }],
            "files": [
                { "filename": "src/new.rs", "status": "added", "additions": 10, "deletions": 0 },
                { "filename": "src/b.rs", "status": "renamed", "previous_filename": "src/a.rs", "additions": 1, "deletions": 1 },
                { "filename": "old.txt", "status": "removed", "additions": 0, "deletions": 4 },
                { "filename": "x.rs", "status": "modified", "additions": 2, "deletions": 2 }
            ]
        });
        let f = parse_github_files(&compare, "o/r", Some(5)).unwrap();
        assert_eq!(f.base, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(f.head, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let changes: Vec<_> = f
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.change))
            .collect();
        assert_eq!(
            changes,
            [
                ("src/new.rs", Change::Added),
                ("src/b.rs", Change::Renamed),
                ("old.txt", Change::Deleted),
                ("x.rs", Change::Modified),
            ]
        );
        assert_eq!(f.files[1].old_path.as_deref(), Some("src/a.rs"));
        assert!(f.truncated, "five changed and four listed");
    }

    /// A pull request's own record, as `GET _apis/git/pullrequests/{id}`
    /// answers, trimmed.
    fn azure_pr() -> Value {
        json!({
            "pullRequestId": 26051,
            "status": "active",
            "isDraft": false,
            "mergeStatus": "succeeded",
            "title": "Use Grid for the usage tables",
            "description": "Swaps the tables for `Grid`.",
            "creationDate": "2026-10-29T17:04:48.703Z",
            "sourceRefName": "refs/heads/tobias/CODE-20391-grid",
            "targetRefName": "refs/heads/master",
            "createdBy": { "displayName": "Tobias Wadseth", "id": "me-1", "uniqueName": "you@example.com" },
            "repository": {
                "id": "b8066eee-5517-4eae-a310-4d2751785492",
                "name": "Inet.Server",
                "project": { "id": "21c5ec8d-f557-4535-a200-2f53c747e18f", "name": "inet" }
            },
            "reviewers": [
                { "displayName": "Josef Andersson Engman", "id": "j-1", "vote": 10, "isRequired": true },
                { "displayName": "[inet]\\Backend", "id": "g-1", "vote": 0, "isContainer": true }
            ]
        })
    }

    /// Threads as `…/threads` answers: a vote, a reviewer joining, and a
    /// thread on a line. The first two are measured against a real answer.
    fn azure_threads() -> Value {
        json!({ "value": [
            { "id": 111977, "publishedDate": "2026-10-30T10:27:48.643Z", "isDeleted": false,
              "comments": [{ "id": 1, "content": "Josef Andersson Engman joined as a reviewer", "commentType": "system",
                             "author": { "displayName": "Microsoft.VisualStudio.Services.TFS", "id": "00000002-0000-8888-8000-000000000000" } }],
              "properties": { "CodeReviewThreadType": { "$type": "System.String", "$value": "ReviewersUpdate" } } },
            { "id": 111978, "publishedDate": "2026-10-30T10:27:48.657Z", "isDeleted": false,
              "comments": [{ "id": 1, "content": "Josef Andersson Engman voted 10", "commentType": 3,
                             "author": { "displayName": "Josef Andersson Engman", "id": "j-1" } }],
              "properties": {
                  "CodeReviewThreadType": { "$type": "System.String", "$value": "VoteUpdate" },
                  "CodeReviewVoteResult": { "$type": "System.String", "$value": "10" },
                  "CodeReviewVotedByIdentity": { "$type": "System.String", "$value": "1" }
              },
              "identities": { "1": { "displayName": "Josef Andersson Engman", "id": "j-1" } } },
            { "id": 111990, "publishedDate": "2026-10-30T09:00:00Z", "status": "fixed", "isDeleted": false,
              "threadContext": { "filePath": "/src/Grid.cs", "rightFileStart": { "line": 12, "offset": 1 } },
              "comments": [
                  { "id": 1, "content": "Is this sorted?", "commentType": "text", "publishedDate": "2026-10-30T09:00:00Z",
                    "author": { "displayName": "Josef Andersson Engman", "id": "j-1" } },
                  { "id": 2, "content": "It is now.", "commentType": "text", "publishedDate": "2026-10-30T09:30:00Z",
                    "author": { "displayName": "Tobias Wadseth", "id": "me-1" } },
                  { "id": 3, "content": "gone", "commentType": "text", "isDeleted": true,
                    "author": { "displayName": "Tobias Wadseth", "id": "me-1" } }
              ] }
        ] })
    }

    fn azure_detail() -> PullDetail {
        let statuses = json!({ "value": [
            { "id": 1, "state": "pending", "context": { "genre": "sonar", "name": "quality" } },
            { "id": 2, "state": "succeeded", "description": "Passed", "context": { "genre": "sonar", "name": "quality" }, "targetUrl": "https://sonar.example/1" }
        ] });
        let evaluations = json!({ "value": [
            { "status": "rejected", "context": { "buildId": 777 },
              "configuration": { "isEnabled": true, "isBlocking": true,
                  "type": { "displayName": "Build" }, "settings": { "displayName": "PR build" } } },
            { "status": "approved",
              "configuration": { "isEnabled": true, "isBlocking": true,
                  "type": { "displayName": "Minimum number of reviewers" }, "settings": {} } },
            { "status": "notApplicable",
              "configuration": { "isEnabled": true, "isBlocking": false, "type": { "displayName": "Work item linking" } } }
        ] });
        parse_azure(
            &azure_pr(),
            &azure_threads(),
            Some(&statuses),
            Some(&evaluations),
            &azure_source(),
            &me(),
        )
        .unwrap()
    }

    #[test]
    fn an_azure_pull_request_reads_its_state_and_reviewers() {
        let d = azure_detail();
        assert_eq!(d.key, "!26051");
        assert_eq!(d.state, PullState::Open);
        assert!(d.mine);
        assert_eq!(
            (d.branch.as_str(), d.base.as_str()),
            ("tobias/CODE-20391-grid", "master")
        );
        assert_eq!(
            d.url,
            "https://dev.azure.com/contoso/tools/_git/Inet.Server/pullrequest/26051"
        );
        let who: Vec<_> = d
            .reviewers
            .iter()
            .map(|r| (r.who.name.as_str(), r.verdict, r.required, r.group))
            .collect();
        assert_eq!(
            who,
            [
                ("Josef Andersson Engman", PullVerdict::Approved, true, false),
                ("[inet]\\Backend", PullVerdict::Pending, false, true),
            ]
        );
        assert_eq!(d.my_vote, None, "the author is not among the reviewers");
        assert_eq!(d.offers.len(), 5, "Azure DevOps lets anybody vote");
    }

    #[test]
    fn an_azure_conversation_keeps_votes_and_comments_and_drops_notices() {
        let d = azure_detail();
        assert_eq!(d.conversation.len(), 2, "the reviewer joining is a notice");
        let thread = &d.conversation[0];
        assert_eq!(thread.id, "111990");
        assert_eq!(thread.path.as_deref(), Some("/src/Grid.cs"));
        assert_eq!(thread.line, Some(12));
        assert!(thread.resolved, "fixed is done with");
        assert_eq!(thread.comments.len(), 2, "a deleted comment is gone");
        assert!(thread.comments[1].author.as_ref().unwrap().me);
        let vote = &d.conversation[1];
        assert_eq!(vote.verdict, Some(PullVerdict::Approved));
        assert_eq!(
            vote.author.as_ref().map(|a| a.name.as_str()),
            Some("Josef Andersson Engman")
        );
        assert!(vote.comments.is_empty());
    }

    #[test]
    fn azure_checks_are_its_policies_and_the_newest_of_each_status() {
        let d = azure_detail();
        let checks: Vec<_> = d
            .checks
            .iter()
            .map(|c| (c.name.as_str(), c.state, c.required))
            .collect();
        assert_eq!(
            checks,
            [
                ("Build / PR build", PullCheckState::Failing, true),
                ("Minimum number of reviewers", PullCheckState::Passing, true),
                ("sonar/quality", PullCheckState::Passing, false),
            ]
        );
        assert_eq!(
            d.checks[0].url.as_deref(),
            Some("https://dev.azure.com/contoso/tools/_build/results?buildId=777")
        );
    }

    #[test]
    fn azure_threads_from_the_sdks_read_the_same() {
        // The SDKs send numbers where REST sends names.
        let threads = json!({ "value": [
            { "id": 5, "status": 1, "publishedDate": "2026-10-30T09:00:00Z",
              "comments": [{ "id": 1, "content": "Hm.", "commentType": 1, "author": { "displayName": "A", "id": "a" } }] }
        ] });
        let d = parse_azure(&azure_pr(), &threads, None, None, &azure_source(), &me()).unwrap();
        assert_eq!(d.conversation.len(), 1);
        assert!(!d.conversation[0].resolved, "1 is active");
        assert!(d.checks.is_empty());
    }

    #[test]
    fn azure_files_come_from_the_last_push() {
        let iteration = json!({
            "id": 3,
            "sourceRefCommit": { "commitId": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
            "targetRefCommit": { "commitId": "cccccccccccccccccccccccccccccccccccccccc" },
            "commonRefCommit": { "commitId": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }
        });
        let changes = json!({ "changeEntries": [
            { "item": { "path": "/src" , "isFolder": true }, "changeType": "edit" },
            { "item": { "path": "/src/Grid.cs" }, "changeType": "edit" },
            { "item": { "path": "/src/New.cs" }, "changeType": "add" },
            { "item": { "path": "/src/Moved.cs" }, "changeType": "edit, rename", "originalPath": "/src/Old.cs" },
            { "item": { "path": "/src/Gone.cs" }, "changeType": 16 }
        ] });
        let f = parse_azure_files(&iteration, &changes, "b8066eee-5517-4eae-a310-4d2751785492")
            .unwrap();
        assert_eq!(
            f.base, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "the merge base, not the target's tip"
        );
        let changes: Vec<_> = f
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.change))
            .collect();
        assert_eq!(
            changes,
            [
                ("/src/Grid.cs", Change::Modified),
                ("/src/New.cs", Change::Added),
                ("/src/Moved.cs", Change::Renamed),
                ("/src/Gone.cs", Change::Deleted),
            ]
        );
        assert_eq!(f.files[2].old_path.as_deref(), Some("/src/Old.cs"));
        assert!(!f.truncated);
    }

    #[test]
    fn what_goes_into_a_url_is_checked() {
        assert!(github_repo("o/r").is_ok());
        for bad in ["o", "o/r/x", "../r", "o/..", "o r/x", "o/r?x"] {
            assert!(github_repo(bad).is_err(), "{bad}");
        }
        assert!(number("61").is_ok());
        for bad in ["", "6a", "-1", "1/2"] {
            assert!(number(bad).is_err(), "{bad}");
        }
        assert!(is_sha("abc1234") && !is_sha("abc12") && !is_sha("xyz1234"));
        assert!(is_guid("b8066eee-5517-4eae-a310-4d2751785492") && !is_guid("Inet.Server"));
    }

    #[test]
    fn a_request_for_changes_on_github_needs_words() {
        let stored = Stored {
            source: github_source(),
            token: Some("t".into()),
        };
        let e = pull_vote(&stored, "o/r", "61", PullVerdict::ChangesRequested, "  ").unwrap_err();
        assert!(e.contains("comment"), "{e}");
        let e = pull_vote(&stored, "o/r", "61", PullVerdict::Waiting, "").unwrap_err();
        assert!(e.contains("no `wait for author` review"), "{e}");
    }

    #[test]
    fn jira_has_no_pull_requests() {
        let stored = Stored {
            source: Source {
                kind: Kind::Jira,
                name: "jira".into(),
                site: Some("https://x.atlassian.net".into()),
                email: Some("a@b".into()),
                ..Default::default()
            },
            token: Some("t".into()),
        };
        assert!(pull(&stored, "o/r", "1").unwrap_err().contains("Jira"));
    }
}
