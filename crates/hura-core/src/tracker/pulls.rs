//! Pull requests: the open ones you opened, and the ones waiting on you.
//!
//! Read with the GitHub and Azure DevOps trackers' own tokens, beside their
//! tickets. A pull request is not a ticket to start work on, so it has its own
//! shape rather than being a [`Task`](super::Task) with a flag: what matters
//! about one is whether its checks pass, whether it is approved and whether it
//! merges, and a ticket has none of those.
//!
//! GitHub's come from the same GraphQL request as its issues (see
//! `super::github`); this module turns each node into a row. Azure DevOps has
//! no search across a project's repositories other than its pull request list,
//! asked twice: by creator and by reviewer.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AzureMe, Kind, Relation, Source, azure_auth, get, string};

/// One open pull request.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    /// Which configured tracker this came from, by name.
    pub tracker: String,
    pub kind: Kind,
    /// The forge's own number for it.
    pub id: String,
    /// What a person calls it: `#58` on GitHub, `!26056` on Azure DevOps.
    pub key: String,
    pub title: String,
    /// Where to open it in a browser.
    pub url: String,
    /// `owner/name` on GitHub, the repository's name on Azure DevOps.
    pub repo: String,
    /// The repository's address in a browser, which is what a worktree's clone
    /// URL is compared with to say that the two are the same repository.
    pub repo_url: String,
    pub relation: Relation,
    /// Why one that is assigned is: a review asked of you, rather than the
    /// assignee field.
    pub review_requested: bool,
    /// Who opened it, as the forge shows them.
    pub author: String,
    /// The branch it merges from, without `refs/heads/`. A worktree working on
    /// the same branch is the one the pull request came from.
    pub branch: String,
    /// The branch it merges into.
    pub base: String,
    pub draft: bool,
    /// Opened from a fork, whose branch is not in this repository: a
    /// worktree here can neither check it out nor push to it.
    #[serde(default)]
    pub fork: bool,
    /// Whether the forge says it cannot be merged as it stands.
    pub conflicts: bool,
    /// Its checks as one verdict. `None` where the forge's list does not say:
    /// Azure DevOps keeps them behind a request per pull request.
    pub checks: Option<Checks>,
    /// Where its reviews stand. `None` when nobody has been asked.
    pub review: Option<Review>,
    /// How many comments it has. `None` where the list does not say.
    pub comments: Option<u32>,
    /// When it last changed; on Azure DevOps, which does not say, when it was
    /// opened.
    pub updated: Option<String>,
}

/// A pull request's checks, rolled into one.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "PullChecks"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Checks {
    Passing,
    Failing,
    Pending,
}

/// Where a pull request's reviews stand.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "PullReview"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Review {
    Approved,
    ChangesRequested,
    /// Asked for and not given yet.
    Pending,
}

// ---------------------------------------------------------------- GitHub

/// One node of a GitHub search, as a pull request. `None` for a node that is
/// not one, which a search across repositories the token cannot fully see
/// answers with as an empty object.
pub(super) fn github(
    p: &Value,
    tracker: &str,
    relation: Relation,
    review_requested: bool,
) -> Option<PullRequest> {
    let number = p.get("number")?.as_u64()?;
    let repo = p
        .get("repository")
        .map(|r| string(r, "nameWithOwner"))
        .unwrap_or_default();
    let checks = p
        .get("commits")
        .and_then(|c| c.get("nodes"))
        .and_then(|n| n.as_array())
        .and_then(|n| n.last())
        .and_then(|n| n.get("commit"))
        .and_then(|c| c.get("statusCheckRollup"))
        .map(|r| string(r, "state"))
        .and_then(|state| match state.as_str() {
            "SUCCESS" => Some(Checks::Passing),
            "FAILURE" | "ERROR" => Some(Checks::Failing),
            "PENDING" | "EXPECTED" => Some(Checks::Pending),
            _ => None,
        });
    let review = match string(p, "reviewDecision").as_str() {
        "APPROVED" => Some(Review::Approved),
        "CHANGES_REQUESTED" => Some(Review::ChangesRequested),
        "REVIEW_REQUIRED" => Some(Review::Pending),
        _ => None,
    };
    Some(PullRequest {
        tracker: tracker.to_string(),
        kind: Kind::GitHub,
        id: number.to_string(),
        key: format!("#{number}"),
        title: string(p, "title"),
        url: string(p, "url"),
        repo_url: format!("https://github.com/{repo}"),
        repo,
        relation,
        review_requested: relation == Relation::Assigned && review_requested,
        author: p
            .get("author")
            .map(|a| string(a, "login"))
            .unwrap_or_default(),
        branch: string(p, "headRefName"),
        base: string(p, "baseRefName"),
        draft: p.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        fork: p
            .get("isCrossRepository")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        // `UNKNOWN` while GitHub is still working it out, which is not a
        // conflict.
        conflicts: string(p, "mergeable") == "CONFLICTING",
        checks,
        review,
        comments: p
            .get("comments")
            .and_then(|c| c.get("totalCount"))
            .and_then(Value::as_u64)
            .map(|c| c as u32),
        updated: Some(string(p, "updatedAt")).filter(|s| !s.is_empty()),
    })
}

// ---------------------------------------------------------- Azure DevOps

/// What a refused pull request read says, with the likely reason when the
/// refusal is about the token.
///
/// A token made for the tickets has *Work Items (read)*, and pull requests
/// need *Code (read)* as well. That is the one thing about this read that is
/// new, so it is the one worth naming rather than "is the credential valid".
pub(super) fn azure_refusal(error: &str) -> String {
    if error.starts_with("401") || error.starts_with("403") {
        format!("{error}. Reading pull requests needs the token to have Code (read) too")
    } else {
        error.to_string()
    }
}

/// The project's open pull requests that are yours or wait on you.
pub(super) fn azure(
    source: &Source,
    token: &str,
    me: &AzureMe,
) -> Result<Vec<PullRequest>, String> {
    let org = source.org.as_deref().unwrap_or_default();
    let project = source.project.as_deref().unwrap_or_default();
    let auth = azure_auth(token);
    let list = format!(
        "https://dev.azure.com/{org}/{project}/_apis/git/pullrequests?searchCriteria.status=active&$top=100&api-version=7.1"
    );
    let created = get(
        &format!("{list}&searchCriteria.creatorId={}", me.id),
        &auth,
        &[],
    )?;
    let reviewing = get(
        &format!("{list}&searchCriteria.reviewerId={}", me.id),
        &auth,
        &[],
    )?;
    parse_azure(&created, &reviewing, source, me)
}

/// Both answers, flattened, yours first.
pub(super) fn parse_azure(
    created: &Value,
    reviewing: &Value,
    source: &Source,
    me: &AzureMe,
) -> Result<Vec<PullRequest>, String> {
    let list = |v: &Value| -> Result<Vec<Value>, String> {
        v.get("value")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| "azure devops did not answer with pull requests".to_string())
    };
    let mut out: Vec<PullRequest> = Vec::new();
    for p in list(created)? {
        if let Some(pull) = azure_pull(&p, source, Relation::Created) {
            out.push(pull);
        }
    }
    for p in list(reviewing)? {
        // Only while it waits on you: once you have voted, it is the author's
        // move. That is what GitHub means by a review request too, which goes
        // away when the review is given.
        let voted = p
            .get("reviewers")
            .and_then(Value::as_array)
            .is_some_and(|rs| {
                rs.iter()
                    .any(|r| me.is(r) && r.get("vote").and_then(Value::as_i64).unwrap_or(0) != 0)
            });
        let yours = p.get("createdBy").is_some_and(|c| me.is(c));
        if voted || yours {
            continue;
        }
        let Some(pull) = azure_pull(&p, source, Relation::Assigned) else {
            continue;
        };
        if !out.iter().any(|o| o.repo == pull.repo && o.id == pull.id) {
            out.push(pull);
        }
    }
    Ok(out)
}

fn azure_pull(p: &Value, source: &Source, relation: Relation) -> Option<PullRequest> {
    let id = p.get("pullRequestId")?.as_u64()?;
    let repository = p.get("repository")?;
    let repo = string(repository, "name");
    // The list leaves `webUrl` out where a single pull request has it, so it
    // is built when it is missing. The project is the repository's own, which
    // is the configured one unless the list strayed.
    let repo_url = Some(string(repository, "webUrl"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let org = source.org.as_deref().unwrap_or_default();
            let project = repository
                .get("project")
                .map(|p| string(p, "name"))
                .filter(|s| !s.is_empty())
                .or_else(|| source.project.clone())
                .unwrap_or_default();
            format!("https://dev.azure.com/{org}/{project}/_git/{repo}")
        });
    let votes: Vec<i64> = p
        .get("reviewers")
        .and_then(Value::as_array)
        .map(|rs| {
            rs.iter()
                .map(|r| r.get("vote").and_then(Value::as_i64).unwrap_or(0))
                .collect()
        })
        .unwrap_or_default();
    // 10 approved, 5 approved with suggestions, 0 no vote, -5 waiting for the
    // author, -10 rejected. Any objection outweighs any approval, the way the
    // pull request page itself reads.
    let review = if votes.iter().any(|v| *v < 0) {
        Some(Review::ChangesRequested)
    } else if votes.iter().any(|v| *v > 0) {
        Some(Review::Approved)
    } else if !votes.is_empty() {
        Some(Review::Pending)
    } else {
        None
    };
    // A string over REST, and a number from the SDKs that wrap it: 2 is
    // `conflicts` in `PullRequestAsyncStatus`.
    let conflicts = match p.get("mergeStatus") {
        Some(Value::String(s)) => s == "conflicts",
        Some(Value::Number(n)) => n.as_u64() == Some(2),
        _ => false,
    };
    let refs = |key: &str| {
        let name = string(p, key);
        name.strip_prefix("refs/heads/")
            .map(str::to_string)
            .unwrap_or(name)
    };
    Some(PullRequest {
        tracker: source.name.clone(),
        kind: Kind::AzureDevOps,
        id: id.to_string(),
        key: format!("!{id}"),
        title: string(p, "title"),
        url: format!("{repo_url}/pullrequest/{id}"),
        repo,
        repo_url,
        relation,
        review_requested: relation == Relation::Assigned,
        author: p
            .get("createdBy")
            .map(|c| string(c, "displayName"))
            .unwrap_or_default(),
        branch: refs("sourceRefName"),
        base: refs("targetRefName"),
        draft: p.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        fork: p.get("forkSource").is_some_and(|f| !f.is_null()),
        conflicts,
        checks: None,
        review,
        comments: None,
        updated: Some(string(p, "creationDate")).filter(|s| !s.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A node as GitHub's search answers with it, measured against a real
    /// answer and trimmed to what is read.
    const GITHUB_PULL: &str = r#"{
      "number": 58,
      "title": "A refresh no longer writes over a create that finished meanwhile",
      "url": "https://github.com/o/hura/pull/58",
      "isDraft": false,
      "updatedAt": "2026-10-09T08:34:59Z",
      "headRefName": "tobias/refresh-race",
      "baseRefName": "main",
      "mergeable": "UNKNOWN",
      "reviewDecision": "CHANGES_REQUESTED",
      "author": { "login": "tobias" },
      "repository": { "nameWithOwner": "o/hura" },
      "comments": { "totalCount": 2 },
      "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "state": "FAILURE" } } }] }
    }"#;

    #[test]
    fn a_github_pull_request_says_its_checks_review_and_branch() {
        let p = github(
            &serde_json::from_str(GITHUB_PULL).unwrap(),
            "github",
            Relation::Created,
            false,
        )
        .unwrap();
        assert_eq!(p.key, "#58");
        assert_eq!(p.repo, "o/hura");
        assert_eq!(p.repo_url, "https://github.com/o/hura");
        assert_eq!(p.branch, "tobias/refresh-race");
        assert_eq!(p.base, "main");
        assert_eq!(p.checks, Some(Checks::Failing));
        assert_eq!(p.review, Some(Review::ChangesRequested));
        assert!(!p.conflicts, "UNKNOWN is not a conflict");
        assert_eq!(p.comments, Some(2));
        assert!(!p.review_requested, "only an assigned one can be");
    }

    #[test]
    fn a_github_pull_request_with_nothing_run_has_no_verdict() {
        let mut v: Value = serde_json::from_str(GITHUB_PULL).unwrap();
        v["commits"] =
            serde_json::json!({ "nodes": [{ "commit": { "statusCheckRollup": null } }] });
        v["reviewDecision"] = Value::Null;
        v["mergeable"] = "CONFLICTING".into();
        let p = github(&v, "github", Relation::Assigned, true).unwrap();
        assert_eq!(p.checks, None);
        assert_eq!(p.review, None);
        assert!(p.conflicts);
        assert!(p.review_requested);

        // A node the token could not see is an empty object.
        assert!(github(&serde_json::json!({}), "github", Relation::Created, false).is_none());
    }

    /// The pull request list's shape, trimmed to what is read. `status` and
    /// `mergeStatus` are strings over REST.
    fn azure_pull_json(id: u64, author: &str, reviewers: Value) -> Value {
        serde_json::json!({
            "pullRequestId": id,
            "title": format!("Change {id}"),
            "status": "active",
            "createdBy": { "id": author, "displayName": "Someone", "uniqueName": "someone@example.com" },
            "creationDate": "2026-10-30T06:11:46.276Z",
            "sourceRefName": "refs/heads/tobias/CODE-1-thing",
            "targetRefName": "refs/heads/master",
            "mergeStatus": "conflicts",
            "isDraft": true,
            "repository": {
                "id": "r-1",
                "name": "Inet.Server",
                "project": { "id": "p-1", "name": "tools" }
            },
            "reviewers": reviewers
        })
    }

    #[test]
    fn an_azure_pull_request_is_addressed_by_its_repository_and_number() {
        let created = serde_json::json!({
            "value": [azure_pull_json(26056, "me-1", serde_json::json!([{ "id": "them-2", "vote": 10 }]))]
        });
        let none = serde_json::json!({ "value": [] });
        let pulls = parse_azure(&created, &none, &azure_source(), &me()).unwrap();
        assert_eq!(pulls.len(), 1);
        let p = &pulls[0];
        assert_eq!(p.key, "!26056");
        assert_eq!(p.relation, Relation::Created);
        assert_eq!(
            p.url,
            "https://dev.azure.com/contoso/tools/_git/Inet.Server/pullrequest/26056"
        );
        assert_eq!(
            p.repo_url,
            "https://dev.azure.com/contoso/tools/_git/Inet.Server"
        );
        assert_eq!(p.branch, "tobias/CODE-1-thing", "without refs/heads/");
        assert_eq!(p.base, "master");
        assert!(p.draft);
        assert!(p.conflicts);
        assert_eq!(p.review, Some(Review::Approved));
        assert_eq!(p.checks, None, "the list does not say");
    }

    /// Asked of you and not answered yet. One you voted on is the author's
    /// move, and one you opened is yours, not a review.
    #[test]
    fn an_azure_review_waits_on_you_until_you_vote() {
        let none = serde_json::json!({ "value": [] });
        let reviewing = serde_json::json!({ "value": [
            azure_pull_json(1, "them-2", serde_json::json!([{ "id": "me-1", "vote": 0 }])),
            azure_pull_json(2, "them-2", serde_json::json!([{ "id": "me-1", "vote": 10 }])),
            azure_pull_json(3, "them-2", serde_json::json!([{ "id": "ME-1", "vote": -5 }])),
            azure_pull_json(4, "me-1", serde_json::json!([{ "id": "me-1", "vote": 0 }])),
            azure_pull_json(5, "them-2", serde_json::json!([
                { "id": "x", "uniqueName": "YOU@example.com", "vote": 0 },
                { "id": "them-3", "vote": -10 }
            ])),
        ]});
        let pulls = parse_azure(&none, &reviewing, &azure_source(), &me()).unwrap();
        let keys: Vec<&str> = pulls.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["!1", "!5"]);
        assert!(
            pulls
                .iter()
                .all(|p| p.relation == Relation::Assigned && p.review_requested)
        );
        assert_eq!(pulls[0].review, Some(Review::Pending));
        assert_eq!(pulls[1].review, Some(Review::ChangesRequested));
    }

    /// What an SDK hands on is numbers where REST has strings.
    #[test]
    fn a_numeric_merge_status_is_read_too() {
        let mut p = azure_pull_json(7, "me-1", serde_json::json!([]));
        p["mergeStatus"] = 2.into();
        let created = serde_json::json!({ "value": [p] });
        let none = serde_json::json!({ "value": [] });
        let pulls = parse_azure(&created, &none, &azure_source(), &me()).unwrap();
        assert!(pulls[0].conflicts);
        assert_eq!(pulls[0].review, None, "nobody asked");
    }

    #[test]
    fn a_refused_read_names_the_scope_it_needs() {
        let said =
            azure_refusal("401: TF400813: not authorized (is the stored credential still valid?)");
        assert!(said.contains("Code (read)"), "{said}");
        assert_eq!(
            azure_refusal("the request failed: timeout"),
            "the request failed: timeout"
        );
    }
}
