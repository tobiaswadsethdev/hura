//! One Fizzy card, read in full and changed: the same [`Issue`], edit form,
//! status moves and comments a Jira ticket has, so the apps draw a card with
//! the panel they already have.
//!
//! **Addressed by its key.** `#4`, or `acme#4` beside other accounts, is what
//! the sidebar shows and what the apps send back; the account it is in is
//! found again from the token's identity, as the list found it.
//!
//! **A status is where the card is.** Fizzy has no workflow to transition
//! along: a card is in *Maybe?*, in one of its board's columns, put off as
//! *Not Now*, or *Done*, and can be moved from any of those to any other. So
//! [`transitions`] lists every place it is not, and [`transition`] is one
//! request each (moving a closed or postponed card into a column or back to
//! *Maybe?* reopens it, Fizzy's own rule).
//!
//! **The form is Fizzy's card, not a field list.** Title and description,
//! the people on it, its tags, its steps (a checklist, written here as one)
//! and its board. Assignees and tags are toggles in Fizzy's API, so a save
//! reads the card again and toggles only what differs from what was asked
//! for: whatever somebody else changed meanwhile is left alone.
//!
//! Rich text is HTML there and Markdown here, through [`super::super::html`].

use serde_json::{Map, Value, json};

use super::{Account, Connection, connect, get, is_id, pages, send};
use crate::tracker::doc::{Block, Item};
use crate::tracker::edit::{Choice, EditField, EditForm, Editable, FieldChange, Transition};
use crate::tracker::html::{from_html, from_markdown};
use crate::tracker::issue::{CustomField, Issue, IssueComment, Person, Stage};
use crate::tracker::markdown::{Markdown, to_markdown};
use crate::tracker::{Kind, Stored, doc, string};

use super::UNTRIAGED;

/// What Fizzy calls a closed card, and one put off.
const DONE: &str = "Done";
const NOT_NOW: &str = "Not Now";

/// One card, and where its requests go.
struct Card<'a> {
    conn: Connection<'a>,
    account: Account,
    /// `https://app.fizzy.do/897362094/cards/4`
    base: String,
}

impl<'a> Card<'a> {
    fn open(stored: &'a Stored, key: &str) -> Result<Card<'a>, String> {
        let (label, number) = key
            .trim()
            .rsplit_once('#')
            .ok_or_else(|| format!("`{key}` is not a fizzy card (`#4`)"))?;
        if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("`{key}` is not a fizzy card (`#4`)"));
        }
        let conn = connect(stored)?;
        let account = if label.is_empty() {
            match conn.accounts.as_slice() {
                [only] => only.clone(),
                _ => {
                    return Err(format!(
                        "`{key}` could be in any of {} accounts; it needs the account's name in front",
                        conn.accounts.len()
                    ));
                }
            }
        } else {
            conn.accounts
                .iter()
                .find(|a| a.label.as_deref() == Some(label))
                .cloned()
                .ok_or_else(|| format!("`{key}`: the token has no account `{label}`"))?
        };
        let base = format!("{}{}/cards/{number}", conn.site, account.slug);
        Ok(Card {
            conn,
            account,
            base,
        })
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        get(&format!("{}{path}", self.base), self.conn.token)
    }

    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        send(
            method,
            &format!("{}{path}", self.base),
            self.conn.token,
            body,
        )
    }

    /// The account's own, for what is not the card's: its people, its boards.
    fn account_list(&self, path: &str) -> Result<Vec<Value>, String> {
        let url = format!("{}{}{path}", self.conn.site, self.account.slug);
        pages(&url, self.conn.token).map(|(list, _)| list)
    }
}

/// Read one card, with its steps and its comments.
pub(in crate::tracker) fn issue(stored: &Stored, key: &str) -> Result<Issue, String> {
    let card = Card::open(stored, key)?;
    let body = card.get("")?;
    let (comments, _) = pages(&format!("{}/comments", card.base), card.conn.token)?;
    parse(
        &body,
        &comments,
        &stored.source.name,
        key.trim(),
        &card.base,
        &card.account.user,
    )
}

fn parse(
    card: &Value,
    comments: &[Value],
    tracker: &str,
    key: &str,
    base: &str,
    me: &str,
) -> Result<Issue, String> {
    if card.get("number").and_then(Value::as_u64).is_none() {
        return Err("fizzy did not answer with a card".into());
    }
    let person = |v: Option<&Value>| -> Option<Person> {
        let v = v.filter(|v| v.is_object())?;
        let name = string(v, "name");
        (!name.is_empty()).then(|| Person {
            me: string(v, "id") == me,
            name,
        })
    };
    let (status, stage) = place(card);
    let mut comments: Vec<IssueComment> = comments
        .iter()
        .map(|c| {
            let author = person(c.get("creator"));
            let body = c.get("body").map(rich).unwrap_or_default();
            let created = Some(string(c, "created_at")).filter(|s| !s.is_empty());
            let updated = Some(string(c, "updated_at")).filter(|s| !s.is_empty());
            IssueComment {
                id: string(c, "id"),
                markdown: author.as_ref().filter(|a| a.me).map(|_| to_markdown(&body)),
                author,
                edited: updated.filter(|u| Some(u) != created.as_ref()),
                created,
                body,
            }
        })
        .collect();
    comments.sort_by(|a, b| a.created.cmp(&b.created));

    let steps = steps(card);
    let mut custom = Vec::new();
    if !steps.is_empty() {
        let done = steps.iter().filter(|(_, d)| *d).count();
        custom.push(CustomField {
            id: "steps".into(),
            name: "Steps".into(),
            value: format!("{done} of {} done", steps.len()),
            doc: vec![Block::List {
                ordered: false,
                start: 1,
                items: steps
                    .iter()
                    .map(|(text, done)| Item {
                        checked: Some(*done),
                        content: doc::plain(text),
                    })
                    .collect(),
            }],
        });
    }

    Ok(Issue {
        tracker: tracker.to_string(),
        kind: Kind::Fizzy,
        key: key.to_string(),
        title: string(card, "title"),
        url: Some(string(card, "url"))
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| base.to_string()),
        status,
        stage,
        // The board, as in the sidebar.
        item_type: card
            .get("board")
            .map(|b| string(b, "name"))
            .unwrap_or_default(),
        // The nearest Fizzy has to a priority: a card marked as the one that
        // matters.
        priority: (card.get("golden").and_then(Value::as_bool) == Some(true))
            .then(|| "Golden".to_string()),
        assignee: assignees(card, me),
        reporter: person(card.get("creator")),
        labels: tags(card),
        parent: None,
        created: Some(string(card, "created_at")).filter(|s| !s.is_empty()),
        updated: Some(string(card, "last_active_at")).filter(|s| !s.is_empty()),
        due: None,
        description: description(card),
        custom,
        comments_total: comments.len() as u32,
        comments,
    })
}

/// Where the card is, in words and as a stage.
fn place(card: &Value) -> (String, Stage) {
    let flag = |k: &str| card.get(k).and_then(Value::as_bool) == Some(true);
    if flag("closed") {
        return (DONE.into(), Stage::Done);
    }
    if flag("postponed") {
        return (NOT_NOW.into(), Stage::Todo);
    }
    match card
        .get("column")
        .map(|c| string(c, "name"))
        .filter(|s| !s.is_empty())
    {
        Some(column) => (column, Stage::Doing),
        None => (UNTRIAGED.into(), Stage::Todo),
    }
}

/// The people on a card, as the one line the panel has for them. Fizzy
/// answers with the first five, and says when there are more.
fn assignees(card: &Value, me: &str) -> Option<Person> {
    let list = card.get("assignees").and_then(Value::as_array)?;
    let mut names: Vec<String> = list
        .iter()
        .map(|u| string(u, "name"))
        .filter(|n| !n.is_empty())
        .collect();
    if names.is_empty() {
        return None;
    }
    if card.get("has_more_assignees").and_then(Value::as_bool) == Some(true) {
        names.push("more".into());
    }
    Some(Person {
        name: names.join(", "),
        me: list.iter().any(|u| string(u, "id") == me),
    })
}

fn tags(card: &Value) -> Vec<String> {
    card.get("tags")
        .and_then(Value::as_array)
        .map(|t| {
            t.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn steps(card: &Value) -> Vec<(String, bool)> {
    card.get("steps")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|s| {
                    (
                        string(s, "content"),
                        s.get("completed").and_then(Value::as_bool) == Some(true),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn step_ids(card: &Value) -> Vec<String> {
    card.get("steps")
        .and_then(Value::as_array)
        .map(|list| list.iter().map(|s| string(s, "id")).collect())
        .unwrap_or_default()
}

fn description(card: &Value) -> Vec<Block> {
    let html = string(card, "description_html");
    if html.trim().is_empty() {
        doc::plain(&string(card, "description"))
    } else {
        from_html(&html)
    }
}

/// A comment's body: `{ "plain_text", "html" }`.
fn rich(body: &Value) -> Vec<Block> {
    let html = string(body, "html");
    if html.trim().is_empty() {
        doc::plain(&string(body, "plain_text"))
    } else {
        from_html(&html)
    }
}

// -------------------------------------------------------------- the form

/// The card as a form: what Fizzy lets a card's page change.
pub(in crate::tracker) fn edit_form(stored: &Stored, key: &str) -> Result<EditForm, String> {
    let card = Card::open(stored, key)?;
    let body = card.get("")?;
    let people = card.account_list("/users")?;
    let boards = card.account_list("/boards")?;
    Ok(EditForm {
        tracker: stored.source.name.clone(),
        key: key.trim().to_string(),
        fields: form(&body, &people, &boards),
    })
}

fn form(card: &Value, people: &[Value], boards: &[Value]) -> Vec<EditField> {
    let field = |id: &str, name: &str, required: bool, edit: Editable| EditField {
        id: id.into(),
        name: name.into(),
        required,
        edit,
    };
    let assigned: Vec<String> = card
        .get("assignees")
        .and_then(Value::as_array)
        .map(|l| l.iter().map(|u| string(u, "id")).collect())
        .unwrap_or_default();
    // Only five come back with a card. A form that showed five of seven
    // would take the other two off at the first save.
    let assignees = if card.get("has_more_assignees").and_then(Value::as_bool) == Some(true) {
        Editable::ReadOnly {
            value: assignees(card, "").map(|p| p.name).unwrap_or_default(),
        }
    } else {
        Editable::Multi {
            value: assigned,
            options: choices(people, "name"),
        }
    };
    let steps = steps(card)
        .into_iter()
        .map(|(text, done)| format!("- [{}] {text}", if done { "x" } else { " " }))
        .collect::<Vec<_>>()
        .join("\n");
    let board = card.get("board").map(|b| string(b, "id"));
    vec![
        field(
            "title",
            "Title",
            true,
            Editable::Text {
                value: string(card, "title"),
            },
        ),
        field(
            "description",
            "Description",
            false,
            Editable::Doc {
                value: to_markdown(&description(card)),
            },
        ),
        field("assignees", "Assigned to", false, assignees),
        field(
            "tags",
            "Tags",
            false,
            Editable::Labels { value: tags(card) },
        ),
        field(
            "steps",
            "Steps",
            false,
            Editable::Doc {
                value: Markdown {
                    text: steps,
                    lost: Vec::new(),
                },
            },
        ),
        field(
            "board",
            "Board",
            true,
            Editable::Select {
                value: board.filter(|b| !b.is_empty()),
                options: choices(boards, "name"),
            },
        ),
    ]
}

fn choices(list: &[Value], name: &str) -> Vec<Choice> {
    list.iter()
        .filter(|v| v.get("active").and_then(Value::as_bool) != Some(false))
        .map(|v| Choice {
            id: string(v, "id"),
            name: string(v, name),
        })
        .filter(|c| !c.id.is_empty())
        .collect()
}

/// Save what changed on the form. The card is read again first, so a
/// toggle is only sent for what differs from it now.
pub(in crate::tracker) fn save(
    stored: &Stored,
    key: &str,
    changes: &[FieldChange],
) -> Result<(), String> {
    if changes.is_empty() {
        return Ok(());
    }
    let plan = plan(changes)?;
    let card = Card::open(stored, key)?;
    let now = card.get("")?;

    if !plan.card.is_empty() {
        card.send("PUT", "", Some(&json!({ "card": plan.card })))?;
    }
    if let Some(wanted) = &plan.assignees {
        let have: Vec<String> = now
            .get("assignees")
            .and_then(Value::as_array)
            .map(|l| l.iter().map(|u| string(u, "id")).collect())
            .unwrap_or_default();
        for id in toggled(&have, wanted, |a, b| a == b) {
            card.send("POST", "/assignments", Some(&json!({ "assignee_id": id })))?;
        }
    }
    if let Some(wanted) = &plan.tags {
        let same = |a: &String, b: &String| a.eq_ignore_ascii_case(b);
        for title in toggled(&tags(&now), wanted, same) {
            card.send("POST", "/taggings", Some(&json!({ "tag_title": title })))?;
        }
    }
    if let Some(wanted) = &plan.steps {
        let have = steps(&now);
        let ids = step_ids(&now);
        for (i, (text, done)) in wanted.iter().enumerate() {
            match (have.get(i), ids.get(i)) {
                (Some(old), Some(id)) if old != &(text.clone(), *done) => {
                    check(id, "step")?;
                    card.send(
                        "PUT",
                        &format!("/steps/{id}"),
                        Some(&json!({ "step": { "content": text, "completed": done } })),
                    )?;
                }
                (Some(_), Some(_)) => {}
                _ => {
                    card.send(
                        "POST",
                        "/steps",
                        Some(&json!({ "step": { "content": text, "completed": done } })),
                    )?;
                }
            }
        }
        for id in ids.iter().skip(wanted.len()) {
            check(id, "step")?;
            card.send("DELETE", &format!("/steps/{id}"), None)?;
        }
    }
    if let Some(board) = &plan.board {
        let current = now.get("board").map(|b| string(b, "id"));
        if current.as_deref() != Some(board.as_str()) {
            card.send("PUT", "/board", Some(&json!({ "board_id": board })))?;
        }
    }
    Ok(())
}

/// What a save asks for, checked before anything is sent.
#[derive(Debug, Default, PartialEq)]
struct Plan {
    /// Title and description: one `PUT` of the card.
    card: Map<String, Value>,
    assignees: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    steps: Option<Vec<(String, bool)>>,
    board: Option<String>,
}

fn plan(changes: &[FieldChange]) -> Result<Plan, String> {
    let mut plan = Plan::default();
    for FieldChange { id, edit } in changes {
        match (id.as_str(), edit) {
            ("title", Editable::Text { value }) => {
                let value = value.trim();
                if value.is_empty() {
                    return Err("a card needs a title".into());
                }
                plan.card.insert("title".into(), json!(value));
            }
            ("description", Editable::Doc { value }) => {
                let html = if value.text.trim().is_empty() {
                    String::new()
                } else {
                    from_markdown(&value.text)
                };
                plan.card.insert("description".into(), json!(html));
            }
            ("assignees", Editable::Multi { value, .. }) => {
                for id in value {
                    check(id, "person")?;
                }
                plan.assignees = Some(value.clone());
            }
            ("tags", Editable::Labels { value }) => {
                let mut tags: Vec<String> = Vec::new();
                for t in value {
                    let t = t.trim().trim_start_matches('#').trim();
                    if !t.is_empty() && !tags.iter().any(|have| have.eq_ignore_ascii_case(t)) {
                        tags.push(t.to_string());
                    }
                }
                plan.tags = Some(tags);
            }
            ("steps", Editable::Doc { value }) => plan.steps = Some(parse_steps(&value.text)),
            ("board", Editable::Select { value, .. }) => match value {
                Some(board) => {
                    check(board, "board")?;
                    plan.board = Some(board.clone());
                }
                None => return Err("a card is always on a board".into()),
            },
            (other, _) => return Err(format!("`{other}` is not something a card has here")),
        }
    }
    Ok(plan)
}

/// Each line of a checklist as a step: `- [x] done`, `- [ ] not`, and a
/// plain line or bullet as one not done yet.
fn parse_steps(text: &str) -> Vec<(String, bool)> {
    text.lines()
        .filter_map(|line| {
            let mut t = line.trim();
            for bullet in ["- ", "* ", "+ "] {
                if let Some(rest) = t.strip_prefix(bullet) {
                    t = rest.trim_start();
                    break;
                }
            }
            let (done, t) =
                if let Some(rest) = t.strip_prefix("[x]").or_else(|| t.strip_prefix("[X]")) {
                    (true, rest)
                } else if let Some(rest) = t.strip_prefix("[ ]").or_else(|| t.strip_prefix("[]")) {
                    (false, rest)
                } else {
                    (false, t)
                };
            let t = t.trim();
            (!t.is_empty()).then(|| (t.to_string(), done))
        })
        .collect()
}

/// What has to be toggled to turn `have` into `wanted`: what is in one and
/// not the other.
fn toggled(
    have: &[String],
    wanted: &[String],
    same: impl Fn(&String, &String) -> bool,
) -> Vec<String> {
    let gone = have.iter().filter(|h| !wanted.iter().any(|w| same(h, w)));
    let new = wanted.iter().filter(|w| !have.iter().any(|h| same(h, w)));
    gone.chain(new).cloned().collect()
}

fn check(id: &str, what: &str) -> Result<(), String> {
    if is_id(id) {
        Ok(())
    } else {
        Err(format!("`{id}` is not a fizzy {what}"))
    }
}

// --------------------------------------------------------------- status

/// Every place the card can go from where it is: back to *Maybe?*, into
/// each of its board's other columns, *Not Now*, or *Done*.
pub(in crate::tracker) fn transitions(
    stored: &Stored,
    key: &str,
) -> Result<Vec<Transition>, String> {
    let card = Card::open(stored, key)?;
    let body = card.get("")?;
    let board = body
        .get("board")
        .map(|b| string(b, "id"))
        .unwrap_or_default();
    let columns = if is_id(&board) {
        let url = format!(
            "{}{}/boards/{board}/columns",
            card.conn.site, card.account.slug
        );
        pages(&url, card.conn.token)?.0
    } else {
        Vec::new()
    };
    Ok(moves(&body, &columns))
}

fn moves(card: &Value, columns: &[Value]) -> Vec<Transition> {
    let flag = |k: &str| card.get(k).and_then(Value::as_bool) == Some(true);
    let (closed, postponed) = (flag("closed"), flag("postponed"));
    let column = card
        .get("column")
        .map(|c| string(c, "id"))
        .filter(|c| !c.is_empty() && !closed && !postponed);
    let to = |id: String, to: &str, stage: Stage| Transition {
        id: id.clone(),
        name: to.to_string(),
        to_id: id,
        to: to.to_string(),
        stage,
    };
    let mut out = Vec::new();
    if column.is_some() || closed || postponed {
        out.push(to("maybe".into(), UNTRIAGED, Stage::Todo));
    }
    for c in columns {
        let id = string(c, "id");
        if id.is_empty() || column.as_deref() == Some(id.as_str()) {
            continue;
        }
        out.push(to(format!("column:{id}"), &string(c, "name"), Stage::Doing));
    }
    if !postponed {
        out.push(to("not-now".into(), NOT_NOW, Stage::Todo));
    }
    if !closed {
        out.push(to("done".into(), DONE, Stage::Done));
    }
    out
}

/// Move the card to one of [`transitions`]' places.
pub(in crate::tracker) fn transition(stored: &Stored, key: &str, id: &str) -> Result<(), String> {
    let (method, path, body) = match id {
        "maybe" => ("DELETE", "/triage".to_string(), None),
        "not-now" => ("POST", "/not_now".to_string(), None),
        "done" => ("POST", "/closure".to_string(), None),
        other => match other.strip_prefix("column:") {
            Some(column) if is_id(column) => (
                "POST",
                "/triage".to_string(),
                Some(json!({ "column_id": column })),
            ),
            _ => return Err(format!("`{id}` is not somewhere a card can go")),
        },
    };
    let card = Card::open(stored, key)?;
    card.send(method, &path, body.as_ref())?;
    Ok(())
}

// ------------------------------------------------------------- comments

pub(in crate::tracker) fn comment(
    stored: &Stored,
    key: &str,
    markdown: &str,
) -> Result<(), String> {
    if markdown.trim().is_empty() {
        return Err("that comment is empty".into());
    }
    let card = Card::open(stored, key)?;
    card.send(
        "POST",
        "/comments",
        Some(&json!({ "comment": { "body": from_markdown(markdown) } })),
    )?;
    Ok(())
}

pub(in crate::tracker) fn edit_comment(
    stored: &Stored,
    key: &str,
    id: &str,
    markdown: &str,
) -> Result<(), String> {
    if markdown.trim().is_empty() {
        return Err("that comment is empty; delete it instead".into());
    }
    check(id, "comment")?;
    let card = Card::open(stored, key)?;
    card.send(
        "PUT",
        &format!("/comments/{id}"),
        Some(&json!({ "comment": { "body": from_markdown(markdown) } })),
    )?;
    Ok(())
}

pub(in crate::tracker) fn delete_comment(
    stored: &Stored,
    key: &str,
    id: &str,
) -> Result<(), String> {
    check(id, "comment")?;
    let card = Card::open(stored, key)?;
    card.send("DELETE", &format!("/comments/{id}"), None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GET /897362094/cards/42`, as Fizzy's card partial writes it.
    fn card() -> Value {
        json!({
            "id": "03f5vaeq985jlvwv3arl4srq2",
            "number": 42,
            "title": "Retry uploads",
            "status": "published",
            "description": "Keep it short.",
            "description_html": "<div class=\"action-text-content\"><p>Keep it <strong>short</strong>.</p></div>",
            "tags": ["bug", "web"],
            "closed": false,
            "postponed": false,
            "golden": true,
            "last_active_at": "2025-12-05T19:38:48.553Z",
            "created_at": "2025-12-05T19:38:48.540Z",
            "url": "https://app.fizzy.do/897362094/cards/42",
            "board": { "id": "b1", "name": "Website" },
            "column": { "id": "c2", "name": "In Progress" },
            "creator": { "id": "u2", "name": "Grace Hopper" },
            "assignees": [{ "id": "me", "name": "Ada Lovelace" }, { "id": "u3", "name": "Alan Turing" }],
            "has_more_assignees": false,
            "steps": [
                { "id": "s1", "content": "Write tests", "completed": true },
                { "id": "s2", "content": "Ship it", "completed": false }
            ]
        })
    }

    fn comments() -> Vec<Value> {
        vec![
            json!({ "id": "k2", "created_at": "2025-12-06T10:00:00Z", "updated_at": "2025-12-06T11:00:00Z",
                    "body": { "plain_text": "mine", "html": "<div class=\"action-text-content\">mine</div>" },
                    "creator": { "id": "me", "name": "Ada Lovelace" } }),
            json!({ "id": "k1", "created_at": "2025-12-05T10:00:00Z", "updated_at": "2025-12-05T10:00:00Z",
                    "body": { "plain_text": "first", "html": "" },
                    "creator": { "id": "u2", "name": "Grace Hopper" } }),
        ]
    }

    #[test]
    fn a_card_reads_as_an_issue_with_its_steps_and_its_conversation() {
        let issue = parse(
            &card(),
            &comments(),
            "fizzy",
            "#42",
            "https://app.fizzy.do/897362094/cards/42",
            "me",
        )
        .unwrap();
        assert_eq!(issue.kind, Kind::Fizzy);
        assert_eq!(issue.key, "#42");
        assert_eq!(issue.status, "In Progress");
        assert_eq!(issue.stage, Stage::Doing);
        assert_eq!(issue.item_type, "Website");
        assert_eq!(issue.priority.as_deref(), Some("Golden"));
        let assignee = issue.assignee.unwrap();
        assert_eq!(assignee.name, "Ada Lovelace, Alan Turing");
        assert!(assignee.me);
        assert_eq!(issue.reporter.unwrap().name, "Grace Hopper");
        assert_eq!(issue.labels, vec!["bug", "web"]);
        assert_eq!(to_markdown(&issue.description).text, "Keep it **short**.");
        assert_eq!(issue.custom[0].value, "1 of 2 done");
        assert_eq!(
            to_markdown(&issue.custom[0].doc).text,
            "- [x] Write tests\n- [ ] Ship it"
        );
        // Oldest first; only your own carries Markdown to edit.
        assert_eq!(issue.comments[0].id, "k1");
        assert_eq!(to_markdown(&issue.comments[0].body).text, "first");
        assert!(issue.comments[0].markdown.is_none());
        assert_eq!(issue.comments[0].edited, None);
        assert_eq!(issue.comments[1].markdown.as_ref().unwrap().text, "mine");
        assert!(issue.comments[1].edited.is_some());
        assert_eq!(issue.comments_total, 2);
    }

    #[test]
    fn where_a_card_is_is_its_status() {
        let mut c = card();
        c["closed"] = json!(true);
        assert_eq!(place(&c), ("Done".into(), Stage::Done));
        let mut c = card();
        c["postponed"] = json!(true);
        assert_eq!(place(&c), ("Not Now".into(), Stage::Todo));
        let mut c = card();
        c.as_object_mut().unwrap().remove("column");
        assert_eq!(place(&c), ("Maybe?".into(), Stage::Todo));
    }

    #[test]
    fn a_card_can_go_anywhere_it_is_not() {
        let columns = vec![
            json!({ "id": "c1", "name": "Ready" }),
            json!({ "id": "c2", "name": "In Progress" }),
        ];
        let names = |c: &Value| -> Vec<String> {
            moves(c, &columns)
                .into_iter()
                .map(|t| format!("{}={}", t.id, t.to))
                .collect()
        };
        assert_eq!(
            names(&card()),
            vec![
                "maybe=Maybe?",
                "column:c1=Ready",
                "not-now=Not Now",
                "done=Done"
            ]
        );
        let mut closed = card();
        closed["closed"] = json!(true);
        assert_eq!(
            names(&closed),
            vec![
                "maybe=Maybe?",
                "column:c1=Ready",
                "column:c2=In Progress",
                "not-now=Not Now"
            ]
        );
        let mut maybe = card();
        maybe.as_object_mut().unwrap().remove("column");
        assert_eq!(
            names(&maybe),
            vec![
                "column:c1=Ready",
                "column:c2=In Progress",
                "not-now=Not Now",
                "done=Done"
            ]
        );
    }

    #[test]
    fn the_form_is_the_card() {
        let people = vec![
            json!({ "id": "me", "name": "Ada Lovelace", "active": true }),
            json!({ "id": "gone", "name": "Left", "active": false }),
        ];
        let boards = vec![json!({ "id": "b1", "name": "Website" })];
        let fields = form(&card(), &people, &boards);
        let ids: Vec<&str> = fields.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "title",
                "description",
                "assignees",
                "tags",
                "steps",
                "board"
            ]
        );
        assert_eq!(
            fields[1].edit,
            Editable::Doc {
                value: Markdown {
                    text: "Keep it **short**.".into(),
                    lost: vec![]
                }
            }
        );
        let Editable::Multi { value, options } = &fields[2].edit else {
            panic!("{:?}", fields[2].edit)
        };
        assert_eq!(value, &vec!["me".to_string(), "u3".to_string()]);
        assert_eq!(options.len(), 1, "only the active");
        let Editable::Doc { value } = &fields[4].edit else {
            panic!()
        };
        assert_eq!(value.text, "- [x] Write tests\n- [ ] Ship it");

        // Five of seven is not something to save back.
        let mut crowded = card();
        crowded["has_more_assignees"] = json!(true);
        assert!(matches!(
            form(&crowded, &people, &boards)[2].edit,
            Editable::ReadOnly { .. }
        ));
    }

    #[test]
    fn a_save_is_planned_and_checked_before_it_is_sent() {
        let change = |id: &str, edit: Editable| FieldChange {
            id: id.into(),
            edit,
        };
        let plan = plan(&[
            change(
                "title",
                Editable::Text {
                    value: " New title ".into(),
                },
            ),
            change(
                "description",
                Editable::Doc {
                    value: Markdown {
                        text: "Hi **there**".into(),
                        lost: vec![],
                    },
                },
            ),
            change(
                "tags",
                Editable::Labels {
                    value: vec!["#bug".into(), "Bug".into(), " ".into(), "ui".into()],
                },
            ),
            change(
                "steps",
                Editable::Doc {
                    value: Markdown {
                        text: "- [x] one\n\n- [ ] two\nthree\n* [X] four".into(),
                        lost: vec![],
                    },
                },
            ),
        ])
        .unwrap();
        assert_eq!(plan.card["title"], "New title");
        assert_eq!(plan.card["description"], "<p>Hi <strong>there</strong></p>");
        assert_eq!(plan.tags, Some(vec!["bug".to_string(), "ui".to_string()]));
        assert_eq!(
            plan.steps,
            Some(vec![
                ("one".to_string(), true),
                ("two".to_string(), false),
                ("three".to_string(), false),
                ("four".to_string(), true)
            ])
        );

        assert!(super::plan(&[change("title", Editable::Text { value: " ".into() })]).is_err());
        assert!(
            super::plan(&[change(
                "assignees",
                Editable::Multi {
                    value: vec!["../x".into()],
                    options: vec![]
                }
            )])
            .is_err()
        );
        assert!(super::plan(&[change("priority", Editable::Text { value: "x".into() })]).is_err());
    }

    #[test]
    fn toggles_are_the_difference() {
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let same = |a: &String, b: &String| a.eq_ignore_ascii_case(b);
        assert_eq!(
            toggled(&s(&["bug", "web"]), &s(&["Web", "ui"]), same),
            s(&["bug", "ui"])
        );
        assert!(toggled(&s(&["a"]), &s(&["a"]), same).is_empty());
    }

    #[test]
    fn only_ids_go_into_a_path() {
        let stored = Stored {
            source: crate::tracker::Source {
                kind: Kind::Fizzy,
                name: "fizzy".into(),
                ..Default::default()
            },
            token: Some("t".into()),
        };
        assert!(transition(&stored, "#4", "column:../x").is_err());
        assert!(transition(&stored, "#4", "nowhere").is_err());
        assert!(edit_comment(&stored, "#4", "a/b", "x").is_err());
        assert!(delete_comment(&stored, "#4", "").is_err());
        assert!(Card::open(&stored, "PROJ-4").is_err());
        assert!(Card::open(&stored, "#4/../5").is_err());
    }
}
