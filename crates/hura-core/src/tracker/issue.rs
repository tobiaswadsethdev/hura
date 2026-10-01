//! One ticket, read in full: what the board's card leaves out.
//!
//! The board is a search, and a search answers with a line per ticket. Reading
//! one means its description, its comments and the people on it -- which used
//! to mean opening it in a browser, leaving the window for the one thing the
//! window had just shown you a title of. This is the same read Jira's own page
//! makes, made from here with the token the board already uses.
//!
//! Jira only, for now. A GitHub issue or an Azure DevOps work item is the same
//! shape, and a reader for either is a function here; until there is one, the
//! card links out as it always did.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::doc::{self, Block};
use super::{Kind, Stored, get, jira_auth, jira_me, jira_site, string};

/// A ticket, read in full.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub tracker: String,
    pub kind: Kind,
    pub key: String,
    pub title: String,
    /// Where it is in a browser, for the one link the view keeps.
    pub url: String,
    pub status: String,
    /// `todo`, `doing` or `done`: where the status sits in its workflow, which
    /// is what its colour says. The words are the tracker's; this is ours.
    pub stage: Stage,
    pub item_type: String,
    pub priority: Option<String>,
    pub assignee: Option<Person>,
    pub reporter: Option<Person>,
    pub labels: Vec<String>,
    /// The epic or parent issue, when there is one.
    pub parent: Option<Parent>,
    /// In the tracker's own format, as on the board.
    pub created: Option<String>,
    pub updated: Option<String>,
    pub due: Option<String>,
    pub description: Vec<Block>,
    /// Oldest first, the order a conversation is read in.
    pub comments: Vec<IssueComment>,
    /// How many there are, which is more than `comments` when the tracker
    /// answered with a page of them.
    pub comments_total: u32,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "IssueStage"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Todo,
    Doing,
    Done,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "IssuePerson"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub name: String,
    /// Whether this is the token's owner -- you.
    pub me: bool,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "IssueParent"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parent {
    pub key: String,
    pub title: String,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueComment {
    pub author: Option<Person>,
    pub created: Option<String>,
    /// Only when it differs from `created`: an edit worth saying so about.
    pub edited: Option<String>,
    pub body: Vec<Block>,
}

/// The fields asked for, and no more: an issue with forty custom fields
/// answers with all of them unless told otherwise.
const FIELDS: &str = "summary,status,issuetype,priority,assignee,reporter,labels,parent,created,updated,duedate,description,comment";

/// Read one ticket from the tracker it came from.
pub fn issue(tracker: &Stored, key: &str) -> Result<Issue, String> {
    let source = &tracker.source;
    if source.kind != Kind::Jira {
        return Err(format!(
            "{} tickets open in the browser; only Jira is read here so far",
            source.kind.label()
        ));
    }
    if let Some(problem) = source.problem() {
        return Err(problem);
    }
    let token = tracker
        .token
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| format!("{}: no token yet", source.name))?;
    // It goes into a URL path, so it has to be a key and nothing else.
    if !is_key(key) {
        return Err(format!("`{key}` is not a Jira issue key"));
    }

    let me = jira_me(source, token)?;
    let url = format!(
        "{}/rest/api/3/issue/{key}?fields={FIELDS}",
        jira_site(source)
    );
    let body = get(
        &url,
        &jira_auth(source, token),
        &["Accept: application/json"],
    )?;
    parse(&body, &source.name, jira_site(source), me.as_deref())
}

/// `PROJ-123`: letters, digits and underscores, a dash, a number.
fn is_key(key: &str) -> bool {
    let Some((project, number)) = key.split_once('-') else {
        return false;
    };
    !project.is_empty()
        && project
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && project
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

fn parse(body: &Value, tracker: &str, site: &str, me: Option<&str>) -> Result<Issue, String> {
    let key = string(body, "key");
    let fields = body
        .get("fields")
        .filter(|_| !key.is_empty())
        .ok_or("jira did not answer with an issue")?;

    let person = |v: Option<&Value>| -> Option<Person> {
        let v = v.filter(|v| v.is_object())?;
        let name = string(v, "displayName");
        (!name.is_empty()).then(|| Person {
            me: me.is_some_and(|me| string(v, "accountId") == me),
            name,
        })
    };
    let text = |k: &str| Some(string(fields, k)).filter(|s| !s.is_empty());
    let status = fields.get("status");
    let comment = fields.get("comment");

    let mut comments: Vec<IssueComment> = comment
        .and_then(|c| c.get("comments"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|c| {
                    let created = Some(string(c, "created")).filter(|s| !s.is_empty());
                    let updated = Some(string(c, "updated")).filter(|s| !s.is_empty());
                    IssueComment {
                        author: person(c.get("author")),
                        edited: updated.filter(|u| Some(u) != created.as_ref()),
                        created,
                        body: c.get("body").map(doc::from_adf).unwrap_or_default(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    // By time rather than as listed: the list is a page, and its order is
    // Jira's to change.
    comments.sort_by(|a, b| a.created.cmp(&b.created));

    Ok(Issue {
        tracker: tracker.to_string(),
        kind: Kind::Jira,
        url: format!("{site}/browse/{key}"),
        title: string(fields, "summary"),
        status: status.map(|s| string(s, "name")).unwrap_or_default(),
        stage: match status
            .and_then(|s| s.get("statusCategory"))
            .map(|c| string(c, "key"))
            .as_deref()
        {
            Some("done") => Stage::Done,
            Some("indeterminate") => Stage::Doing,
            _ => Stage::Todo,
        },
        item_type: fields
            .get("issuetype")
            .map(|t| string(t, "name"))
            .unwrap_or_default(),
        priority: fields
            .get("priority")
            .map(|p| string(p, "name"))
            .filter(|s| !s.is_empty()),
        assignee: person(fields.get("assignee")),
        reporter: person(fields.get("reporter")),
        labels: fields
            .get("labels")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        parent: fields.get("parent").and_then(|p| {
            let key = string(p, "key");
            (!key.is_empty()).then(|| Parent {
                title: p
                    .get("fields")
                    .map(|f| string(f, "summary"))
                    .unwrap_or_default(),
                key,
            })
        }),
        created: text("created"),
        updated: text("updated"),
        due: text("duedate"),
        description: fields
            .get("description")
            .map(doc::from_adf)
            .unwrap_or_default(),
        comments_total: comment
            .and_then(|c| c.get("total"))
            .and_then(Value::as_u64)
            .map_or(comments.len() as u32, |t| t as u32),
        comments,
        key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::doc::Inline;

    /// `GET /rest/api/3/issue/PROJ-7`, trimmed to what this reads.
    const ISSUE: &str = r#"{
      "key": "PROJ-7",
      "fields": {
        "summary": "Add the changelog",
        "status": { "name": "In Review", "statusCategory": { "key": "indeterminate" } },
        "issuetype": { "name": "Story" },
        "priority": { "name": "High" },
        "assignee": { "displayName": "Ada Lovelace", "accountId": "me-1" },
        "reporter": { "displayName": "Grace Hopper", "accountId": "gh-2" },
        "labels": ["docs", "release"],
        "parent": { "key": "PROJ-1", "fields": { "summary": "Ship 1.0" } },
        "created": "2026-09-28T09:00:00.000+0200",
        "updated": "2026-10-01T14:30:00.000+0200",
        "duedate": null,
        "description": { "type": "doc", "version": 1, "content": [
          { "type": "paragraph", "content": [{ "type": "text", "text": "Keep a CHANGELOG.md." }] }
        ] },
        "comment": { "total": 2, "comments": [
          { "author": { "displayName": "Ada Lovelace", "accountId": "me-1" },
            "created": "2026-10-01T10:00:00.000+0200", "updated": "2026-10-01T11:00:00.000+0200",
            "body": { "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "second" }] }] } },
          { "author": { "displayName": "Grace Hopper", "accountId": "gh-2" },
            "created": "2026-09-30T10:00:00.000+0200", "updated": "2026-09-30T10:00:00.000+0200",
            "body": { "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "first" }] }] } }
        ] }
      }
    }"#;

    #[test]
    fn an_issue_reads_with_its_people_its_stage_and_its_conversation() {
        let issue = parse(
            &serde_json::from_str(ISSUE).unwrap(),
            "work",
            "https://example.atlassian.net",
            Some("me-1"),
        )
        .unwrap();
        assert_eq!(issue.key, "PROJ-7");
        assert_eq!(issue.url, "https://example.atlassian.net/browse/PROJ-7");
        assert_eq!(issue.status, "In Review");
        assert_eq!(issue.stage, Stage::Doing);
        assert_eq!(issue.priority.as_deref(), Some("High"));
        assert_eq!(
            issue.assignee,
            Some(Person {
                name: "Ada Lovelace".into(),
                me: true
            })
        );
        assert_eq!(
            issue.reporter,
            Some(Person {
                name: "Grace Hopper".into(),
                me: false
            })
        );
        assert_eq!(issue.labels, ["docs", "release"]);
        assert_eq!(
            issue.parent,
            Some(Parent {
                key: "PROJ-1".into(),
                title: "Ship 1.0".into()
            })
        );
        assert_eq!(issue.due, None);
        assert_eq!(issue.description.len(), 1);

        // Oldest first, whatever order Jira listed them in.
        let bodies: Vec<_> = issue
            .comments
            .iter()
            .map(|c| match &c.body[0] {
                Block::Paragraph { content } => match &content[0] {
                    Inline::Text { text, .. } => text.clone(),
                    _ => panic!(),
                },
                _ => panic!(),
            })
            .collect();
        assert_eq!(bodies, ["first", "second"]);
        assert_eq!(issue.comments[0].edited, None, "never edited");
        assert!(issue.comments[1].edited.is_some(), "edited an hour later");
        assert_eq!(issue.comments_total, 2);
    }

    #[test]
    fn an_unassigned_issue_with_nothing_written_still_reads() {
        let issue = parse(
            &serde_json::json!({ "key": "X-1", "fields": {
                "summary": "t", "assignee": null, "description": null,
                "status": { "name": "To Do", "statusCategory": { "key": "new" } }
            } }),
            "work",
            "https://example.atlassian.net",
            None,
        )
        .unwrap();
        assert_eq!(issue.assignee, None);
        assert!(issue.description.is_empty());
        assert!(issue.comments.is_empty());
        assert_eq!(issue.stage, Stage::Todo);
    }

    /// The key goes into a URL path, so it is checked rather than escaped.
    #[test]
    fn only_an_issue_key_is_asked_for() {
        for good in ["PROJ-7", "AB_C-12", "X2-1"] {
            assert!(is_key(good), "{good}");
        }
        for bad in [
            "",
            "PROJ",
            "PROJ-",
            "-7",
            "7-7",
            "PROJ-7/../../myself",
            "PROJ-7?x",
            "PR OJ-7",
        ] {
            assert!(!is_key(bad), "{bad}");
        }
    }
}
