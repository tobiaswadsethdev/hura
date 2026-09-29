//! Tickets: what your trackers' filters match, read over REST.
//!
//! **Read by the client, with the client's own tokens.** A tracker is set up in
//! the desktop application and stored on the machine it runs on, and the
//! requests go from there; the server holds no tracker, no token and no ticket.
//! A server is where sandboxes run, and a Jira token is not something a
//! sandbox -- or the machine hosting them -- has any business holding.
//!
//! REST is for what the *interface* shows: a list, on a timer, rendered as
//! rows. What an *agent* does with a ticket -- commenting, moving it -- goes
//! through an MCP server when the agent decides to, which is a different
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
    /// What the tickets screen calls it. Defaults to the kind, which is right
    /// until somebody has two Jira sites.
    pub name: String,
    /// GitHub: `owner/name`, or `None` for everything assigned to you.
    pub repo: Option<String>,
    /// Azure DevOps organisation, and the project the query runs in.
    pub org: Option<String>,
    pub project: Option<String>,
    /// Jira site, `https://your-org.atlassian.net`, and the account the token
    /// belongs to -- Jira Cloud is Basic auth with the email as the username.
    pub site: Option<String>,
    pub email: Option<String>,
    /// Named queries, each its own section of the tickets screen: "ready to
    /// start", "assigned to me". Empty means the one [`DEFAULT_FILTER`].
    ///
    /// Jira and Azure DevOps only, because they are the two with a query
    /// language this reads. A GitHub tracker lists what is assigned to you.
    pub filters: Vec<Filter>,
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
            filters: Vec::new(),
        }
    }
}

/// One named query on a tracker.
///
/// A name rather than only a query, because the name is what the inbox shows
/// and what a notification says a ticket turned up in: `PROJ-12 is ready to
/// start` is a sentence, and a line of JQL is not.
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, rename = "TrackerFilter")
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    pub name: String,
    /// JQL for Jira, WIQL for Azure DevOps.
    pub query: String,
}

/// What a tracker with no query of its own asks for: assigned to me, not done.
///
/// Per kind, and public, because a client editing a tracker's filters starts
/// from the one it has been reading -- and that has to be the same text the
/// server would have sent, not a client's guess at it.
pub fn default_query(kind: Kind) -> &'static str {
    match kind {
        // `statusCategory != Done` rather than a list of status names: every
        // Jira project renames its statuses and none of them rename the
        // categories.
        Kind::Jira => "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC",
        Kind::AzureDevOps => {
            "SELECT [System.Id] FROM WorkItems \
             WHERE [System.AssignedTo] = @Me \
             AND [System.State] NOT IN ('Closed', 'Done', 'Removed', 'Resolved') \
             ORDER BY [System.ChangedDate] DESC"
        }
        // Not a query: GitHub's `/issues` is asked with parameters.
        Kind::GitHub => "",
    }
}

/// The name of the filter a tracker without any has.
pub const DEFAULT_FILTER: &str = "assigned to me";

impl Source {
    /// The filters this tracker actually runs: its own, or the kind's default.
    pub fn effective(&self) -> Vec<Filter> {
        if !self.filters.is_empty() {
            return self.filters.clone();
        }
        vec![Filter {
            name: DEFAULT_FILTER.to_string(),
            query: default_query(self.kind).to_string(),
        }]
    }

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
            filters: self
                .filters
                .iter()
                .map(|f| Filter {
                    name: f.name.trim().to_string(),
                    query: f.query.trim().to_string(),
                })
                .collect(),
        }
    }

    /// What is missing, if anything.
    pub fn problem(&self) -> Option<String> {
        if let Some(problem) = self.filter_problem() {
            return Some(problem);
        }
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

    fn filter_problem(&self) -> Option<String> {
        if self.filters.is_empty() {
            return None;
        }
        if self.kind == Kind::GitHub {
            return Some(format!(
                "`{}` is a github tracker, which takes no filters: it lists what is assigned to you",
                self.name
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for f in &self.filters {
            if f.name.trim().is_empty() {
                return Some(format!("`{}` has a filter with no name", self.name));
            }
            if f.query.trim().is_empty() {
                return Some(format!(
                    "`{}` has a filter `{}` with no query",
                    self.name, f.name
                ));
            }
            // The inbox's sections and a notification's sentence both name a
            // filter, and two with one name would be two sections nobody can
            // tell apart.
            if !seen.insert(f.name.trim()) {
                return Some(format!(
                    "`{}` has two filters called `{}`",
                    self.name, f.name
                ));
            }
        }
        None
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
    /// file. Carried because `/issues` spans repositories: a comment has to go
    /// to the one the issue is actually in, which the entry may not name.
    pub repo: Option<String>,
    /// Which of the tracker's filters this row came from. A ticket two filters
    /// match is two rows, one in each section.
    pub filter: String,
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

/// The inbox, and whatever could not be read.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inbox {
    pub tasks: Vec<Task>,
    /// One per tracker that failed, in words. A tracker that cannot be read is
    /// a row missing from a list, which is invisible -- so it is said out loud
    /// rather than left as an empty inbox.
    pub warnings: Vec<String>,
    /// Every filter that was read, in order, whether or not it matched
    /// anything.
    ///
    /// What lets a client tell "nothing matches" from "could not ask": a
    /// filter missing from here was not read, so a ticket missing from it has
    /// not left it -- and one appearing in it after an outage has not just
    /// arrived.
    #[serde(default)]
    pub read: Vec<FilterRead>,
}

/// One filter an inbox read.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterRead {
    pub tracker: String,
    pub filter: String,
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
    /// The filters it runs, the implied default included. What an editor
    /// starts from, so the first filter somebody adds does not silently
    /// replace the "assigned to me" they have been reading.
    pub filters: Vec<Filter>,
}

impl From<&Stored> for Configured {
    fn from(s: &Stored) -> Self {
        Configured {
            token_set: s.token.as_deref().is_some_and(|t| !t.trim().is_empty()),
            filters: s.source.effective(),
            source: s.source.clone(),
        }
    }
}

/// Read every tracker, one filter at a time.
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
        let reader = match Reader::new(source, token) {
            Ok(reader) => reader,
            Err(e) => {
                out.warnings.push(format!("{}: {e}", source.name));
                continue;
            }
        };
        for filter in source.effective() {
            match reader.read(&filter, branch_prefix) {
                Ok(mut tasks) => {
                    for t in &mut tasks {
                        t.filter = filter.name.clone();
                    }
                    out.tasks.append(&mut tasks);
                    out.read.push(FilterRead {
                        tracker: source.name.clone(),
                        filter: filter.name.clone(),
                    });
                }
                Err(e) => out
                    .warnings
                    .push(format!("{} · {}: {e}", source.name, filter.name)),
            }
        }
    }
    out
}

/// One tracker, with its credential, for as many filters as it has.
struct Reader<'a> {
    source: &'a Source,
    token: &'a str,
    /// Jira's account id for the credential's owner, asked once per read
    /// rather than once per filter.
    me: Option<String>,
}

impl<'a> Reader<'a> {
    /// Refused when Jira will not say who the token belongs to. That request
    /// is the one every search would have failed the same way, so it stands
    /// for all of them: one warning, rather than one per filter each waiting
    /// out the timeout against a site that is not answering.
    fn new(source: &'a Source, token: &'a str) -> Result<Self, String> {
        let me = match source.kind {
            Kind::Jira => jira_me(source, token)?,
            _ => None,
        };
        Ok(Reader { source, token, me })
    }

    fn read(&self, filter: &Filter, prefix: &str) -> Result<Vec<Task>, String> {
        match self.source.kind {
            Kind::GitHub => github(self.source, self.token, prefix),
            Kind::AzureDevOps => azure(self.source, self.token, &filter.query, prefix),
            Kind::Jira => jira(
                self.source,
                self.token,
                &filter.query,
                self.me.as_deref(),
                prefix,
            ),
        }
    }
}

// ---------------------------------------------------------------- GitHub

fn github(source: &Source, token: &str, prefix: &str) -> Result<Vec<Task>, String> {
    let auth = format!("Bearer {token}");
    // The repository's own issues when one is named, and everything assigned to
    // the token's owner otherwise -- which is what `/issues` means, and is the
    // whole inbox in one request.
    let url = match &source.repo {
        Some(repo) => format!(
            "https://api.github.com/repos/{repo}/issues?assignee=@me&state=open&per_page=50"
        ),
        None => "https://api.github.com/issues?filter=assigned&state=open&per_page=50".to_string(),
    };
    let body = get(&url, &auth, &["Accept: application/vnd.github+json"])?;
    parse_github(&body, source, prefix)
}

/// GitHub's answer, flattened. Separate from the request so every shape it
/// sends can be asserted on without a network -- which is the only way to have
/// any confidence in a reader of somebody else's JSON.
fn parse_github(
    body: &serde_json::Value,
    source: &Source,
    prefix: &str,
) -> Result<Vec<Task>, String> {
    let items = body.as_array().ok_or("github did not answer with a list")?;

    Ok(items
        .iter()
        // `/issues` returns pull requests too -- they are issues to GitHub --
        // and a pull request is not a task to start work on.
        .filter(|i| i.get("pull_request").is_none())
        .filter_map(|i| {
            let number = i.get("number")?.as_u64()?;
            let title = string(i, "title");
            let repo = i
                .get("repository")
                .map(|r| string(r, "full_name"))
                .filter(|s| !s.is_empty())
                .or_else(|| source.repo.clone())
                .unwrap_or_default();
            let key = format!("#{number}");
            Some(Task {
                tracker: source.name.clone(),
                kind: Kind::GitHub,
                id: number.to_string(),
                session_name: session_name(&key, &title),
                branch: branch(prefix, &key, &title),
                key,
                title,
                url: string(i, "html_url"),
                status: string(i, "state"),
                item_type: label_of(i).unwrap_or_default(),
                repo: (!repo.is_empty()).then_some(repo),
                filter: String::new(),
                updated: Some(string(i, "updated_at")).filter(|s| !s.is_empty()),
                comments: i.get("comments").and_then(|c| c.as_u64()).map(|c| c as u32),
                // `/issues` counts comments but does not say whose the last
                // one is; finding out is a request per issue.
                last_commenter: None,
                last_comment_mine: false,
            })
        })
        .collect())
}

/// The first label, which is the closest GitHub has to a type.
fn label_of(issue: &serde_json::Value) -> Option<String> {
    let labels = issue.get("labels")?.as_array()?;
    labels.first().map(|l| string(l, "name"))
}

// ---------------------------------------------------------- Azure DevOps

fn azure(source: &Source, token: &str, wiql: &str, prefix: &str) -> Result<Vec<Task>, String> {
    let org = source.org.as_deref().unwrap_or_default();
    let project = source.project.as_deref().unwrap_or_default();
    let auth = azure_auth(token);

    // Two requests, and there is no way around it: WIQL answers with ids only.
    let query_url =
        format!("https://dev.azure.com/{org}/{project}/_apis/wit/wiql?api-version=7.1&$top=50");
    let body = serde_json::json!({ "query": wiql });
    let answer = post(&query_url, &auth, &body.to_string(), JSON)?;

    let ids = parse_azure_ids(&answer);
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    // `fields` rather than the whole work item: a work item with its history is
    // tens of kilobytes and four of those fields are the whole row.
    let detail_url = format!(
        "https://dev.azure.com/{org}/{project}/_apis/wit/workitems?ids={}&fields=System.Id,System.Title,System.State,System.WorkItemType,System.ChangedDate,System.CommentCount&api-version=7.1",
        ids.join(",")
    );
    let detail = get(&detail_url, &auth, &[])?;
    parse_azure(&detail, source, prefix)
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
                filter: String::new(),
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
/// it. Emails are hidden on most Jira Cloud sites, so the configured one
/// cannot be matched against an author.
///
/// Also the cheapest question that says whether the site and the token work
/// at all: a failure here is the answer for every filter too.
fn jira_me(source: &Source, token: &str) -> Result<Option<String>, String> {
    let url = format!("{}/rest/api/3/myself", jira_site(source));
    let body = get(
        &url,
        &jira_auth(source, token),
        &["Accept: application/json"],
    )?;
    Ok(Some(string(&body, "accountId")).filter(|s| !s.is_empty()))
}

fn jira(
    source: &Source,
    token: &str,
    jql: &str,
    me: Option<&str>,
    prefix: &str,
) -> Result<Vec<Task>, String> {
    let site = jira_site(source);
    // `/search/jql`, not `/search`: the older endpoint is deprecated on Jira
    // Cloud and answers 410 on newer sites. `comment` is what makes a new
    // comment noticeable without a request per issue.
    let url = format!(
        "{site}/rest/api/3/search/jql?jql={}&fields=summary,status,issuetype,updated,comment&maxResults=50",
        urlencode(jql)
    );
    let body = get(
        &url,
        &jira_auth(source, token),
        &["Accept: application/json"],
    )?;
    parse_jira(&body, source, me, prefix)
}

fn parse_jira(
    body: &serde_json::Value,
    source: &Source,
    me: Option<&str>,
    prefix: &str,
) -> Result<Vec<Task>, String> {
    let site = source
        .site
        .as_deref()
        .unwrap_or_default()
        .trim_end_matches('/');
    let issues = body
        .get("issues")
        .and_then(|i| i.as_array())
        .ok_or("jira did not answer with issues")?;

    Ok(issues
        .iter()
        .filter_map(|i| {
            let key = string(i, "key");
            if key.is_empty() {
                return None;
            }
            let fields = i.get("fields")?;
            let title = string(fields, "summary");
            let comment = fields.get("comment");
            // The newest by `created`, rather than the last in the list: the
            // list is a page, and its order is Jira's to change.
            let last = comment
                .and_then(|c| c.get("comments"))
                .and_then(|c| c.as_array())
                .and_then(|list| list.iter().max_by_key(|c| string(c, "created")));
            Some(Task {
                tracker: source.name.clone(),
                kind: Kind::Jira,
                // The key *is* the id in Jira's API: every write-back path
                // takes an issue key or its numeric id interchangeably.
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
                filter: String::new(),
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
        })
        .collect())
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
        config.push_str(&format!(
            "header = {}\n",
            quote(&format!("Content-Type: {content_type}"))
        ));
        config.push_str(&format!("data-raw = {}\n", quote(body)));
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
            filters: Vec::new(),
        }
    }

    /// The shape `GET /issues?filter=assigned` answers with, trimmed to the
    /// fields this reads -- including a pull request, which GitHub returns from
    /// the same endpoint because a pull request *is* an issue to it.
    const GITHUB: &str = r#"[
      {
        "number": 45,
        "title": "Readme says the wrong port",
        "html_url": "https://github.com/o/r/issues/45",
        "state": "open",
        "labels": [{ "name": "bug" }, { "name": "docs" }],
        "repository": { "full_name": "o/r" }
      },
      {
        "number": 46,
        "title": "Bump the lockfile",
        "html_url": "https://github.com/o/r/pull/46",
        "state": "open",
        "labels": [],
        "repository": { "full_name": "o/r" },
        "pull_request": { "url": "https://api.github.com/repos/o/r/pulls/46" }
      }
    ]"#;

    #[test]
    fn a_github_issue_becomes_a_task_and_a_pull_request_does_not() {
        let tasks = parse_github(
            &serde_json::from_str(GITHUB).unwrap(),
            &source(Kind::GitHub),
            "tobias",
        )
        .unwrap();

        assert_eq!(tasks.len(), 1, "a pull request is not a task: {tasks:?}");
        let t = &tasks[0];
        assert_eq!(t.key, "#45");
        assert_eq!(t.id, "45");
        assert_eq!(t.title, "Readme says the wrong port");
        assert_eq!(t.status, "open");
        assert_eq!(t.item_type, "bug", "the first label is the closest thing");
        assert_eq!(t.repo.as_deref(), Some("o/r"), "a comment needs it");
        assert_eq!(t.session_name, "45-readme-says-the-wrong-port");
        assert_eq!(t.branch, "tobias/45-readme-says-the-wrong-port");
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
            "System.WorkItemType": "Bug"
          }
        }
      ]
    }"#;

    #[test]
    fn a_work_item_becomes_a_task_with_a_url_that_was_not_in_the_answer() {
        let tasks = parse_azure(
            &serde_json::from_str(AZURE).unwrap(),
            &source(Kind::AzureDevOps),
            "tobias",
        )
        .unwrap();

        assert_eq!(tasks.len(), 1);
        let t = &tasks[0];
        assert_eq!(t.key, "AB#1234");
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
            "issuetype": { "name": "Story" }
          }
        }
      ]
    }"#;

    #[test]
    fn a_jira_issue_keeps_its_own_status_words() {
        let tasks = parse_jira(
            &serde_json::from_str(JIRA).unwrap(),
            &source(Kind::Jira),
            None,
            "tobias",
        )
        .unwrap();

        assert_eq!(tasks.len(), 1);
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

    /// A tracker with no filters runs the kind's default, and its own filters
    /// replace it rather than sitting beside it.
    #[test]
    fn a_tracker_without_filters_runs_the_default() {
        let s = source(Kind::Jira);
        let f = s.effective();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, DEFAULT_FILTER);
        assert_eq!(f[0].query, default_query(Kind::Jira));

        let s = Source {
            filters: vec![
                Filter {
                    name: "ready".into(),
                    query: "status = Ready".into(),
                },
                Filter {
                    name: "mine".into(),
                    query: "assignee = currentUser()".into(),
                },
            ],
            ..source(Kind::Jira)
        };
        let names: Vec<String> = s.effective().into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["ready", "mine"]);
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

    #[test]
    fn filters_that_cannot_work_are_refused() {
        let with = |kind: Kind, filters: Vec<Filter>| Source {
            filters,
            ..source(kind)
        };
        let f = |name: &str, query: &str| Filter {
            name: name.into(),
            query: query.into(),
        };
        let dup = with(Kind::Jira, vec![f("a", "x = 1"), f("a", "y = 2")]);
        assert!(dup.problem().unwrap().contains("two filters called `a`"));
        let empty = with(Kind::Jira, vec![f("a", "  ")]);
        assert!(empty.problem().unwrap().contains("no query"));
        let github = with(Kind::GitHub, vec![f("a", "x")]);
        assert!(github.problem().unwrap().contains("takes no filters"));
        assert_eq!(with(Kind::Jira, vec![f("a", "x = 1")]).problem(), None);
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
