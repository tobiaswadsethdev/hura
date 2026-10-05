//! Changing a Jira ticket from the window: its fields, its status, its
//! comments.
//!
//! The tickets screen began as a reader, on the reasoning that REST is for
//! what the interface shows and an agent's MCP server for what changes. That
//! held until the board: a board you can read but not move a card on sends
//! you to Jira for the one thing a board is for. So this writes -- with the
//! token the board reads with, from this machine, and only when somebody
//! presses a button. Nothing here runs on a timer.
//!
//! **What is editable is Jira's answer, not ours.** `?expand=editmeta` says
//! which fields the ticket's edit screen has, what type each is, whether it
//! is required and what it may be set to; the form is that, for the types
//! drawn below, and every other field is shown read-only with its value.
//! A field Jira will not let this account change is not in the list at all.
//!
//! **A status is a transition.** Jira does not take a status; it takes one
//! of the workflow's transitions out of the current one, which is what
//! [`transitions`] lists and [`transition`] performs. A transition with a
//! screen of required fields fails with Jira's own message, which is
//! repeated rather than replaced.
//!
//! Rich text goes through [`super::markdown`], which says what it cannot
//! keep before anything is saved.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::doc::from_adf;
use super::issue::{Stage, is_key, stage_of};
use super::markdown::{Markdown, to_adf, to_markdown};
use super::{Kind, Stored, get, jira_auth, jira_site, send, string, urlencode};

/// A ticket's edit screen.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditForm {
    pub tracker: String,
    pub key: String,
    /// Title and description first, then the people and the common ones,
    /// then the rest by name, then what can only be read.
    pub fields: Vec<EditField>,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditField {
    /// `summary`, `customfield_10016`: what a save names it by.
    pub id: String,
    pub name: String,
    pub required: bool,
    pub edit: Editable,
}

/// A field's value, as the kind of control that edits it.
///
/// Sent back as it came, with the value changed: the kind is what a save
/// turns into Jira's shape for it, so a client never writes Jira's JSON.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Editable {
    /// One line: the title, a text field, a URL.
    Text {
        value: String,
    },
    /// Rich text, as Markdown. `lost` is what saving will flatten.
    Doc {
        value: Markdown,
    },
    Number {
        value: Option<f64>,
    },
    /// `YYYY-MM-DD`.
    Date {
        value: Option<String>,
    },
    /// One of a list: a select, the priority. `value` is the choice's id.
    Select {
        value: Option<String>,
        options: Vec<Choice>,
    },
    /// Several of a list: checkboxes, a multi-select, components.
    Multi {
        value: Vec<String>,
        options: Vec<Choice>,
    },
    /// Free words with no spaces in them.
    Labels {
        value: Vec<String>,
    },
    /// A person, found by searching. `assignable` is whether the search is
    /// the people this ticket can be assigned to rather than everybody.
    User {
        value: Option<UserChoice>,
        assignable: bool,
    },
    /// A field of a type this does not edit, shown as its value.
    ReadOnly {
        value: String,
    },
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub name: String,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserChoice {
    pub account_id: String,
    pub name: String,
}

/// One field to change, and what to.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldChange {
    pub id: String,
    pub edit: Editable,
}

/// A way out of the ticket's current status.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub id: String,
    /// What the workflow calls the move: `Start progress`.
    pub name: String,
    /// The status it lands in, by id -- which is how a board column is
    /// matched -- and by name.
    pub to_id: String,
    pub to: String,
    pub stage: Stage,
}

/// Fields that are never on the form: each has a place of its own or is not
/// a field anybody edits from a panel.
const NOT_FORM: &[&str] = &[
    "comment",
    "attachment",
    "issuelinks",
    "issuetype",
    "project",
    "worklog",
    "timetracking",
];

/// The edit screen for one ticket: one request, the values and the metadata
/// together.
pub fn edit_form(stored: &Stored, key: &str) -> Result<EditForm, String> {
    let (site, auth) = connect(stored, key)?;
    let body = get(
        &format!("{site}/rest/api/3/issue/{key}?expand=editmeta&fields=*all"),
        &auth,
        &[],
    )?;
    let fields = parse_form(&body)?;
    Ok(EditForm {
        tracker: stored.source.name.clone(),
        key: key.to_string(),
        fields,
    })
}

fn parse_form(body: &Value) -> Result<Vec<EditField>, String> {
    let meta = body
        .get("editmeta")
        .and_then(|m| m.get("fields"))
        .and_then(Value::as_object)
        .ok_or("jira did not say what can be edited")?;
    let values = body.get("fields").cloned().unwrap_or(Value::Null);
    let mut fields: Vec<EditField> = meta
        .iter()
        .filter(|(id, _)| !NOT_FORM.contains(&id.as_str()))
        .map(|(id, m)| EditField {
            id: id.clone(),
            name: Some(string(m, "name"))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| id.clone()),
            required: m.get("required").and_then(Value::as_bool).unwrap_or(false),
            edit: editable(id, m, values.get(id).unwrap_or(&Value::Null)),
        })
        .collect();
    fields.sort_by_key(|f| (rank(f), f.name.to_lowercase()));
    Ok(fields)
}

/// Where a field goes on the form.
fn rank(f: &EditField) -> u8 {
    match f.id.as_str() {
        "summary" => 0,
        "description" => 1,
        "assignee" => 2,
        "priority" => 3,
        "labels" => 4,
        "duedate" => 5,
        _ if matches!(f.edit, Editable::ReadOnly { .. }) => 9,
        _ => 6,
    }
}

/// What control a field gets, from its schema, and its value in it.
fn editable(id: &str, meta: &Value, value: &Value) -> Editable {
    let schema = meta.get("schema").cloned().unwrap_or(Value::Null);
    let kind = string(&schema, "type");
    let items = string(&schema, "items");
    let custom = string(&schema, "custom");
    let system = string(&schema, "system");
    let options = choices(meta);

    match (id, kind.as_str()) {
        ("summary", _) => Editable::Text {
            value: value.as_str().unwrap_or_default().to_string(),
        },
        // Rich text in v3 of the API: the system ones, and a custom
        // paragraph field.
        ("description" | "environment", _) => doc(value),
        (_, "string") if custom.ends_with(":textarea") => doc(value),
        (_, "string") => Editable::Text {
            value: value.as_str().unwrap_or_default().to_string(),
        },
        (_, "number") => Editable::Number {
            value: value.as_f64(),
        },
        (_, "date") => Editable::Date {
            value: value.as_str().map(str::to_string),
        },
        (_, "option" | "priority" | "resolution") if !options.is_empty() => Editable::Select {
            value: Some(string(value, "id")).filter(|s| !s.is_empty()),
            options,
        },
        (_, "array")
            if items == "string" && (system == "labels" || custom.ends_with(":labels")) =>
        {
            Editable::Labels {
                value: value
                    .as_array()
                    .map(|l| {
                        l.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        }
        (_, "array")
            if !options.is_empty()
                && matches!(items.as_str(), "option" | "component" | "version") =>
        {
            Editable::Multi {
                value: value
                    .as_array()
                    .map(|l| {
                        l.iter()
                            .map(|o| string(o, "id"))
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
                options,
            }
        }
        (_, "user") => Editable::User {
            value: user(value),
            assignable: id == "assignee",
        },
        _ => Editable::ReadOnly {
            value: shown(value),
        },
    }
}

fn doc(value: &Value) -> Editable {
    Editable::Doc {
        value: to_markdown(&from_adf(value)),
    }
}

fn choices(meta: &Value) -> Vec<Choice> {
    meta.get("allowedValues")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter(|v| v.get("disabled").and_then(Value::as_bool) != Some(true))
                .filter_map(|v| {
                    let id = string(v, "id");
                    let name = [string(v, "name"), string(v, "value")]
                        .into_iter()
                        .find(|n| !n.is_empty())?;
                    (!id.is_empty()).then_some(Choice { id, name })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn user(value: &Value) -> Option<UserChoice> {
    let account_id = string(value, "accountId");
    (!account_id.is_empty()).then(|| UserChoice {
        account_id,
        name: string(value, "displayName"),
    })
}

/// Any value, as the words a person would read it as.
fn shown(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "yes" } else { "no" }.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(items) => items
            .iter()
            .map(shown)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(_) => ["displayName", "name", "value", "key"]
            .iter()
            .map(|k| string(value, k))
            .find(|s| !s.is_empty())
            // A document: its text, without the formatting.
            .or_else(|| {
                (string(value, "type") == "doc").then(|| to_markdown(&from_adf(value)).text)
            })
            .unwrap_or_default(),
    }
}

/// Save some fields. Only what changed is sent, so a field somebody else
/// edited a minute ago is not written back over with what the form loaded.
pub fn save(stored: &Stored, key: &str, changes: &[FieldChange]) -> Result<(), String> {
    if changes.is_empty() {
        return Ok(());
    }
    let (site, auth) = connect(stored, key)?;
    let body = json!({ "fields": fields_json(changes)? });
    send(
        "PUT",
        &format!("{site}/rest/api/3/issue/{key}"),
        &auth,
        Some(&body),
    )?;
    Ok(())
}

/// Each change in the shape Jira takes it.
fn fields_json(changes: &[FieldChange]) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for FieldChange { id, edit } in changes {
        // A JSON key rather than a path, but still only a field's id.
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("`{id}` is not a field"));
        }
        let value = match edit {
            Editable::Text { value } => {
                let value = value.trim();
                if id == "summary" && value.is_empty() {
                    return Err("a ticket needs a title".into());
                }
                json!(value)
            }
            Editable::Doc { value } if value.text.trim().is_empty() => Value::Null,
            Editable::Doc { value } => to_adf(&value.text),
            Editable::Number { value } => value.map_or(Value::Null, |n| json!(n)),
            Editable::Date { value } => match value.as_deref().map(str::trim) {
                None | Some("") => Value::Null,
                Some(day) if is_day(day) => json!(day),
                Some(other) => return Err(format!("`{other}` is not a date (YYYY-MM-DD)")),
            },
            Editable::Select { value, .. } => {
                value.as_ref().map_or(Value::Null, |id| json!({ "id": id }))
            }
            Editable::Multi { value, .. } => {
                json!(
                    value
                        .iter()
                        .map(|id| json!({ "id": id }))
                        .collect::<Vec<_>>()
                )
            }
            Editable::Labels { value } => {
                let labels: Vec<&str> = value
                    .iter()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty())
                    .collect();
                if let Some(bad) = labels.iter().find(|l| l.contains(char::is_whitespace)) {
                    return Err(format!("a label cannot have a space in it: `{bad}`"));
                }
                json!(labels)
            }
            Editable::User { value, .. } => value
                .as_ref()
                .map_or(Value::Null, |u| json!({ "accountId": u.account_id })),
            Editable::ReadOnly { .. } => return Err(format!("`{id}` is not editable here")),
        };
        out.insert(id.clone(), value);
    }
    Ok(out)
}

fn is_day(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// The moves out of the ticket's current status.
pub fn transitions(stored: &Stored, key: &str) -> Result<Vec<Transition>, String> {
    let (site, auth) = connect(stored, key)?;
    let body = get(
        &format!("{site}/rest/api/3/issue/{key}/transitions"),
        &auth,
        &[],
    )?;
    Ok(parse_transitions(&body))
}

fn parse_transitions(body: &Value) -> Vec<Transition> {
    body.get("transitions")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                // `isAvailable` is false for a transition whose conditions
                // this account does not meet; offering it is offering an
                // error.
                .filter(|t| t.get("isAvailable").and_then(Value::as_bool) != Some(false))
                .filter_map(|t| {
                    let to = t.get("to")?;
                    Some(Transition {
                        id: string(t, "id"),
                        name: string(t, "name"),
                        to_id: string(to, "id"),
                        to: string(to, "name"),
                        stage: stage_of(Some(to)),
                    })
                })
                .filter(|t| !t.id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Move the ticket along one transition.
pub fn transition(stored: &Stored, key: &str, id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("`{id}` is not a transition"));
    }
    let (site, auth) = connect(stored, key)?;
    send(
        "POST",
        &format!("{site}/rest/api/3/issue/{key}/transitions"),
        &auth,
        Some(&json!({ "transition": { "id": id } })),
    )?;
    Ok(())
}

/// Add a comment, written in Markdown.
pub fn comment(stored: &Stored, key: &str, markdown: &str) -> Result<(), String> {
    if markdown.trim().is_empty() {
        return Err("that comment is empty".into());
    }
    let (site, auth) = connect(stored, key)?;
    send(
        "POST",
        &format!("{site}/rest/api/3/issue/{key}/comment"),
        &auth,
        Some(&json!({ "body": to_adf(markdown) })),
    )?;
    Ok(())
}

/// Rewrite one of your comments.
pub fn edit_comment(stored: &Stored, key: &str, id: &str, markdown: &str) -> Result<(), String> {
    if markdown.trim().is_empty() {
        return Err("that comment is empty; delete it instead".into());
    }
    let (site, auth) = connect(stored, key)?;
    check_comment(id)?;
    send(
        "PUT",
        &format!("{site}/rest/api/3/issue/{key}/comment/{id}"),
        &auth,
        Some(&json!({ "body": to_adf(markdown) })),
    )?;
    Ok(())
}

pub fn delete_comment(stored: &Stored, key: &str, id: &str) -> Result<(), String> {
    let (site, auth) = connect(stored, key)?;
    check_comment(id)?;
    send(
        "DELETE",
        &format!("{site}/rest/api/3/issue/{key}/comment/{id}"),
        &auth,
        None,
    )?;
    Ok(())
}

fn check_comment(id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("`{id}` is not a comment"));
    }
    Ok(())
}

/// People whose name starts with `query`: the ones this ticket can be
/// assigned to, or everybody, for any other person field.
pub fn users(
    stored: &Stored,
    key: &str,
    query: &str,
    assignable: bool,
) -> Result<Vec<UserChoice>, String> {
    let (site, auth) = connect(stored, key)?;
    let query = urlencode(query.trim());
    let url = if assignable {
        format!(
            "{site}/rest/api/3/user/assignable/search?issueKey={key}&query={query}&maxResults=20"
        )
    } else {
        format!("{site}/rest/api/3/user/search?query={query}&maxResults=20")
    };
    let body = get(&url, &auth, &[])?;
    Ok(parse_users(&body))
}

fn parse_users(body: &Value) -> Vec<UserChoice> {
    body.as_array()
        .map(|list| {
            list.iter()
                // People, not the apps and bots Jira also calls users.
                .filter(|u| {
                    let kind = string(u, "accountType");
                    kind.is_empty() || kind == "atlassian"
                })
                .filter(|u| u.get("active").and_then(Value::as_bool) != Some(false))
                .filter_map(user)
                .collect()
        })
        .unwrap_or_default()
}

/// The site and the credential for writing to `key`, or why not.
fn connect(stored: &Stored, key: &str) -> Result<(String, String), String> {
    // It goes into a URL path, so it has to be a key and nothing else.
    if !is_key(key) {
        return Err(format!("`{key}` is not a Jira issue key"));
    }
    connect_any(stored)
}

fn connect_any(stored: &Stored) -> Result<(String, String), String> {
    let source = &stored.source;
    if source.kind != Kind::Jira {
        return Err(format!(
            "{} tickets are changed in the browser; only Jira's are changed here so far",
            source.kind.label()
        ));
    }
    if let Some(problem) = source.problem() {
        return Err(problem);
    }
    let token = stored
        .token
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| format!("{}: no token yet", source.name))?;
    Ok((jira_site(source).to_string(), jira_auth(source, token)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GET /rest/api/3/issue/X?expand=editmeta&fields=*all`, trimmed.
    const FORM: &str = r#"{
      "key": "CODE-7",
      "fields": {
        "summary": "Add the changelog",
        "description": { "type": "doc", "content": [
          { "type": "paragraph", "content": [{ "type": "text", "text": "Keep it ", "marks": [] }, { "type": "text", "text": "short", "marks": [{ "type": "strong" }] }] }
        ] },
        "priority": { "id": "3", "name": "Medium" },
        "labels": ["docs"],
        "assignee": { "accountId": "me-1", "displayName": "Ada Lovelace" },
        "customfield_10016": 3,
        "customfield_10020": [{ "id": 41, "name": "Sprint 12" }],
        "customfield_10050": { "id": "10100", "value": "Web" },
        "customfield_10060": [{ "id": "1" }],
        "comment": { "comments": [] }
      },
      "editmeta": { "fields": {
        "summary": { "required": true, "name": "Summary", "schema": { "type": "string", "system": "summary" } },
        "description": { "required": false, "name": "Description", "schema": { "type": "string", "system": "description" } },
        "priority": { "required": false, "name": "Priority", "schema": { "type": "priority", "system": "priority" },
          "allowedValues": [{ "id": "1", "name": "Highest" }, { "id": "3", "name": "Medium" }] },
        "labels": { "required": false, "name": "Labels", "schema": { "type": "array", "items": "string", "system": "labels" } },
        "assignee": { "required": false, "name": "Assignee", "schema": { "type": "user", "system": "assignee" } },
        "customfield_10016": { "required": false, "name": "Story point estimate", "schema": { "type": "number", "custom": "com.atlassian.jira.plugin.system.customfieldtypes:float" } },
        "customfield_10020": { "required": false, "name": "Sprint", "schema": { "type": "array", "items": "json", "custom": "com.pyxis.greenhopper.jira:gh-sprint" } },
        "customfield_10050": { "required": false, "name": "Area", "schema": { "type": "option", "custom": "com.atlassian.jira.plugin.system.customfieldtypes:select" },
          "allowedValues": [{ "id": "10100", "value": "Web" }, { "id": "10101", "value": "App", "disabled": true }] },
        "customfield_10060": { "required": false, "name": "Platforms", "schema": { "type": "array", "items": "option" },
          "allowedValues": [{ "id": "1", "value": "Linux" }, { "id": "2", "value": "Windows" }] },
        "comment": { "required": false, "name": "Comment", "schema": { "type": "comments-page", "system": "comment" } }
      } }
    }"#;

    #[test]
    fn the_form_is_what_jira_says_can_be_edited_as_the_control_for_each() {
        let fields = parse_form(&serde_json::from_str(FORM).unwrap()).unwrap();
        let ids: Vec<&str> = fields.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "summary",
                "description",
                "assignee",
                "priority",
                "labels",
                "customfield_10050",
                "customfield_10060",
                "customfield_10016",
                "customfield_10020"
            ],
            "comment has its own place; read-only goes last"
        );
        let by = |id: &str| &fields.iter().find(|f| f.id == id).unwrap().edit;

        assert!(fields[0].required);
        assert_eq!(
            by("summary"),
            &Editable::Text {
                value: "Add the changelog".into()
            }
        );
        match by("description") {
            Editable::Doc { value } => {
                assert_eq!(value.text, "Keep it **short**");
                assert!(value.lost.is_empty());
            }
            other => panic!("{other:?}"),
        }
        match by("priority") {
            Editable::Select { value, options } => {
                assert_eq!(value.as_deref(), Some("3"));
                assert_eq!(options.len(), 2);
            }
            other => panic!("{other:?}"),
        }
        match by("customfield_10050") {
            Editable::Select { options, .. } => {
                assert_eq!(
                    options,
                    &[Choice {
                        id: "10100".into(),
                        name: "Web".into()
                    }],
                    "a disabled option is not offered"
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            by("customfield_10016"),
            &Editable::Number { value: Some(3.0) }
        );
        assert_eq!(
            by("labels"),
            &Editable::Labels {
                value: vec!["docs".into()]
            }
        );
        match by("customfield_10060") {
            Editable::Multi { value, .. } => assert_eq!(value, &["1"]),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            by("assignee"),
            &Editable::User {
                value: Some(UserChoice {
                    account_id: "me-1".into(),
                    name: "Ada Lovelace".into()
                }),
                assignable: true
            }
        );
        assert_eq!(
            by("customfield_10020"),
            &Editable::ReadOnly {
                value: "Sprint 12".into()
            }
        );
    }

    #[test]
    fn a_save_is_written_in_the_shape_jira_takes_each_field() {
        let change = |id: &str, edit: Editable| FieldChange {
            id: id.into(),
            edit,
        };
        let out = fields_json(&[
            change(
                "summary",
                Editable::Text {
                    value: "  New title ".into(),
                },
            ),
            change(
                "description",
                Editable::Doc {
                    value: Markdown {
                        text: "**hi**".into(),
                        lost: vec![],
                    },
                },
            ),
            change(
                "priority",
                Editable::Select {
                    value: Some("1".into()),
                    options: vec![],
                },
            ),
            change(
                "assignee",
                Editable::User {
                    value: None,
                    assignable: true,
                },
            ),
            change(
                "labels",
                Editable::Labels {
                    value: vec!["a".into(), " ".into()],
                },
            ),
            change("customfield_10016", Editable::Number { value: None }),
            change(
                "duedate",
                Editable::Date {
                    value: Some("2026-10-07".into()),
                },
            ),
            change(
                "customfield_10060",
                Editable::Multi {
                    value: vec!["2".into()],
                    options: vec![],
                },
            ),
        ])
        .unwrap();
        assert_eq!(out["summary"], "New title");
        assert_eq!(out["description"]["type"], "doc");
        assert_eq!(
            out["description"]["content"][0]["content"][0]["marks"][0]["type"],
            "strong"
        );
        assert_eq!(out["priority"], json!({ "id": "1" }));
        assert_eq!(out["assignee"], Value::Null, "unassigned");
        assert_eq!(out["labels"], json!(["a"]));
        assert_eq!(out["customfield_10016"], Value::Null);
        assert_eq!(out["duedate"], "2026-10-07");
        assert_eq!(out["customfield_10060"], json!([{ "id": "2" }]));

        // An emptied description is cleared, not saved as an empty document.
        let out = fields_json(&[change(
            "description",
            Editable::Doc {
                value: Markdown {
                    text: "  ".into(),
                    lost: vec![],
                },
            },
        )])
        .unwrap();
        assert_eq!(out["description"], Value::Null);
    }

    #[test]
    fn a_save_that_cannot_work_is_refused_before_it_is_sent() {
        let one = |id: &str, edit: Editable| {
            fields_json(&[FieldChange {
                id: id.into(),
                edit,
            }])
        };
        assert!(
            one("summary", Editable::Text { value: " ".into() })
                .unwrap_err()
                .contains("title")
        );
        assert!(
            one(
                "duedate",
                Editable::Date {
                    value: Some("7 Oct".into())
                }
            )
            .unwrap_err()
            .contains("date")
        );
        assert!(
            one(
                "labels",
                Editable::Labels {
                    value: vec!["two words".into()]
                }
            )
            .unwrap_err()
            .contains("space")
        );
        assert!(one("customfield_1", Editable::ReadOnly { value: "x".into() }).is_err());
        assert!(
            one("a\"b", Editable::Text { value: "x".into() })
                .unwrap_err()
                .contains("not a field")
        );
    }

    #[test]
    fn the_transitions_are_the_available_ones_and_where_each_lands() {
        let body = json!({ "transitions": [
            { "id": "21", "name": "Start", "isAvailable": true,
              "to": { "id": "3", "name": "In Progress", "statusCategory": { "key": "indeterminate" } } },
            { "id": "31", "name": "Done", "isAvailable": false,
              "to": { "id": "10046", "name": "Done", "statusCategory": { "key": "done" } } }
        ] });
        let t = parse_transitions(&body);
        assert_eq!(t.len(), 1, "an unavailable one is not offered");
        assert_eq!(t[0].to_id, "3");
        assert_eq!(t[0].stage, Stage::Doing);
    }

    #[test]
    fn a_people_search_is_people() {
        let body = json!([
            { "accountId": "a", "displayName": "Ada", "accountType": "atlassian", "active": true },
            { "accountId": "b", "displayName": "Automation", "accountType": "app" },
            { "accountId": "c", "displayName": "Gone", "accountType": "atlassian", "active": false }
        ]);
        let found = parse_users(&body);
        assert_eq!(
            found,
            [UserChoice {
                account_id: "a".into(),
                name: "Ada".into()
            }]
        );
    }

    #[test]
    fn only_ids_go_into_a_path() {
        let stored = Stored {
            source: crate::tracker::Source {
                kind: Kind::Jira,
                name: "w".into(),
                site: Some("http://127.0.0.1:9".into()),
                email: Some("e@x".into()),
                ..Default::default()
            },
            token: Some("t".into()),
        };
        assert!(
            transition(&stored, "X-1", "21/../x")
                .unwrap_err()
                .contains("not a transition")
        );
        assert!(
            save(
                &stored,
                "X-1/../../myself",
                &[FieldChange {
                    id: "summary".into(),
                    edit: Editable::Text { value: "x".into() }
                }]
            )
            .unwrap_err()
            .contains("not a Jira issue key")
        );
        assert!(
            delete_comment(&stored, "X-1", "1?x")
                .unwrap_err()
                .contains("not a comment")
        );
    }
}
