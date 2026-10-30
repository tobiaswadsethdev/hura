//! Tickets and pull requests: what is yours on each tracker, read over REST.
//!
//! **Yours, and nothing else.** Every tracker is asked the same question: the
//! tickets assigned to you or created by you that are not done, and on GitHub
//! and Azure DevOps the open pull requests you opened or that wait on you. No
//! query language is involved, so the sidebar means the same thing whichever
//! tracker it reads.
//!
//! **Read by the client, with the client's own tokens.** A tracker is set up in
//! the desktop application and stored on the machine it runs on, and the
//! requests go from there; the server holds no tracker, no token and no ticket.
//! A server is where sandboxes run, and a Jira token is not something a
//! sandbox -- or the machine hosting them -- has any business holding.
//!
//! REST is for what the *interface* shows: a list, on a timer, rendered as
//! rows. It is also what a person changes from there (a status moved, a field
//! edited, a comment written), each when somebody presses something and never
//! on the timer (see [`edit`]). What an *agent* does with a ticket
//! goes through an MCP server when the agent decides to, which is a different
//! consumer with different failure modes.
//!
//! ## Why curl
//!
//! Three hosts, ordinary public certificates, JSON in and out. `curl` is on
//! every Linux machine and on every Windows since 10 1803, and the alternative
//! is an HTTP client, a TLS root store and a redirect policy pulled in for a
//! handful of GETs.
//!
//! **The credential goes in on stdin, never in the argument list.** `curl -K -`
//! reads its configuration -- the url and the `Authorization` header
//! included -- from standard input, so a token never appears in `ps` output or
//! in the error text of a failed spawn.

use serde::{Deserialize, Serialize};

pub mod doc;
mod edit;
mod issue;
pub mod markdown;
mod pull;
mod pulls;
pub use edit::{
    Choice, EditField, EditForm, Editable, FieldChange, Transition, UserChoice, comment,
    delete_comment, edit_comment, edit_form, save, transition, transitions, users,
};
pub use issue::{
    Issue, IssueComment, Parent as IssueParent, Person as IssuePerson, Stage as IssueStage, issue,
};
pub use pull::{
    PullCheck, PullCheckState, PullComment, PullDetail, PullFile, PullFiles, PullReviewer,
    PullState, PullThread, PullVerdict, pull, pull_comment, pull_file, pull_files, pull_vote,
};
pub use pulls::{Checks, PullRequest, Review};

/// A tracker this knows how to read.
// `TrackerKind` on the wire: `session::Kind` is already `Kind` in the one flat
// directory the bindings land in. Caught by the count check in
// `scripts/gen-bindings.sh`, which is what that check is for.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "TrackerKind"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    GitHub,
    AzureDevOps,
    Jira,
}

impl Kind {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "github" | "gh" => Ok(Kind::GitHub),
            "azure-devops" | "azure" | "ado" => Ok(Kind::AzureDevOps),
            "jira" | "atlassian" => Ok(Kind::Jira),
            other => Err(format!(
                "`{other}` is not a tracker; use github, azure-devops or jira"
            )),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::GitHub => "github",
            Kind::AzureDevOps => "azure-devops",
            Kind::Jira => "jira",
        }
    }
}

/// One configured tracker, without its token.
///
/// Validated when it is saved, so a Jira entry with no site or an Azure DevOps
/// entry with no organisation fails against the form that made it rather than
/// against a 404 on a timer. The token is kept beside it rather than in it --
/// see [`Configured`] -- so this is safe to hand a webview.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "Tracker"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Source {
    pub kind: Kind,
    /// What the sidebar calls it. Defaults to the kind, which is right until
    /// somebody has two Jira sites.
    pub name: String,
    /// GitHub: `owner/name` to read one repository, or `None` for every one the
    /// token can see.
    pub repo: Option<String>,
    /// Azure DevOps organisation, and the project its work items and pull
    /// requests are read from.
    pub org: Option<String>,
    pub project: Option<String>,
    /// Jira site, `https://your-org.atlassian.net`, and the account the token
    /// belongs to -- Jira Cloud is Basic auth with the email as the username.
    pub site: Option<String>,
    pub email: Option<String>,
}

impl Default for Source {
    /// Every field but the kind is optional on the wire, so a client sending a
    /// GitHub tracker does not have to send Jira's fields as nulls. The kind is
    /// the one thing there is no sensible default for, and `github` is the one
    /// that needs the least beside it.
    fn default() -> Self {
        Source {
            kind: Kind::GitHub,
            name: String::new(),
            repo: None,
            org: None,
            project: None,
            site: None,
            email: None,
        }
    }
}

impl Source {
    /// Trim every field, and fall back to the kind for an unnamed tracker.
    pub fn normalized(&self) -> Source {
        let text = |v: &Option<String>| {
            v.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let name = self.name.trim();
        Source {
            kind: self.kind,
            name: if name.is_empty() {
                self.kind.label().to_string()
            } else {
                name.to_string()
            },
            repo: text(&self.repo),
            org: text(&self.org),
            project: text(&self.project),
            site: text(&self.site),
            email: text(&self.email),
        }
    }

    /// What is missing, if anything.
    pub fn problem(&self) -> Option<String> {
        let missing = |what: &str| {
            Some(format!(
                "`{}` is a {} tracker with no {what}",
                self.name,
                self.kind.label()
            ))
        };
        match self.kind {
            Kind::Jira => {
                if self.site.is_none() {
                    return missing("site");
                }
                if self.email.is_none() {
                    // Jira Cloud's Basic auth is email + API token; a token
                    // alone authenticates as nobody.
                    return missing("email");
                }
                None
            }
            Kind::AzureDevOps => {
                if self.org.is_none() {
                    return missing("org");
                }
                if self.project.is_none() {
                    return missing("project");
                }
                None
            }
            Kind::GitHub => None,
        }
    }
}

/// One ticket, as every tracker's answer is flattened into.
///
/// Normalised here rather than in a client, because there are two clients and
/// three trackers: nine renderings, or one shape. The `id` is the tracker's own
/// and is what the write-back addresses; the `key` is what a person calls it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Which configured tracker this came from, by name.
    pub tracker: String,
    pub kind: Kind,
    /// The tracker's own identifier: a work item id, an issue number, a Jira
    /// key. What a comment or a transition is addressed to.
    pub id: String,
    /// What a person calls it: `PROJ-123`, `#45`, `AB#1234`.
    pub key: String,
    pub title: String,
    /// Where to open it in a browser.
    pub url: String,
    /// Its state, in the tracker's own words -- `In Progress`, `Active`,
    /// `open`. Not mapped onto a scheme of ours: a status somebody configured
    /// is a fact about their process, and renaming it would lose it.
    pub status: String,
    /// Bug, Story, Task, whatever the tracker calls it. Empty when it has no
    /// such notion.
    pub item_type: String,
    /// The session name this ticket suggests: `proj-123-add-the-changelog`.
    /// Derived here so both front ends offer the same one.
    pub session_name: String,
    /// The branch it suggests, prefix included.
    pub branch: String,
    /// `owner/name`, for a GitHub issue. `None` for the other two, whose
    /// write-back is addressed by organisation and project from the config
    /// file. Carried because a search spans repositories: a comment has to go
    /// to the one the issue is actually in, which the entry may not name.
    pub repo: Option<String>,
    /// Whether it is assigned to you, or only created by you. One row per
    /// ticket: one that is both is assigned, because that is the part that
    /// asks something of you.
    pub relation: Relation,
    /// When the tracker last saw it change, in the tracker's own format. Only
    /// ever compared with an earlier value of itself.
    pub updated: Option<String>,
    /// How many comments it has. `None` where the tracker's list answer does
    /// not say.
    pub comments: Option<u32>,
    /// Who wrote the newest comment, as the tracker shows them.
    pub last_commenter: Option<String>,
    /// Whether the newest comment is the credential owner's own -- which is
    /// the one comment nobody needs to be told about.
    pub last_comment_mine: bool,
}

/// Which side of "mine" a ticket or a pull request is on.
///
/// The sidebar's two groups under each heading. Not a filter anybody wrote: it
/// is read off the ticket, the same way on every tracker.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Relation {
    /// A ticket assigned to you. A pull request somebody asked you to
    /// review, or assigned to you, that you did not open.
    Assigned,
    /// A ticket you created that is not assigned to you, or a pull request
    /// you opened.
    Created,
}

/// What a session remembers about the ticket it was started from.
///
/// On the session record as a note about where the work came from: which
/// tracker, which ticket, and where to open it. Nothing writes back through
/// it -- commenting on or moving a ticket is an agent's job, through an MCP
/// server, or yours.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub tracker: String,
    pub kind: Kind,
    pub id: String,
    pub key: String,
    pub url: String,
    /// The GitHub repository the issue is in; see [`Task::repo`].
    #[serde(default)]
    pub repo: Option<String>,
}

/// The pull request a session was started from.
///
/// On the session record as a note of where the work came from, like
/// [`Ticket`], and for one thing more: a session with one is seeded onto the
/// pull request's own branch, which is already on the remote, rather than a
/// new branch made from its base. See `seed::clone_and_branch`.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullOrigin {
    pub tracker: String,
    pub kind: Kind,
    pub id: String,
    /// `#58` or `!26056`.
    pub key: String,
    pub url: String,
    /// `owner/name` on GitHub, the repository's name on Azure DevOps.
    pub repo: String,
}

impl From<&PullRequest> for PullOrigin {
    fn from(p: &PullRequest) -> Self {
        PullOrigin {
            tracker: p.tracker.clone(),
            kind: p.kind,
            id: p.id.clone(),
            key: p.key.clone(),
            url: p.url.clone(),
            repo: p.repo.clone(),
        }
    }
}

impl From<&Task> for Ticket {
    fn from(t: &Task) -> Self {
        Ticket {
            tracker: t.tracker.clone(),
            kind: t.kind,
            id: t.id.clone(),
            key: t.key.clone(),
            url: t.url.clone(),
            repo: t.repo.clone(),
        }
    }
}

/// What is yours on every tracker, and whatever could not be read.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inbox {
    pub tasks: Vec<Task>,
    /// Open pull requests, from the GitHub and Azure DevOps trackers. Jira
    /// has none to give.
    #[serde(default)]
    pub pulls: Vec<PullRequest>,
    /// One per tracker, or half of one, that failed, in words. A tracker that
    /// cannot be read is a row missing from a list, which is invisible, so it
    /// is said out loud rather than left as an empty sidebar.
    pub warnings: Vec<String>,
    /// The trackers whose tickets were read, whether or not they held any.
    ///
    /// What lets a client tell "nothing is yours" from "could not ask": a
    /// tracker missing from here was not read, so a ticket missing from it has
    /// not gone anywhere, and one appearing in it after an outage has not just
    /// arrived.
    #[serde(default)]
    pub read: Vec<String>,
    /// The same, for pull requests. Kept apart from `read` because Azure
    /// DevOps reads them with a scope its work items do not need, so one half
    /// of a tracker can answer while the other is refused.
    #[serde(default)]
    pub pulls_read: Vec<String>,
}

/// How long to give a tracker before giving up on it.
///
/// The inbox is polled, so a tracker that has gone away must not make the
/// window feel broken. Twenty seconds is generous for a search API and short
/// enough that three of them cannot stack into a minute.
const TIMEOUT: &str = "20";

/// A tracker as the client keeps it: the entry, and the token beside it.
///
/// Stored on the machine the window runs on, in its private state file. The
/// token never leaves this type towards a webview: [`Configured`] is what one
/// is shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    pub source: Source,
    #[serde(default)]
    pub token: Option<String>,
}

/// A tracker as a webview sees it: whether there is a token, never the token.
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, rename = "ConfiguredTracker")
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Configured {
    pub source: Source,
    pub token_set: bool,
}

impl From<&Stored> for Configured {
    fn from(s: &Stored) -> Self {
        Configured {
            token_set: s.token.as_deref().is_some_and(|t| !t.trim().is_empty()),
            source: s.source.clone(),
        }
    }
}

/// Read every tracker, one after another.
///
/// Sequentially, because there are one or two of them and a thread per tracker
/// would buy milliseconds at the cost of ordering the result.
pub fn inbox(trackers: &[Stored], branch_prefix: &str) -> Inbox {
    let mut out = Inbox::default();
    for Stored { source, token } in trackers {
        if let Some(problem) = source.problem() {
            out.warnings.push(problem);
            continue;
        }
        let Some(token) = token.as_deref().filter(|t| !t.trim().is_empty()) else {
            out.warnings.push(format!(
                "{}: no token yet, so nothing was read",
                source.name
            ));
            continue;
        };
        let name = &source.name;
        match source.kind {
            Kind::Jira => match jira(source, token, branch_prefix) {
                Ok(tasks) => {
                    out.tasks.extend(tasks);
                    out.read.push(name.clone());
                }
                Err(e) => out.warnings.push(format!("{name}: {e}")),
            },
            // One request for both halves, so they answer or fail together.
            Kind::GitHub => match github(source, token, branch_prefix) {
                Ok(mine) => {
                    out.tasks.extend(mine.tasks);
                    out.pulls.extend(mine.pulls);
                    out.warnings
                        .extend(mine.warnings.into_iter().map(|w| format!("{name}: {w}")));
                    out.read.push(name.clone());
                    out.pulls_read.push(name.clone());
                }
                Err(e) => out.warnings.push(format!("{name}: {e}")),
            },
            Kind::AzureDevOps => {
                // Who the token belongs to, asked once: the work items are
                // sorted by it and the pull requests are searched by it. The
                // request is the cheapest one there is, so a failure here
                // stands for both halves rather than one warning each.
                let me = match azure_me(source, token) {
                    Ok(me) => me,
                    Err(e) => {
                        out.warnings.push(format!("{name}: {e}"));
                        continue;
                    }
                };
                match azure(source, token, &me, branch_prefix) {
                    Ok(tasks) => {
                        out.tasks.extend(tasks);
                        out.read.push(name.clone());
                    }
                    Err(e) => out.warnings.push(format!("{name} · work items: {e}")),
                }
                match pulls::azure(source, token, &me) {
                    Ok(pulls) => {
                        out.pulls.extend(pulls);
                        out.pulls_read.push(name.clone());
                    }
                    Err(e) => out.warnings.push(format!(
                        "{name} · pull requests: {}",
                        pulls::azure_refusal(&e)
                    )),
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------- GitHub

/// What one GitHub tracker holds. Both halves come back from one request.
struct GitHubMine {
    tasks: Vec<Task>,
    pulls: Vec<PullRequest>,
    /// What GitHub said went wrong beside an answer: an organisation behind
    /// SAML the token is not authorised for, say. The rest still counts.
    warnings: Vec<String>,
}

/// Every search a GitHub tracker makes, as one GraphQL request.
///
/// GraphQL rather than `/issues` and `/search`, because it is the one way to
/// ask five questions in a request, and the only one that says in the same
/// answer whether a pull request's checks pass and whether it is approved.
/// REST would be a request per pull request for each of those.
const GITHUB_QUERY: &str = r#"query($assigned: String!, $created: String!, $mine: String!, $review: String!, $assignee: String!) {
  viewer { login }
  assigned: search(type: ISSUE, first: 50, query: $assigned) { nodes { ...issue } }
  created: search(type: ISSUE, first: 50, query: $created) { nodes { ...issue } }
  mine: search(type: ISSUE, first: 50, query: $mine) { nodes { ...pull } }
  review: search(type: ISSUE, first: 50, query: $review) { nodes { ...pull } }
  assignee: search(type: ISSUE, first: 50, query: $assignee) { nodes { ...pull } }
}
fragment issue on Issue {
  number title url state updatedAt
  repository { nameWithOwner }
  labels(first: 1) { nodes { name } }
  comments(last: 1) { totalCount nodes { author { login } } }
}
fragment pull on PullRequest {
  number title url isDraft updatedAt headRefName baseRefName mergeable reviewDecision
  isCrossRepository
  author { login }
  repository { nameWithOwner }
  comments { totalCount }
  commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
}"#;

/// The five searches, scoped to the tracker's repository when it names one.
///
/// `review-requested` rather than `user-review-requested`, so a review asked
/// of a team you are on counts: that is how most reviews are asked for.
fn github_searches(repo: Option<&str>) -> serde_json::Value {
    let open = "is:open archived:false sort:updated-desc";
    let scoped = |q: &str| match repo {
        Some(repo) => format!("{q} {open} repo:{repo}"),
        None => format!("{q} {open}"),
    };
    serde_json::json!({
        "assigned": scoped("is:issue assignee:@me"),
        "created": scoped("is:issue author:@me"),
        "mine": scoped("is:pr author:@me"),
        "review": scoped("is:pr review-requested:@me"),
        "assignee": scoped("is:pr assignee:@me"),
    })
}

fn github(source: &Source, token: &str, prefix: &str) -> Result<GitHubMine, String> {
    let body = serde_json::json!({
        "query": GITHUB_QUERY,
        "variables": github_searches(source.repo.as_deref()),
    });
    let answer = post(
        "https://api.github.com/graphql",
        &format!("Bearer {token}"),
        &body.to_string(),
        JSON,
    )?;
    parse_github(&answer, source, prefix)
}

/// GitHub's answer, flattened. Separate from the request so every shape it
/// sends can be asserted on without a network, which is the only way to have
/// any confidence in a reader of somebody else's JSON.
fn parse_github(
    body: &serde_json::Value,
    source: &Source,
    prefix: &str,
) -> Result<GitHubMine, String> {
    // GraphQL answers 200 with its errors in the body, and with whatever it
    // could read beside them.
    let errors: Vec<String> = body
        .get("errors")
        .and_then(|e| e.as_array())
        .map(|e| {
            e.iter()
                .map(|e| string(e, "message"))
                .filter(|m| !m.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let Some(data) = body.get("data").filter(|d| !d.is_null()) else {
        return Err(if errors.is_empty() {
            "github did not answer with any data".to_string()
        } else {
            errors.join("; ")
        });
    };
    let me = data
        .get("viewer")
        .map(|v| string(v, "login"))
        .unwrap_or_default();
    let nodes = |alias: &str| -> Vec<serde_json::Value> {
        data.get(alias)
            .and_then(|s| s.get("nodes"))
            .and_then(|n| n.as_array())
            .cloned()
            .unwrap_or_default()
    };

    // Assigned first, so a ticket that is both is the one that asks
    // something of you.
    let mut tasks: Vec<Task> = Vec::new();
    for (alias, relation) in [
        ("assigned", Relation::Assigned),
        ("created", Relation::Created),
    ] {
        for node in nodes(alias) {
            let Some(task) = github_issue(&node, source, &me, relation, prefix) else {
                continue;
            };
            if !tasks.iter().any(|t| t.repo == task.repo && t.id == task.id) {
                tasks.push(task);
            }
        }
    }

    // Yours first, the other way round: assigning yourself your own pull
    // request is a habit, and it does not make it a review.
    let mut pulls: Vec<PullRequest> = Vec::new();
    for (alias, relation, review) in [
        ("mine", Relation::Created, false),
        ("review", Relation::Assigned, true),
        ("assignee", Relation::Assigned, false),
    ] {
        for node in nodes(alias) {
            let Some(pull) = pulls::github(&node, &source.name, relation, review) else {
                continue;
            };
            if !pulls.iter().any(|p| p.repo == pull.repo && p.id == pull.id) {
                pulls.push(pull);
            }
        }
    }

    Ok(GitHubMine {
        tasks,
        pulls,
        warnings: errors,
    })
}

fn github_issue(
    i: &serde_json::Value,
    source: &Source,
    me: &str,
    relation: Relation,
    prefix: &str,
) -> Option<Task> {
    let number = i.get("number")?.as_u64()?;
    let title = string(i, "title");
    let repo = i
        .get("repository")
        .map(|r| string(r, "nameWithOwner"))
        .filter(|s| !s.is_empty())
        .or_else(|| source.repo.clone());
    let key = format!("#{number}");
    let comments = i.get("comments");
    let last = comments
        .and_then(|c| c.get("nodes"))
        .and_then(|n| n.as_array())
        .and_then(|n| n.last())
        .and_then(|c| c.get("author"))
        .map(|a| string(a, "login"))
        .filter(|s| !s.is_empty());
    Some(Task {
        tracker: source.name.clone(),
        kind: Kind::GitHub,
        id: number.to_string(),
        session_name: session_name(&key, &title),
        branch: branch(prefix, &key, &title),
        key,
        title,
        url: string(i, "url"),
        // `OPEN` in GraphQL, `open` everywhere else GitHub says it.
        status: string(i, "state").to_lowercase(),
        item_type: label_of(i).unwrap_or_default(),
        repo,
        relation,
        updated: Some(string(i, "updatedAt")).filter(|s| !s.is_empty()),
        comments: comments
            .and_then(|c| c.get("totalCount"))
            .and_then(|c| c.as_u64())
            .map(|c| c as u32),
        last_comment_mine: !me.is_empty() && last.as_deref() == Some(me),
        last_commenter: last,
    })
}

/// The first label, which is the closest GitHub has to a type.
fn label_of(issue: &serde_json::Value) -> Option<String> {
    let labels = issue.get("labels")?.get("nodes")?.as_array()?;
    labels
        .first()
        .map(|l| string(l, "name"))
        .filter(|s| !s.is_empty())
}

// ---------------------------------------------------------- Azure DevOps

/// Who an Azure DevOps token belongs to: the identity a work item's assignee
/// and a pull request's reviewers are compared with.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AzureMe {
    id: String,
    /// The sign-in name, `you@example.com`. A second way to recognise the
    /// account, for an answer that carries the name and not the id.
    account: Option<String>,
}

impl AzureMe {
    fn is(&self, identity: &serde_json::Value) -> bool {
        let id = string(identity, "id");
        if !id.is_empty() && id.eq_ignore_ascii_case(&self.id) {
            return true;
        }
        let name = string(identity, "uniqueName");
        self.account
            .as_deref()
            .is_some_and(|a| !name.is_empty() && a.eq_ignore_ascii_case(&name))
    }
}

fn azure_me(source: &Source, token: &str) -> Result<AzureMe, String> {
    let org = source.org.as_deref().unwrap_or_default();
    let url = format!("https://dev.azure.com/{org}/_apis/connectionData");
    let body = get(&url, &azure_auth(token), &[])?;
    parse_azure_me(&body)
}

fn parse_azure_me(body: &serde_json::Value) -> Result<AzureMe, String> {
    let user = body
        .get("authenticatedUser")
        .ok_or("azure devops did not say who the token belongs to")?;
    let id = string(user, "id");
    if id.is_empty() {
        return Err("azure devops did not say who the token belongs to".into());
    }
    let account = user
        .get("properties")
        .and_then(|p| p.get("Account"))
        .and_then(|a| a.get("$value"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(AzureMe { id, account })
}

/// Assigned to you or created by you, in this project, and not finished.
///
/// A list of state names rather than a category: WIQL has no state category to
/// ask by, and these four are what the shipped processes call "over".
const AZURE_MINE: &str = "SELECT [System.Id] FROM WorkItems \
     WHERE [System.TeamProject] = @project \
     AND ([System.AssignedTo] = @Me OR [System.CreatedBy] = @Me) \
     AND [System.State] NOT IN ('Closed', 'Done', 'Removed', 'Resolved') \
     ORDER BY [System.ChangedDate] DESC";

fn azure(source: &Source, token: &str, me: &AzureMe, prefix: &str) -> Result<Vec<Task>, String> {
    let org = source.org.as_deref().unwrap_or_default();
    let project = source.project.as_deref().unwrap_or_default();
    let auth = azure_auth(token);

    // Two requests, and there is no way around it: WIQL answers with ids only.
    let query_url =
        format!("https://dev.azure.com/{org}/{project}/_apis/wit/wiql?api-version=7.1&$top=100");
    let body = serde_json::json!({ "query": AZURE_MINE });
    let answer = post(&query_url, &auth, &body.to_string(), JSON)?;

    let ids = parse_azure_ids(&answer);
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    // `fields` rather than the whole work item: a work item with its history is
    // tens of kilobytes and these are the whole row.
    let detail_url = format!(
        "https://dev.azure.com/{org}/{project}/_apis/wit/workitems?ids={}&fields=System.Id,System.Title,System.State,System.WorkItemType,System.ChangedDate,System.CommentCount,System.AssignedTo&api-version=7.1",
        ids.join(",")
    );
    let detail = get(&detail_url, &auth, &[])?;
    parse_azure(&detail, source, me, prefix)
}

/// The ids a WIQL query answered with. Its own function because the two-request
/// shape is the thing worth asserting: a query that matches nothing must not
/// produce a second request for zero ids, which Azure DevOps answers with a
/// 400.
fn parse_azure_ids(body: &serde_json::Value) -> Vec<String> {
    body.get("workItems")
        .and_then(|w| w.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.get("id").and_then(|v| v.as_u64()))
                .map(|id| id.to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_azure(
    detail: &serde_json::Value,
    source: &Source,
    me: &AzureMe,
    prefix: &str,
) -> Result<Vec<Task>, String> {
    let org = source.org.as_deref().unwrap_or_default();
    let project = source.project.as_deref().unwrap_or_default();
    let items = detail
        .get("value")
        .and_then(|v| v.as_array())
        .ok_or("azure devops did not answer with work items")?;

    Ok(items
        .iter()
        .filter_map(|i| {
            let id = i.get("id")?.as_u64()?;
            let fields = i.get("fields")?;
            let title = string(fields, "System.Title");
            let key = format!("AB#{id}");
            let mine = fields.get("System.AssignedTo").is_some_and(|a| me.is(a));
            Some(Task {
                tracker: source.name.clone(),
                kind: Kind::AzureDevOps,
                id: id.to_string(),
                session_name: session_name(&key, &title),
                branch: branch(prefix, &key, &title),
                key,
                title,
                // Built rather than read from `_links`, which needs `$expand`
                // and doubles the payload for a URL whose shape is fixed.
                url: format!("https://dev.azure.com/{org}/{project}/_workitems/edit/{id}"),
                status: string(fields, "System.State"),
                item_type: string(fields, "System.WorkItemType"),
                repo: None,
                relation: if mine {
                    Relation::Assigned
                } else {
                    Relation::Created
                },
                updated: Some(string(fields, "System.ChangedDate")).filter(|s| !s.is_empty()),
                comments: fields
                    .get("System.CommentCount")
                    .and_then(|c| c.as_u64())
                    .map(|c| c as u32),
                last_commenter: None,
                last_comment_mine: false,
            })
        })
        .collect())
}

/// Azure DevOps PATs are HTTP Basic with the token as the *password* and an
/// empty username. A bearer token gets a 302 to a sign-in page rather than a
/// 401, which is a singularly unhelpful way to fail -- the same lesson
/// [`crate::forge`] records for git.
fn azure_auth(token: &str) -> String {
    format!(
        "Basic {}",
        crate::skills::base64(format!(":{token}").as_bytes())
    )
}

// ------------------------------------------------------------------ Jira

fn jira_site(source: &Source) -> &str {
    source
        .site
        .as_deref()
        .unwrap_or_default()
        .trim_end_matches('/')
}

fn jira_auth(source: &Source, token: &str) -> String {
    let email = source.email.as_deref().unwrap_or_default();
    format!(
        "Basic {}",
        crate::skills::base64(format!("{email}:{token}").as_bytes())
    )
}

/// The credential owner's account id, which is how a comment says who wrote
/// it and an assignee says whose it is. Emails are hidden on most Jira Cloud
/// sites, so the configured one cannot be matched against either.
///
/// Also the cheapest question that says whether the site and the token work
/// at all: a failure here is the answer for the search too.
fn jira_me(source: &Source, token: &str) -> Result<Option<String>, String> {
    let url = format!("{}/rest/api/3/myself", jira_site(source));
    let body = get(
        &url,
        &jira_auth(source, token),
        &["Accept: application/json"],
    )?;
    Ok(Some(string(&body, "accountId")).filter(|s| !s.is_empty()))
}

/// Assigned to you or reported by you, and not done.
///
/// `statusCategory != Done` rather than a list of status names: every Jira
/// project renames its statuses and none of them rename the categories.
const JIRA_MINE: &str = "(assignee = currentUser() OR reporter = currentUser()) \
     AND statusCategory != Done ORDER BY updated DESC";

fn jira(source: &Source, token: &str, prefix: &str) -> Result<Vec<Task>, String> {
    let me = jira_me(source, token)?;
    let site = jira_site(source);
    // `/search/jql`, not `/search`: the older endpoint is deprecated on Jira
    // Cloud and answers 410 on newer sites. `comment` is what makes a new
    // comment noticeable without a request per issue, and `assignee` is what
    // says which group a ticket goes in.
    let url = format!(
        "{site}/rest/api/3/search/jql?jql={}&fields=summary,status,issuetype,updated,comment,assignee&maxResults=100",
        urlencode(JIRA_MINE)
    );
    let body = get(
        &url,
        &jira_auth(source, token),
        &["Accept: application/json"],
    )?;
    parse_jira(&body, source, me.as_deref(), prefix)
}

fn parse_jira(
    body: &serde_json::Value,
    source: &Source,
    me: Option<&str>,
    prefix: &str,
) -> Result<Vec<Task>, String> {
    let issues = body
        .get("issues")
        .and_then(|i| i.as_array())
        .ok_or("jira did not answer with issues")?;
    Ok(issues
        .iter()
        .filter_map(|i| jira_task(i, source, me, prefix))
        .collect())
}

/// One issue from a Jira search, as a task.
fn jira_task(
    i: &serde_json::Value,
    source: &Source,
    me: Option<&str>,
    prefix: &str,
) -> Option<Task> {
    let site = jira_site(source);
    let key = string(i, "key");
    if key.is_empty() {
        return None;
    }
    let fields = i.get("fields")?;
    let title = string(fields, "summary");
    let comment = fields.get("comment");
    // The newest by `created`, rather than the last in the list: the list is a
    // page, and its order is Jira's to change.
    let last = comment
        .and_then(|c| c.get("comments"))
        .and_then(|c| c.as_array())
        .and_then(|list| list.iter().max_by_key(|c| string(c, "created")));
    let assignee = fields
        .get("assignee")
        .map(|a| string(a, "accountId"))
        .filter(|s| !s.is_empty());
    Some(Task {
        tracker: source.name.clone(),
        kind: Kind::Jira,
        // The key *is* the id in Jira's API: every write-back path takes an
        // issue key or its numeric id interchangeably.
        id: key.clone(),
        session_name: session_name(&key, &title),
        branch: branch(prefix, &key, &title),
        url: format!("{site}/browse/{key}"),
        key,
        title,
        status: fields
            .get("status")
            .map(|s| string(s, "name"))
            .unwrap_or_default(),
        item_type: fields
            .get("issuetype")
            .map(|t| string(t, "name"))
            .unwrap_or_default(),
        repo: None,
        // The search matched it as assigned or as reported. Without knowing
        // who "me" is, an assignee is the better guess at which.
        relation: match (me, assignee.as_deref()) {
            (Some(me), Some(assignee)) if me == assignee => Relation::Assigned,
            (None, Some(_)) => Relation::Assigned,
            _ => Relation::Created,
        },
        updated: Some(string(fields, "updated")).filter(|s| !s.is_empty()),
        comments: comment
            .and_then(|c| c.get("total"))
            .and_then(|t| t.as_u64())
            .map(|t| t as u32),
        last_commenter: last
            .and_then(|c| c.get("author"))
            .map(|a| string(a, "displayName"))
            .filter(|s| !s.is_empty()),
        last_comment_mine: match (me, last.and_then(|c| c.get("author"))) {
            (Some(me), Some(author)) => string(author, "accountId") == me,
            _ => false,
        },
    })
}

// --------------------------------------------------------- naming things

/// The session name a ticket suggests: `proj-123-add-the-changelog`.
///
/// The key first, because that is what makes a session findable next to a
/// tracker, and the title after it for the sake of the person reading the list.
/// Truncated to what [`crate::session::validate_name`] accepts, at a dash, so
/// the result never ends mid-word or -- worse -- mid-dash.
pub fn session_name(key: &str, title: &str) -> String {
    let key = slug(key);
    let rest = slug(title);
    let joined = if rest.is_empty() {
        key.clone()
    } else {
        format!("{key}-{rest}")
    };
    // 40 is the session name limit; see `session::MAX_NAME`.
    let cut = truncate_at_dash(&joined, 40);
    if cut.is_empty() { key } else { cut }
}

/// The branch a ticket suggests, which is the convention this whole loop exists
/// to keep: `<prefix>/<KEY>-<description>`.
///
/// The key keeps its case here where the session name lowercases it: a branch
/// name is read by people and by the tracker's own commit hooks, and `PROJ-123`
/// is what both look for.
pub fn branch(prefix: &str, key: &str, title: &str) -> String {
    let key = key.trim().trim_start_matches('#').replace(' ', "");
    let rest = slug(title);
    let stem = if rest.is_empty() {
        key
    } else {
        format!("{key}-{}", truncate_at_dash(&rest, 40))
    };
    match prefix.trim().trim_matches('/') {
        "" => stem,
        prefix => format!("{prefix}/{stem}"),
    }
}

/// Lowercase, dashes, and nothing else. Deliberately not
/// [`crate::session::slugify`], which drops filler words to make a name out of
/// a *task description*; a ticket title is already a title and dropping "the"
/// from it would make it harder to recognise, not easier.
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn truncate_at_dash(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let cut = &s[..max];
    match cut.rfind('-') {
        Some(i) if i > 0 => cut[..i].to_string(),
        _ => cut.trim_end_matches('-').to_string(),
    }
}

// -------------------------------------------------------------------- curl

const JSON: &str = "application/json";

fn get(url: &str, auth: &str, headers: &[&str]) -> Result<serde_json::Value, String> {
    curl(url, auth, headers, None)
}

fn post(
    url: &str,
    auth: &str,
    body: &str,
    content_type: &str,
) -> Result<serde_json::Value, String> {
    curl(url, auth, &[], Some(("POST", body, content_type)))
}

/// A write with a JSON body -- `POST`, `PUT` -- or none, for a `DELETE`.
fn send(
    method: &str,
    url: &str,
    auth: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let body = body.map(serde_json::Value::to_string).unwrap_or_default();
    curl(url, auth, &[], Some((method, &body, JSON)))
}

/// One request, with the credential on stdin.
///
/// `-K -` makes curl read its configuration from standard input, which is how
/// the url and the `Authorization` header stay out of the argument list -- and
/// so out of `ps`, out of a failed spawn's error text, and out of anything that
/// logs a command line. The body goes the same way, since a body can carry a
/// credential too.
///
/// The status code is asked for separately and printed after the body, because
/// curl's exit code is about the transport: a 401 is a request that worked and
/// an answer that has to be read as one.
fn curl(
    url: &str,
    auth: &str,
    headers: &[&str],
    body: Option<(&str, &str, &str)>,
) -> Result<serde_json::Value, String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut config = String::new();
    config.push_str(&format!("url = {}\n", quote(url)));
    config.push_str(&format!("header = {}\n", quote(&auth_header(auth))));
    // Every one of these APIs answers JSON and some of them need to be told.
    config.push_str(&format!("header = {}\n", quote(&format!("Accept: {JSON}"))));
    // GitHub refuses a request with no user agent, with a 403 that says so in
    // prose.
    config.push_str("user-agent = \"hura\"\n");
    for header in headers {
        config.push_str(&format!("header = {}\n", quote(header)));
    }
    config.push_str("silent\nshow-error\nlocation\n");
    config.push_str(&format!("max-time = {TIMEOUT}\n"));
    // The status code, after the body, on its own line.
    config.push_str("write-out = \"\\n%{http_code}\"\n");
    if let Some((method, body, content_type)) = body {
        config.push_str(&format!("request = {}\n", quote(method)));
        // A `DELETE` has no body, and a `Content-Type` with nothing after it
        // is a request some servers refuse.
        if !body.is_empty() {
            config.push_str(&format!(
                "header = {}\n",
                quote(&format!("Content-Type: {content_type}"))
            ));
            config.push_str(&format!("data-raw = {}\n", quote(body)));
        }
    }

    let mut command = Command::new("curl");
    command
        .arg("-K")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A GUI process that spawns a console program gets a console window for it,
    // flashing up every time the tickets are read. `CREATE_NO_WINDOW` is the
    // flag that says not to.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not run curl: {e}"))?;
    child
        .stdin
        .as_mut()
        .ok_or("curl took no input")?
        .write_all(config.as_bytes())
        .map_err(|e| format!("could not write to curl: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("curl did not finish: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "the request failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    interpret(&String::from_utf8_lossy(&out.stdout))
}

/// Split the body from the status line curl appended, and read one as the other
/// asks for.
///
/// Separate from the request so every status the trackers answer with can be
/// asserted on without a network.
fn interpret(stdout: &str) -> Result<serde_json::Value, String> {
    let (body, code) = stdout
        .rsplit_once('\n')
        .ok_or("the request produced nothing at all")?;
    let code: u16 = code.trim().parse().unwrap_or(0);
    let body = body.trim();

    if (200..300).contains(&code) {
        if body.is_empty() {
            // A comment posts and answers 201 with a body some of the time and
            // 204 with none the rest; neither is a failure.
            return Ok(serde_json::Value::Null);
        }
        return serde_json::from_str(body)
            .map_err(|e| format!("the answer was not json: {e}: {}", first_line(body)));
    }

    // Each of the three says what is wrong in a different field, and all three
    // are worth quoting rather than replacing with "the request failed": a
    // wrong project name and an expired token are the same status code.
    let said = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            if let Some(said) = jira_errors(&v) {
                return Some(said);
            }
            for key in ["message", "errorMessages", "error_description", "detail"] {
                if let Some(found) = v.get(key) {
                    return Some(match found {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    });
                }
            }
            None
        })
        .unwrap_or_else(|| first_line(body));

    Err(match code {
        401 | 403 => format!("{code}: {said} (is the stored credential still valid?)"),
        404 => format!("404: {said} (check the org, project or site in the config file)"),
        0 => format!("no status came back: {said}"),
        _ => format!("{code}: {said}"),
    })
}

/// Jira's refusal of a write: `errorMessages` for the request as a whole and
/// `errors` per field -- `summary: You must specify a summary` -- either of
/// which may be the empty one. `None` when neither says anything.
fn jira_errors(v: &serde_json::Value) -> Option<String> {
    let mut said: Vec<String> = v
        .get("errorMessages")
        .and_then(|m| m.as_array())
        .map(|m| {
            m.iter()
                .filter_map(|m| m.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if let Some(errors) = v.get("errors").and_then(|e| e.as_object()) {
        for (field, message) in errors {
            let message = message
                .as_str()
                .map_or_else(|| message.to_string(), str::to_string);
            said.push(format!("{field}: {message}"));
        }
    }
    (!said.is_empty()).then(|| said.join("; "))
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(200).collect()
}

/// The credential as a header line.
///
/// Every caller above builds the *value* -- `Bearer x`, `Basic y` -- and the
/// name is added here, so nothing that reaches this file can name a header of
/// its own. It was missing, briefly, and the failure is worth recording: curl
/// takes a `-H` string with no colon in it as an instruction to *remove* a
/// header, so the request went out with no `Authorization` at all and the
/// trackers answered 401. Nothing in the unit tests could see it, which is why
/// there is a test that sends a request to a listener and reads the headers.
fn auth_header(auth: &str) -> String {
    format!("Authorization: {auth}")
}

/// A curl config value: double quotes, with backslashes and quotes escaped.
///
/// Written out rather than assumed, because everything interpolated above is
/// attacker-influenced in the general case -- a ticket title, a JQL string, a
/// token -- and an unescaped quote would end the value and start a new
/// configuration line, which in a curl config file can name a file to write.
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            // A newline would end the line whatever the quoting; there is no
            // escape for one in a curl config, so it is dropped. None of the
            // values here legitimately contain one.
            '\n' | '\r' => out.push(' '),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Percent-encode everything that is not unreserved.
///
/// JQL is full of spaces, quotes, equals signs and parentheses, and it goes in
/// a query string. A small table rather than a dependency: this is the only
/// place in the crate that needs one.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn string(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(kind: Kind) -> Source {
        Source {
            kind,
            name: kind.label().to_string(),
            repo: None,
            org: Some("contoso".into()),
            project: Some("tools".into()),
            site: Some("https://example.atlassian.net".into()),
            email: Some("you@example.com".into()),
        }
    }

    /// The GraphQL answer's shape, measured against a real one and trimmed to
    /// what is read: an issue in both of its searches, one only created, a
    /// pull request of yours that you also assigned yourself, and one
    /// waiting on your review.
    const GITHUB: &str = r#"{
      "data": {
        "viewer": { "login": "me" },
        "assigned": { "nodes": [
          {
            "number": 45,
            "title": "Readme says the wrong port",
            "url": "https://github.com/o/r/issues/45",
            "state": "OPEN",
            "updatedAt": "2026-10-20T10:00:00Z",
            "repository": { "nameWithOwner": "o/r" },
            "labels": { "nodes": [{ "name": "bug" }] },
            "comments": { "totalCount": 3, "nodes": [{ "author": { "login": "me" } }] }
          }
        ] },
        "created": { "nodes": [
          {
            "number": 45,
            "title": "Readme says the wrong port",
            "url": "https://github.com/o/r/issues/45",
            "state": "OPEN",
            "repository": { "nameWithOwner": "o/r" },
            "labels": { "nodes": [] },
            "comments": { "totalCount": 3, "nodes": [] }
          },
          {
            "number": 9,
            "title": "Flaky spec",
            "url": "https://github.com/o/other/issues/9",
            "state": "OPEN",
            "repository": { "nameWithOwner": "o/other" },
            "labels": { "nodes": [] },
            "comments": { "totalCount": 1, "nodes": [{ "author": { "login": "sam" } }] }
          }
        ] },
        "mine": { "nodes": [
          { "number": 46, "title": "Bump the lockfile", "url": "https://github.com/o/r/pull/46",
            "headRefName": "me/bump", "baseRefName": "main", "repository": { "nameWithOwner": "o/r" } }
        ] },
        "review": { "nodes": [
          { "number": 50, "title": "Cache the dashboard", "url": "https://github.com/o/r/pull/50",
            "headRefName": "sam/cache", "baseRefName": "main", "repository": { "nameWithOwner": "o/r" } },
          {}
        ] },
        "assignee": { "nodes": [
          { "number": 46, "title": "Bump the lockfile", "url": "https://github.com/o/r/pull/46",
            "headRefName": "me/bump", "baseRefName": "main", "repository": { "nameWithOwner": "o/r" } }
        ] }
      }
    }"#;

    #[test]
    fn github_issues_and_pull_requests_come_back_once_each() {
        let mine = parse_github(
            &serde_json::from_str(GITHUB).unwrap(),
            &source(Kind::GitHub),
            "tobias",
        )
        .unwrap();

        // #45 is assigned and created: one row, in the group that asks
        // something of you.
        let keys: Vec<(&str, Relation)> = mine
            .tasks
            .iter()
            .map(|t| (t.key.as_str(), t.relation))
            .collect();
        assert_eq!(
            keys,
            [("#45", Relation::Assigned), ("#9", Relation::Created)]
        );
        let t = &mine.tasks[0];
        assert_eq!(t.id, "45");
        assert_eq!(t.status, "open", "the word GitHub uses everywhere else");
        assert_eq!(t.item_type, "bug", "the first label is the closest thing");
        assert_eq!(t.repo.as_deref(), Some("o/r"), "a comment needs it");
        assert_eq!(t.comments, Some(3));
        assert!(t.last_comment_mine, "the viewer wrote the last one");
        assert_eq!(t.session_name, "45-readme-says-the-wrong-port");
        assert_eq!(t.branch, "tobias/45-readme-says-the-wrong-port");
        assert_eq!(mine.tasks[1].last_commenter.as_deref(), Some("sam"));
        assert!(!mine.tasks[1].last_comment_mine);

        // #46 is yours even though you assigned it to yourself; #50 waits on
        // your review; the empty node is one the token could not see.
        let pulls: Vec<(&str, Relation, bool)> = mine
            .pulls
            .iter()
            .map(|p| (p.key.as_str(), p.relation, p.review_requested))
            .collect();
        assert_eq!(
            pulls,
            [
                ("#46", Relation::Created, false),
                ("#50", Relation::Assigned, true)
            ]
        );
        assert!(mine.warnings.is_empty());
    }

    /// GraphQL answers 200 either way. Errors with no data are a failure;
    /// errors beside data are said, and the data still counts.
    #[test]
    fn a_graphql_error_is_a_failure_only_without_data() {
        let refused = serde_json::json!({
            "data": null,
            "errors": [{ "message": "Bad credentials" }]
        });
        let e = parse_github(&refused, &source(Kind::GitHub), "t")
            .err()
            .unwrap();
        assert!(e.contains("Bad credentials"), "{e}");

        let partial = serde_json::json!({
            "data": { "viewer": { "login": "me" }, "assigned": { "nodes": [] } },
            "errors": [{ "message": "Resource protected by organization SAML enforcement" }]
        });
        let mine = parse_github(&partial, &source(Kind::GitHub), "t").unwrap();
        assert_eq!(mine.warnings.len(), 1);
    }

    /// Every search is the same question, scoped to the repository when the
    /// tracker names one.
    #[test]
    fn the_github_searches_are_mine_and_open() {
        let all = github_searches(None);
        assert_eq!(
            all["review"],
            "is:pr review-requested:@me is:open archived:false sort:updated-desc"
        );
        let one = github_searches(Some("o/r"));
        for alias in ["assigned", "created", "mine", "review", "assignee"] {
            let q = one[alias].as_str().unwrap();
            assert!(q.ends_with(" repo:o/r"), "{alias}: {q}");
            assert!(q.contains("is:open"), "{alias}: {q}");
        }
    }

    /// Two requests, because WIQL answers with ids only.
    #[test]
    fn a_wiql_answer_of_ids_becomes_one_detail_request_or_none() {
        let ids = parse_azure_ids(
            &serde_json::from_str(r#"{ "workItems": [{ "id": 1234 }, { "id": 99 }] }"#).unwrap(),
        );
        assert_eq!(ids, ["1234", "99"]);

        // A query that matches nothing: asking for zero ids is a 400, so the
        // second request has to be skipped rather than built.
        assert!(parse_azure_ids(&serde_json::json!({ "workItems": [] })).is_empty());
        assert!(parse_azure_ids(&serde_json::json!({})).is_empty());
    }

    const AZURE: &str = r#"{
      "count": 1,
      "value": [
        {
          "id": 1234,
          "fields": {
            "System.Id": 1234,
            "System.Title": "Order backfill throws on empty batch",
            "System.State": "Active",
            "System.WorkItemType": "Bug",
            "System.AssignedTo": { "id": "ME-1", "displayName": "You", "uniqueName": "you@example.com" }
          }
        },
        {
          "id": 77,
          "fields": {
            "System.Title": "Filed, and handed to somebody else",
            "System.State": "New",
            "System.WorkItemType": "Task",
            "System.AssignedTo": { "id": "them-2", "uniqueName": "them@example.com" }
          }
        }
      ]
    }"#;

    fn azure_me_() -> AzureMe {
        AzureMe {
            id: "me-1".into(),
            account: Some("you@example.com".into()),
        }
    }

    #[test]
    fn a_work_item_becomes_a_task_with_a_url_that_was_not_in_the_answer() {
        let tasks = parse_azure(
            &serde_json::from_str(AZURE).unwrap(),
            &source(Kind::AzureDevOps),
            &azure_me_(),
            "tobias",
        )
        .unwrap();

        assert_eq!(tasks.len(), 2);
        let t = &tasks[0];
        assert_eq!(t.key, "AB#1234");
        assert_eq!(t.relation, Relation::Assigned, "ids compare without case");
        assert_eq!(tasks[1].relation, Relation::Created);
        assert_eq!(t.id, "1234");
        assert_eq!(t.status, "Active");
        assert_eq!(t.item_type, "Bug");
        // Built rather than read: `_links` needs `$expand` and doubles the
        // payload for a url whose shape is fixed.
        assert_eq!(
            t.url,
            "https://dev.azure.com/contoso/tools/_workitems/edit/1234"
        );
        assert_eq!(
            t.branch,
            "tobias/AB#1234-order-backfill-throws-on-empty-batch"
        );
    }

    const JIRA: &str = r#"{
      "issues": [
        {
          "key": "PROJ-123",
          "fields": {
            "summary": "Add the changelog",
            "status": { "name": "In Progress", "statusCategory": { "key": "indeterminate" } },
            "issuetype": { "name": "Story" },
            "assignee": { "accountId": "me-1" }
          }
        },
        {
          "key": "PROJ-124",
          "fields": {
            "summary": "Reported, and nobody has it",
            "status": { "name": "To Do" },
            "issuetype": { "name": "Bug" },
            "assignee": null
          }
        }
      ]
    }"#;

    #[test]
    fn a_jira_issue_keeps_its_own_status_words() {
        let tasks = parse_jira(
            &serde_json::from_str(JIRA).unwrap(),
            &source(Kind::Jira),
            Some("me-1"),
            "tobias",
        )
        .unwrap();

        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].relation, Relation::Assigned);
        assert_eq!(
            tasks[1].relation,
            Relation::Created,
            "reported, not assigned"
        );
        let t = &tasks[0];
        assert_eq!(t.key, "PROJ-123");
        assert_eq!(t.id, "PROJ-123", "jira addresses writes by key");
        // Not mapped onto a scheme of ours: a renamed status is a fact about
        // somebody's process.
        assert_eq!(t.status, "In Progress");
        assert_eq!(t.item_type, "Story");
        assert_eq!(t.url, "https://example.atlassian.net/browse/PROJ-123");
        assert_eq!(t.session_name, "proj-123-add-the-changelog");
        assert_eq!(t.branch, "tobias/PROJ-123-add-the-changelog");
    }

    /// What a notification is made from: when it last changed, how many
    /// comments it has, and whose the newest is. The newest by `created`,
    /// whatever order the page came in.
    #[test]
    fn a_jira_issue_says_what_changed_and_who_commented_last() {
        let body = r#"{
          "issues": [{
            "key": "PROJ-7",
            "fields": {
              "summary": "Retry uploads",
              "status": { "name": "To Do" },
              "issuetype": { "name": "Bug" },
              "updated": "2026-09-28T10:00:00.000+0000",
              "comment": {
                "total": 3,
                "comments": [
                  { "created": "2026-09-27T09:00:00.000+0000",
                    "author": { "accountId": "me-1", "displayName": "Tobias" } },
                  { "created": "2026-09-28T09:30:00.000+0000",
                    "author": { "accountId": "them-2", "displayName": "Alex" } },
                  { "created": "2026-09-26T08:00:00.000+0000",
                    "author": { "accountId": "me-1", "displayName": "Tobias" } }
                ]
              }
            }
          }]
        }"#;
        let tasks = parse_jira(
            &serde_json::from_str(body).unwrap(),
            &source(Kind::Jira),
            Some("me-1"),
            "tobias",
        )
        .unwrap();
        let t = &tasks[0];
        assert_eq!(t.updated.as_deref(), Some("2026-09-28T10:00:00.000+0000"));
        assert_eq!(t.comments, Some(3), "the total, not the page");
        assert_eq!(t.last_commenter.as_deref(), Some("Alex"));
        assert!(!t.last_comment_mine);

        // The same answer, read as Alex: now it is theirs.
        let tasks = parse_jira(
            &serde_json::from_str(body).unwrap(),
            &source(Kind::Jira),
            Some("them-2"),
            "tobias",
        )
        .unwrap();
        assert!(tasks[0].last_comment_mine);
    }

    /// The token is the one thing a webview is never handed: it learns there
    /// is one, and that is all.
    #[test]
    fn a_webview_is_told_there_is_a_token_and_not_what_it_is() {
        let stored = Stored {
            source: source(Kind::Jira),
            token: Some("s3cret-api-token".into()),
        };
        let shown = Configured::from(&stored);
        assert!(shown.token_set);
        let json = serde_json::to_string(&shown).unwrap();
        assert!(!json.contains("s3cret"), "{json}");

        let blank = Configured::from(&Stored {
            source: source(Kind::Jira),
            token: Some("  ".into()),
        });
        assert!(!blank.token_set, "whitespace is not a token");
    }

    /// Who an Azure DevOps token belongs to, from `connectionData`.
    #[test]
    fn an_azure_token_says_whose_it_is() {
        let body = serde_json::json!({
            "authenticatedUser": {
                "id": "a9c18208-1768-682e-a929-635c8db024ca",
                "providerDisplayName": "You",
                "properties": { "Account": { "$type": "System.String", "$value": "you@example.com" } }
            }
        });
        let me = parse_azure_me(&body).unwrap();
        assert_eq!(me.id, "a9c18208-1768-682e-a929-635c8db024ca");
        assert_eq!(me.account.as_deref(), Some("you@example.com"));
        assert!(me.is(&serde_json::json!({ "uniqueName": "YOU@example.com" })));
        assert!(!me.is(&serde_json::json!({ "id": "", "uniqueName": "" })));
        assert!(parse_azure_me(&serde_json::json!({})).is_err());
    }

    /// The questions are fixed, and these are them.
    #[test]
    fn the_queries_ask_for_what_is_mine_and_not_done() {
        assert!(JIRA_MINE.contains("assignee = currentUser() OR reporter = currentUser()"));
        assert!(JIRA_MINE.contains("statusCategory != Done"));
        assert!(AZURE_MINE.contains("[System.AssignedTo] = @Me OR [System.CreatedBy] = @Me"));
        assert!(AZURE_MINE.contains("@project"));
    }

    /// The convention this whole loop exists to keep. The key keeps its case in
    /// a branch -- commit hooks and people both look for `PROJ-123` -- and
    /// loses it in a session name, which has to satisfy `validate_name`.
    #[test]
    fn a_ticket_names_its_session_and_its_branch() {
        let long = "Make the nightly reconciliation job cope with a partial batch from upstream";
        let name = session_name("PROJ-123", long);
        assert!(name.len() <= 40, "{name} is {}", name.len());
        assert!(
            crate::session::validate_name(&name).is_ok(),
            "{name}: {:?}",
            crate::session::validate_name(&name)
        );
        assert!(!name.ends_with('-'), "{name}");
        assert!(name.starts_with("proj-123-"), "{name}");

        assert_eq!(branch("", "PROJ-1", "Fix it"), "PROJ-1-fix-it");
        assert_eq!(
            branch("/tobias/", "PROJ-1", "Fix it"),
            "tobias/PROJ-1-fix-it"
        );
        // A title that survives nothing still gives a usable name.
        assert_eq!(session_name("PROJ-9", "!!!"), "proj-9");
        assert_eq!(branch("t", "PROJ-9", "  "), "t/PROJ-9");
    }

    /// A status code is not a transport failure, and each tracker says what is
    /// wrong in a different field. All three are quoted rather than replaced,
    /// because an expired token and a misspelled project are the same code.
    #[test]
    fn a_failed_request_is_read_rather_than_summarised() {
        assert_eq!(
            interpret("{\"ok\":true}\n200").unwrap(),
            serde_json::json!({ "ok": true })
        );
        // 204, which a posted comment answers with.
        assert_eq!(interpret("\n204").unwrap(), serde_json::Value::Null);

        let e = interpret("{\"message\":\"Bad credentials\"}\n401").unwrap_err();
        assert!(
            e.contains("Bad credentials") && e.contains("still valid"),
            "{e}"
        );

        // Jira's shape.
        let e = interpret("{\"errorMessages\":[\"Issue does not exist\"]}\n404").unwrap_err();
        assert!(e.contains("Issue does not exist"), "{e}");
        assert!(e.contains("config file"), "{e}");

        // Azure DevOps sends HTML for some failures, and quoting the first
        // line of it beats saying nothing.
        let e = interpret("<html><head><title>Sign in</title>\n203").unwrap_err();
        assert!(e.contains("203") || e.contains("html"), "{e}");
    }

    /// Everything interpolated into a curl config is attacker-influenced in the
    /// general case -- a ticket title, a JQL string, a token -- and an
    /// unescaped quote would end the value and start a new configuration line,
    /// which in a curl config can name a file to write.
    #[test]
    fn a_config_value_cannot_start_a_new_line() {
        let hostile = "a\" \noutput = \"/tmp/owned\" \"b";
        let quoted = quote(hostile);
        assert!(quoted.starts_with('"') && quoted.ends_with('"'));
        assert!(!quoted[1..quoted.len() - 1].contains('\n'), "{quoted}");
        assert_eq!(quoted.matches("\\\"").count(), 4, "{quoted}");
        // And the one that would escape the escaping.
        assert_eq!(quote(r"a\b"), r#""a\\b""#);
    }

    /// JQL is spaces, quotes, equals signs and parentheses, and it goes in a
    /// query string.
    #[test]
    fn jql_survives_being_a_query_parameter() {
        assert_eq!(
            urlencode("assignee = currentUser() AND status != \"Done\""),
            "assignee%20%3D%20currentUser%28%29%20AND%20status%20%21%3D%20%22Done%22"
        );
        assert_eq!(urlencode("plain-Text_1.0~"), "plain-Text_1.0~");
    }

    /// A tracker entry that cannot work says so against the config file rather
    /// than against a 404 on a timer.
    #[test]
    fn an_incomplete_tracker_entry_says_what_is_missing() {
        let mut jira = source(Kind::Jira);
        assert_eq!(jira.problem(), None);
        jira.email = None;
        assert!(jira.problem().unwrap().contains("no email"));
        jira.site = None;
        assert!(jira.problem().unwrap().contains("no site"));

        let mut azure = source(Kind::AzureDevOps);
        assert_eq!(azure.problem(), None);
        azure.project = None;
        assert!(azure.problem().unwrap().contains("no project"));

        // GitHub needs nothing beyond a token: with no repo it reads
        // everything assigned to whoever the token belongs to.
        let github = source(Kind::GitHub);
        assert_eq!(github.problem(), None);
    }

    #[test]
    fn a_tracker_kind_is_read_generously_and_refused_clearly() {
        assert_eq!(Kind::parse("GitHub").unwrap(), Kind::GitHub);
        assert_eq!(Kind::parse("azure_devops").unwrap(), Kind::AzureDevOps);
        assert_eq!(Kind::parse(" ado ").unwrap(), Kind::AzureDevOps);
        assert_eq!(Kind::parse("jira").unwrap(), Kind::Jira);
        let e = Kind::parse("linear").unwrap_err();
        assert!(e.contains("github") && e.contains("jira"), "{e}");
    }
}
