//! One ticket, read in full: what the sidebar's row leaves out.
//!
//! The sidebar is a search, and a search answers with a line per ticket.
//! Reading one means its description, its comments and the people on it,
//! which used to mean opening it in a browser and leaving the window for the
//! one thing the window had just shown you a title of. This is the same read
//! Jira's own page makes, made from here with the token the sidebar already
//! uses.
//!
//! **Images** in a description or a comment are attachments of the ticket's,
//! which ADF names by file name. [`parse`] matches each to the attachment of
//! that name, and [`attachment`] fetches one: with the token, since Jira
//! serves nothing to a request without one, and through this side rather than
//! the window, which loads nothing from the network.
//!
//! Jira only, for now. A GitHub issue or an Azure DevOps work item is the same
//! shape, and a reader for either is a function here; until there is one, the
//! row links out as it always did.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::doc::{self, Block};
use super::markdown::{Markdown, to_markdown};
use super::{Kind, Stored, get, get_bytes, jira_api, jira_auth, jira_me, jira_site, string};

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
    /// In the tracker's own format, as in the sidebar.
    pub created: Option<String>,
    pub updated: Option<String>,
    pub due: Option<String>,
    pub description: Vec<Block>,
    /// The project's own fields that have a value: story points, the sprint,
    /// a team. By name.
    #[serde(default)]
    pub custom: Vec<CustomField>,
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

#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, rename = "IssueCustomField")
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomField {
    /// `customfield_10016`.
    pub id: String,
    /// What the project calls it: `Story Points`.
    pub name: String,
    /// The value as words. For rich text, its text without the formatting.
    pub value: String,
    /// Rich text, for a client to draw instead of `value`. Empty otherwise.
    pub doc: Vec<Block>,
}

/// An image attached to a ticket, as the bytes it is.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "IssueImage"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    /// `image/png`: from the bytes, not from what Jira said they were.
    pub mime: String,
    /// The bytes, in base64, which JSON can carry and `data:` URLs take.
    pub data: String,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueComment {
    /// Jira's id for it, which an edit or a delete is addressed to.
    pub id: String,
    pub author: Option<Person>,
    pub created: Option<String>,
    /// Only when it differs from `created`: an edit worth saying so about.
    pub edited: Option<String>,
    pub body: Vec<Block>,
    /// The body as Markdown, for a comment that is yours to edit -- and what
    /// Markdown could not hold of it. `None` for anybody else's.
    pub markdown: Option<Markdown>,
}

/// Where a status sits in its workflow, from the category Jira files it
/// under -- which every project renames nothing of.
pub(super) fn stage_of(status: Option<&Value>) -> Stage {
    match status
        .and_then(|s| s.get("statusCategory"))
        .map(|c| string(c, "key"))
        .as_deref()
    {
        Some("done") => Stage::Done,
        Some("indeterminate") => Stage::Doing,
        _ => Stage::Todo,
    }
}

/// Everything but what is never shown here and can be long: the custom
/// fields are each project's own, so they cannot be named in advance.
/// `names` and `schema` say what each one is called and of what type.
const FIELDS: &str = "*all,-worklog,-issuelinks,-subtasks,-watches,-votes";
const EXPAND: &str = "names,schema";

/// Custom fields that are the tool's bookkeeping rather than anything a
/// person filled in, by their schema's `custom` type.
const NOT_SHOWN: &[&str] = &[
    // An opaque sort key: `0|i0001b:`.
    "com.pyxis.greenhopper.jira:gh-lexo-rank",
    // A JSON summary of branches and commits, as a string: `{}`.
    "com.atlassian.jira.plugins.jira-development-integration-plugin:devsummarycf",
    "com.atlassian.jira.plugins.jira-development-integration-plugin:devsummary",
    // A board's colour for the epic: `purple`.
    "com.pyxis.greenhopper.jira:jsw-issue-color",
    "com.pyxis.greenhopper.jira:gh-epic-color",
    // A clock's state, whose only name is the SLA's own.
    "com.atlassian.servicedesk:sd-sla-field",
];

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
    let token = token(tracker)?;
    // It goes into a URL path, so it has to be a key and nothing else.
    if !is_key(key) {
        return Err(format!("`{key}` is not a Jira issue key"));
    }

    let me = jira_me(source, token)?;
    let url = format!(
        "{}/rest/api/3/issue/{key}?fields={FIELDS}&expand={EXPAND}",
        jira_api(source)
    );
    let body = get(
        &url,
        &jira_auth(source, token),
        &["Accept: application/json"],
    )?;
    parse(&body, &source.name, jira_site(source), me.as_deref())
}

/// One of a ticket's images, by the attachment id [`parse`] put on its block.
/// Only an image: anything else is refused rather than handed to a window to
/// guess the type of.
pub fn attachment(tracker: &Stored, id: &str) -> Result<Image, String> {
    let source = &tracker.source;
    if source.kind != Kind::Jira {
        return Err(format!(
            "{} attachments are not read here; only Jira's",
            source.kind.label()
        ));
    }
    if let Some(problem) = source.problem() {
        return Err(problem);
    }
    let token = token(tracker)?;
    // It goes into a URL path: a number and nothing else.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("`{id}` is not a Jira attachment id"));
    }
    // `redirect=false` has Jira answer with the bytes rather than send the
    // request on to its media service: hura cloud follows no redirect.
    let bytes = get_bytes(
        &format!(
            "{}/rest/api/3/attachment/content/{id}?redirect=false",
            jira_api(source)
        ),
        &jira_auth(source, token),
    )?;
    image_of(&bytes)
}

/// Bytes as an image, or why they are not one this can show.
pub(super) fn image_of(bytes: &[u8]) -> Result<Image, String> {
    let mime = sniff(bytes).ok_or("that is not an image this can show")?;
    Ok(Image {
        mime: mime.to_string(),
        data: crate::skills::base64(bytes),
    })
}

fn token(tracker: &Stored) -> Result<&str, String> {
    tracker
        .token
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| format!("{}: no token yet", tracker.source.name))
}

/// What image the bytes are, from how they begin. Only the formats every
/// client draws; not SVG, which is a document rather than a picture.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("image/webp"),
        _ => None,
    }
}

/// `PROJ-123`: letters, digits and underscores, a dash, a number.
pub(super) fn is_key(key: &str) -> bool {
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
                    let author = person(c.get("author"));
                    let body = c.get("body").map(doc::from_adf).unwrap_or_default();
                    IssueComment {
                        id: string(c, "id"),
                        markdown: author.as_ref().filter(|a| a.me).map(|_| to_markdown(&body)),
                        author,
                        edited: updated.filter(|u| Some(u) != created.as_ref()),
                        created,
                        body,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    // By time rather than as listed: the list is a page, and its order is
    // Jira's to change.
    comments.sort_by(|a, b| a.created.cmp(&b.created));

    let mut description = fields
        .get("description")
        .map(doc::from_adf)
        .unwrap_or_default();
    let mut custom = custom(body, fields);
    let images = images(fields);
    if !images.is_empty() {
        let docs = std::iter::once(&mut description)
            .chain(comments.iter_mut().map(|c| &mut c.body))
            .chain(custom.iter_mut().map(|f| &mut f.doc));
        for blocks in docs {
            name_images(blocks, &images);
        }
    }

    Ok(Issue {
        tracker: tracker.to_string(),
        kind: Kind::Jira,
        url: format!("{site}/browse/{key}"),
        title: string(fields, "summary"),
        status: status.map(|s| string(s, "name")).unwrap_or_default(),
        stage: stage_of(status),
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
        description,
        custom,
        comments_total: comment
            .and_then(|c| c.get("total"))
            .and_then(Value::as_u64)
            .map_or(comments.len() as u32, |t| t as u32),
        comments,
        key,
    })
}

/// The ticket's image attachments, by file name. Where two share a name,
/// the later one: what an image in the text means once it is pasted again.
fn images(fields: &Value) -> std::collections::HashMap<String, String> {
    let mut list: Vec<(String, String, String)> = fields
        .get("attachment")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|a| string(a, "mimeType").starts_with("image/"))
                .map(|a| (string(a, "created"), string(a, "filename"), string(a, "id")))
                .filter(|(_, name, id)| !name.is_empty() && !id.is_empty())
                .collect()
        })
        .unwrap_or_default();
    list.sort();
    list.into_iter().map(|(_, name, id)| (name, id)).collect()
}

/// Gives each image in a document the id of the attachment it is.
fn name_images(blocks: &mut [Block], images: &std::collections::HashMap<String, String>) {
    for block in blocks {
        match block {
            Block::Attachment { name, image } => *image = images.get(name.as_str()).cloned(),
            Block::Quote { content }
            | Block::Panel { content, .. }
            | Block::Expand { content, .. } => name_images(content, images),
            Block::List { items, .. } => {
                for item in items {
                    name_images(&mut item.content, images);
                }
            }
            Block::Table { rows } => {
                for cell in rows.iter_mut().flatten() {
                    name_images(&mut cell.content, images);
                }
            }
            Block::Paragraph { .. }
            | Block::Heading { .. }
            | Block::Code { .. }
            | Block::Rule
            | Block::Card { .. } => {}
        }
    }
}

/// The custom fields that have a value, by name.
fn custom(body: &Value, fields: &Value) -> Vec<CustomField> {
    let Some(values) = fields.as_object() else {
        return Vec::new();
    };
    let names = body.get("names");
    let schemas = body.get("schema");
    let mut out: Vec<CustomField> = values
        .iter()
        .filter(|(id, _)| id.starts_with("customfield_"))
        .filter(|(id, _)| {
            let kind = schemas
                .and_then(|s| s.get(id.as_str()))
                .map(|s| string(s, "custom"))
                .unwrap_or_default();
            !NOT_SHOWN.contains(&kind.as_str())
        })
        .filter_map(|(id, value)| {
            let is_doc = string(value, "type") == "doc";
            let doc = if is_doc {
                doc::from_adf(value)
            } else {
                Vec::new()
            };
            let value = if is_doc {
                to_markdown(&doc).text
            } else {
                shown(value)
            };
            let value = value.trim().to_string();
            // A document of only an image or a mention has no text, and is
            // still something somebody wrote.
            if value.is_empty() && doc.is_empty() {
                return None;
            }
            let name = names
                .map(|n| string(n, id))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| id.clone());
            Some(CustomField {
                id: id.clone(),
                name,
                value,
                doc,
            })
        })
        .collect();
    out.sort_by_key(|f| f.name.to_lowercase());
    out
}

/// A custom field's value as words: a cascading select as both its
/// choices, a whole number without its `.0`, everything else as the edit
/// form shows a field it cannot edit.
fn shown(value: &Value) -> String {
    if let Some(n) = value
        .as_f64()
        .filter(|n| n.fract() == 0.0 && n.abs() < 1e15)
    {
        return format!("{}", n as i64);
    }
    match value.get("child") {
        Some(child) if value.get("value").is_some() => {
            let (parent, child) = (string(value, "value"), string(child, "value"));
            if child.is_empty() {
                parent
            } else {
                format!("{parent} / {child}")
            }
        }
        _ => super::edit::shown(value),
    }
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

    #[test]
    fn the_custom_fields_with_a_value_read_by_name() {
        let issue = parse(
            &serde_json::json!({
                "key": "X-1",
                "names": {
                    "customfield_1": "Story Points",
                    "customfield_2": "Sprint",
                    "customfield_3": "Rank",
                    "customfield_4": "Acceptance criteria",
                    "customfield_5": "Empty",
                    "customfield_6": "Area",
                    "customfield_7": "Team"
                },
                "schema": {
                    "customfield_3": { "type": "any", "custom": "com.pyxis.greenhopper.jira:gh-lexo-rank" }
                },
                "fields": {
                    "summary": "t",
                    "customfield_1": 5.0,
                    "customfield_2": [{ "id": 1, "name": "Sprint 12", "state": "active" }],
                    "customfield_3": "0|i0001b:",
                    "customfield_4": { "type": "doc", "version": 1, "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "It works." }] }
                    ] },
                    "customfield_5": null,
                    "customfield_6": { "value": "Backend", "child": { "value": "API" } },
                    "customfield_7": { "id": "t-1", "name": "Platform" }
                }
            }),
            "work",
            "https://example.atlassian.net",
            None,
        )
        .unwrap();
        let shown: Vec<_> = issue
            .custom
            .iter()
            .map(|f| (f.name.as_str(), f.value.as_str(), !f.doc.is_empty()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Acceptance criteria", "It works.", true),
                ("Area", "Backend / API", false),
                ("Sprint", "Sprint 12", false),
                ("Story Points", "5", false),
                ("Team", "Platform", false),
            ]
        );
    }

    #[test]
    fn an_image_in_the_text_is_given_the_attachment_it_is() {
        let media = |alt: &str| {
            serde_json::json!({ "type": "mediaSingle", "content": [
                { "type": "media", "attrs": { "id": "uuid", "type": "file", "alt": alt } }
            ] })
        };
        let issue = parse(
            &serde_json::json!({ "key": "X-1", "fields": {
                "summary": "t",
                "attachment": [
                    { "id": "10", "filename": "shot.png", "mimeType": "image/png", "created": "2026-01-01T00:00:00.000+0000" },
                    { "id": "11", "filename": "shot.png", "mimeType": "image/png", "created": "2026-02-01T00:00:00.000+0000" },
                    { "id": "12", "filename": "log.txt", "mimeType": "text/plain", "created": "2026-01-01T00:00:00.000+0000" }
                ],
                "description": { "type": "doc", "content": [
                    media("shot.png"),
                    media("log.txt"),
                    { "type": "bulletList", "content": [{ "type": "listItem", "content": [media("shot.png")] }] }
                ] },
                "comment": { "total": 1, "comments": [
                    { "id": "1", "created": "2026-02-02T00:00:00.000+0000",
                      "body": { "type": "doc", "content": [media("shot.png")] } }
                ] }
            } }),
            "work",
            "https://example.atlassian.net",
            None,
        )
        .unwrap();
        let image = |b: &Block| match b {
            Block::Attachment { image, .. } => image.clone(),
            other => panic!("{other:?}"),
        };
        // The later of the two with that name; a text file is not an image.
        assert_eq!(image(&issue.description[0]).as_deref(), Some("11"));
        assert_eq!(image(&issue.description[1]), None);
        let Block::List { items, .. } = &issue.description[2] else {
            panic!()
        };
        assert_eq!(image(&items[0].content[0]).as_deref(), Some("11"));
        assert_eq!(image(&issue.comments[0].body[0]).as_deref(), Some("11"));
    }

    #[test]
    fn only_an_image_is_recognised_as_one() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(sniff(b"GIF89a"), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"<svg xmlns="), None);
        assert_eq!(sniff(b"<html>"), None);
        assert_eq!(sniff(b""), None);
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
