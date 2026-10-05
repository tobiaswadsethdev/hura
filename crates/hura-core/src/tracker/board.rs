//! A Jira board, read as Jira draws it: its columns, in order, and the issues
//! in each.
//!
//! The filters are questions somebody wrote; a board is the process the team
//! actually runs, and its columns are the answer to "where is everything".
//! Neither replaces the other, so the tickets screen shows one or the other
//! and you switch.
//!
//! Three questions to Jira Software's own API, all reads:
//!
//! - `GET /rest/agile/1.0/board` -- which boards this account can see, which
//!   is the picker.
//! - `GET /rest/agile/1.0/board/{id}/configuration` -- the columns, and which
//!   statuses each one holds. A column is a set of status ids, so a card's
//!   column is found from its status and not from its status's name, which two
//!   workflows may share.
//! - `GET /rest/agile/1.0/board/{id}/issue` -- what is on it, already limited
//!   to the board's own filter, answered in the search's shape so a card here
//!   is the same card as on a filter.
//!
//! A scrum board is its active sprint, which is one more question first: with
//! no sprint running Jira's board is empty, and so is this one, with a note
//! saying why. A kanban board is everything not done, and done for the two
//! weeks Jira itself keeps showing it -- without that, the last column is
//! every ticket the team ever closed.
//!
//! Jira only. Azure DevOps has boards too, per team, with columns of their
//! own; a reader for those is a function here.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Kind, Stored, Task, get, jira_auth, jira_me, jira_site, jira_task, string, urlencode};

/// A board somebody can pick.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    /// Which configured tracker it is on, by name.
    pub tracker: String,
    /// Jira's number for it, as text: it only ever goes back into a URL.
    pub id: String,
    pub name: String,
    /// `scrum`, `kanban` or `simple` -- the last being a team-managed
    /// project's board -- as Jira calls them.
    pub board_type: String,
    /// The project it lives in, `Inet development (CODE)`, when it is in one.
    /// What tells two boards called "Board" apart.
    pub project: Option<String>,
}

/// Every board every Jira tracker can see, and whatever could not be read.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Boards {
    pub boards: Vec<Board>,
    pub warnings: Vec<String>,
}

/// One board, read: its columns in order, each with its cards.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardView {
    pub tracker: String,
    pub id: String,
    pub name: String,
    pub board_type: String,
    /// The board in a browser, for what only Jira does: dragging a card.
    pub url: String,
    /// The sprints it shows, for a scrum board. Usually one; a board can run
    /// parallel sprints, and then its cards are all of theirs.
    pub sprints: Vec<String>,
    pub columns: Vec<BoardColumn>,
    /// How many issues the board holds, and how many of them were read: a
    /// board is paged, and past [`MAX_ISSUES`] this stops asking and says so.
    pub total: u32,
    pub read: u32,
    /// Why there are no cards, when that is not obvious: a scrum board with
    /// no sprint running.
    pub note: Option<String>,
}

/// A column, as the board configures it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardColumn {
    pub name: String,
    /// The work-in-progress limits, when the board has them on.
    pub min: Option<u32>,
    pub max: Option<u32>,
    /// In the board's rank order, which is the order people put them in.
    pub tasks: Vec<Task>,
}

/// Where the picker stops. Fifty a page is all the board list answers with;
/// twenty pages is a thousand boards, past which nobody is scrolling a list.
const MAX_BOARD_PAGES: usize = 20;

/// Where a board stops being read. A column is a glance, and a kanban board
/// whose filter is a whole project's history is a lot of glances: five pages
/// is enough for any board somebody actually works from, and the view says
/// when there was more.
pub const MAX_ISSUES: usize = 250;

/// What a kanban board shows of what is done: the last two weeks, which is
/// what Jira's own board keeps by default.
const RECENTLY_DONE: &str = "statusCategory != Done OR statusCategoryChangedDate >= -14d";

/// The fields a card is made of -- the search's, plus nothing: the status
/// carries its id, which is what places it in a column.
const FIELDS: &str = "summary,status,issuetype,updated,comment";

/// Every board each Jira tracker's account can see.
pub fn boards(trackers: &[Stored]) -> Boards {
    let mut out = Boards::default();
    for stored in trackers.iter().filter(|t| t.source.kind == Kind::Jira) {
        let source = &stored.source;
        let token = match ready(stored) {
            Ok(token) => token,
            Err(e) => {
                out.warnings.push(e);
                continue;
            }
        };
        match board_list(stored, token) {
            Ok(mut found) => out.boards.append(&mut found),
            Err(e) => out.warnings.push(format!("{}: {e}", source.name)),
        }
    }
    out
}

fn board_list(stored: &Stored, token: &str) -> Result<Vec<Board>, String> {
    let source = &stored.source;
    let auth = jira_auth(source, token);
    let mut boards = Vec::new();
    for _ in 0..MAX_BOARD_PAGES {
        let url = format!(
            "{}/rest/agile/1.0/board?startAt={}&maxResults=50",
            jira_site(source),
            boards.len()
        );
        let page = get(&url, &auth, &[])?;
        let (mut found, last) = parse_boards(&page, &source.name)?;
        let empty = found.is_empty();
        boards.append(&mut found);
        if last || empty {
            break;
        }
    }
    // By project, then by name: the picker is read as "the CODE boards", and
    // the order Jira answers in is the order they were made.
    boards.sort_by(|a, b| (&a.project, &a.name).cmp(&(&b.project, &b.name)));
    Ok(boards)
}

/// One page of the board list, and whether it was the last.
fn parse_boards(body: &Value, tracker: &str) -> Result<(Vec<Board>, bool), String> {
    let values = body
        .get("values")
        .and_then(Value::as_array)
        .ok_or("jira did not answer with boards")?;
    let boards = values
        .iter()
        .filter_map(|b| {
            let id = b.get("id")?.as_u64()?;
            let location = b.get("location");
            Some(Board {
                tracker: tracker.to_string(),
                id: id.to_string(),
                name: string(b, "name"),
                board_type: string(b, "type"),
                project: location
                    .map(|l| string(l, "displayName"))
                    .filter(|s| !s.is_empty()),
            })
        })
        .collect();
    // `isLast` is the board list's own word for it; a server old enough not
    // to send one is taken at a short page.
    let last = body
        .get("isLast")
        .and_then(Value::as_bool)
        .unwrap_or(values.len() < 50);
    Ok((boards, last))
}

/// Read one board: its columns, and the issues in each.
pub fn board(stored: &Stored, id: &str, prefix: &str) -> Result<BoardView, String> {
    let source = &stored.source;
    if source.kind != Kind::Jira {
        return Err(format!(
            "{} has no boards here; only Jira's are read so far",
            source.kind.label()
        ));
    }
    let token = ready(stored)?;
    // It goes into a URL path, so it has to be a number and nothing else.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("`{id}` is not a Jira board"));
    }
    let site = jira_site(source);
    let auth = jira_auth(source, token);
    let me = jira_me(source, token)?;

    let config = get(
        &format!("{site}/rest/agile/1.0/board/{id}/configuration"),
        &auth,
        &[],
    )?;
    let layout = parse_configuration(&config)?;

    // What, of everything the board's filter matches, the board shows.
    let mut sprints = Vec::new();
    let scope = if layout.board_type == "scrum" {
        let answer = get(
            &format!("{site}/rest/agile/1.0/board/{id}/sprint?state=active"),
            &auth,
            &[],
        )?;
        sprints = parse_sprints(&answer);
        if sprints.is_empty() {
            return Ok(view(
                stored,
                id,
                layout,
                Vec::new(),
                Vec::new(),
                0,
                Some("no sprint is running on this board".into()),
            ));
        }
        let ids: Vec<&str> = sprints.iter().map(|(id, _)| id.as_str()).collect();
        format!("sprint in ({})", ids.join(", "))
    } else {
        match &layout.sub_query {
            Some(sub) => format!("({sub}) AND ({RECENTLY_DONE})"),
            None => RECENTLY_DONE.to_string(),
        }
    };
    let jql = format!("{scope} ORDER BY Rank ASC");

    let mut issues: Vec<Value> = Vec::new();
    let mut total = 0;
    while issues.len() < MAX_ISSUES {
        let url = format!(
            "{site}/rest/agile/1.0/board/{id}/issue?jql={}&fields={FIELDS}&startAt={}&maxResults=50",
            urlencode(&jql),
            issues.len()
        );
        let page = get(&url, &auth, &[])?;
        let found = page
            .get("issues")
            .and_then(Value::as_array)
            .ok_or("jira did not answer with issues")?;
        total = page
            .get("total")
            .and_then(Value::as_u64)
            .map_or(0, |t| t as usize);
        if found.is_empty() {
            break;
        }
        issues.extend(found.iter().cloned());
        if issues.len() >= total {
            break;
        }
    }
    issues.truncate(MAX_ISSUES);
    let total = total.max(issues.len());

    let tasks = issues
        .iter()
        .filter_map(|i| {
            let status = i
                .get("fields")
                .and_then(|f| f.get("status"))
                .map(|s| string(s, "id"))?;
            Some((status, jira_task(i, source, me.as_deref(), prefix)?))
        })
        .collect();
    Ok(view(
        stored,
        id,
        layout,
        sprints.into_iter().map(|(_, name)| name).collect(),
        tasks,
        total,
        None,
    ))
}

/// The tracker's token, or why there is none to use.
fn ready(stored: &Stored) -> Result<&str, String> {
    let source = &stored.source;
    if let Some(problem) = source.problem() {
        return Err(problem);
    }
    stored
        .token
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| format!("{}: no token yet, so nothing was read", source.name))
}

/// What a board's configuration says about drawing it.
#[derive(Debug)]
struct Layout {
    name: String,
    board_type: String,
    /// A kanban board's sub-filter: what of its filter it actually shows.
    sub_query: Option<String>,
    columns: Vec<(BoardColumn, Vec<String>)>,
}

fn parse_configuration(body: &Value) -> Result<Layout, String> {
    let columns = body
        .get("columnConfig")
        .and_then(|c| c.get("columns"))
        .and_then(Value::as_array)
        .ok_or("jira did not answer with the board's columns")?;
    // A limit is only a limit when the board says it counts something; with
    // the constraint off, Jira keeps the numbers and ignores them.
    let limited = body
        .get("columnConfig")
        .map(|c| string(c, "constraintType"))
        .is_some_and(|t| !t.is_empty() && t != "none");
    let limit = |c: &Value, k: &str| {
        c.get(k)
            .and_then(Value::as_u64)
            .filter(|_| limited)
            .map(|n| n as u32)
    };
    Ok(Layout {
        name: string(body, "name"),
        board_type: string(body, "type"),
        sub_query: body
            .get("subQuery")
            .map(|q| string(q, "query"))
            .filter(|q| !q.trim().is_empty()),
        columns: columns
            .iter()
            .map(|c| {
                let statuses: Vec<String> = c
                    .get("statuses")
                    .and_then(Value::as_array)
                    .map(|s| s.iter().map(|s| string(s, "id")).collect())
                    .unwrap_or_default();
                let column = BoardColumn {
                    name: string(c, "name"),
                    min: limit(c, "min"),
                    max: limit(c, "max"),
                    tasks: Vec::new(),
                };
                (column, statuses)
            })
            // A column with no status in it can never hold a card -- it is
            // what a kanban board's "Backlog" is when the backlog is off --
            // and Jira does not draw one.
            .filter(|(_, statuses)| !statuses.is_empty())
            .collect(),
    })
}

/// The active sprints' ids and names.
fn parse_sprints(body: &Value) -> Vec<(String, String)> {
    body.get("values")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|s| Some((s.get("id")?.as_u64()?.to_string(), string(s, "name"))))
                .collect()
        })
        .unwrap_or_default()
}

/// Every task into the column that holds its status, in the order they came.
/// A status no column holds is one Jira's board does not show either.
fn view(
    stored: &Stored,
    id: &str,
    layout: Layout,
    sprints: Vec<String>,
    tasks: Vec<(String, Task)>,
    total: usize,
    note: Option<String>,
) -> BoardView {
    let source = &stored.source;
    let mut columns: Vec<BoardColumn> = Vec::with_capacity(layout.columns.len());
    let mut home: HashMap<String, usize> = HashMap::new();
    for (i, (column, statuses)) in layout.columns.into_iter().enumerate() {
        for status in statuses {
            home.entry(status).or_insert(i);
        }
        columns.push(column);
    }
    let read = tasks.len() as u32;
    for (status, mut task) in tasks {
        if let Some(&i) = home.get(&status) {
            // Which column it came from, the way a filter's card says which
            // filter: it is what tells the card's two copies apart nowhere
            // else, and costs nothing here.
            task.filter = columns[i].name.clone();
            columns[i].tasks.push(task);
        }
    }
    BoardView {
        tracker: source.name.clone(),
        id: id.to_string(),
        name: layout.name,
        board_type: layout.board_type,
        url: format!(
            "{}/secure/RapidBoard.jspa?rapidView={id}",
            jira_site(source)
        ),
        sprints,
        columns,
        total: total as u32,
        read,
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::Source;

    fn stored() -> Stored {
        Stored {
            source: Source {
                kind: Kind::Jira,
                name: "work".into(),
                site: Some("https://example.atlassian.net/".into()),
                email: Some("you@example.com".into()),
                ..Default::default()
            },
            token: Some("tok".into()),
        }
    }

    /// `GET /rest/agile/1.0/board`, trimmed to what this reads.
    #[test]
    fn the_board_list_says_where_each_board_lives_and_when_it_ends() {
        let body = serde_json::json!({
            "maxResults": 50, "startAt": 0, "total": 2, "isLast": true,
            "values": [
                { "id": 18, "name": "CODE board", "type": "kanban",
                  "location": { "projectKey": "CODE", "displayName": "Inet development (CODE)" } },
                { "id": 7, "name": "Mine", "type": "scrum" }
            ]
        });
        let (boards, last) = parse_boards(&body, "work").unwrap();
        assert!(last);
        assert_eq!(boards.len(), 2);
        assert_eq!(boards[0].id, "18");
        assert_eq!(boards[0].board_type, "kanban");
        assert_eq!(
            boards[0].project.as_deref(),
            Some("Inet development (CODE)")
        );
        assert_eq!(
            boards[1].project, None,
            "a board on a filter has no project"
        );

        // No `isLast`: a full page is taken to mean there may be more.
        let full: Vec<Value> = (0..50)
            .map(|i| serde_json::json!({ "id": i, "name": "b", "type": "kanban" }))
            .collect();
        let (_, last) = parse_boards(&serde_json::json!({ "values": full }), "work").unwrap();
        assert!(!last);
    }

    /// `GET /rest/agile/1.0/board/{id}/configuration`, as a kanban board with
    /// its backlog switched off answers it.
    const CONFIGURATION: &str = r#"{
      "id": 18, "name": "CODE board", "type": "kanban",
      "subQuery": { "query": "fixVersion in unreleasedVersions() OR fixVersion is EMPTY" },
      "columnConfig": {
        "constraintType": "issueCount",
        "columns": [
          { "name": "Backlog", "statuses": [] },
          { "name": "ToDo", "statuses": [{ "id": "1" }, { "id": "10074" }], "max": 19 },
          { "name": "Doing", "statuses": [{ "id": "10064" }] },
          { "name": "Done", "statuses": [{ "id": "10046" }] }
        ]
      }
    }"#;

    #[test]
    fn a_configuration_is_its_columns_in_order_and_the_statuses_each_holds() {
        let layout = parse_configuration(&serde_json::from_str(CONFIGURATION).unwrap()).unwrap();
        assert_eq!(layout.name, "CODE board");
        assert_eq!(layout.board_type, "kanban");
        assert_eq!(
            layout.sub_query.as_deref(),
            Some("fixVersion in unreleasedVersions() OR fixVersion is EMPTY")
        );
        let names: Vec<_> = layout
            .columns
            .iter()
            .map(|(c, _)| c.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["ToDo", "Doing", "Done"],
            "a column with no status is not drawn"
        );
        assert_eq!(layout.columns[0].1, ["1", "10074"]);
        assert_eq!(layout.columns[0].0.max, Some(19));
        assert_eq!(layout.columns[1].0.max, None);
    }

    /// With the constraint off, Jira keeps the numbers and ignores them.
    #[test]
    fn a_limit_the_board_does_not_enforce_is_not_shown() {
        let body = serde_json::json!({
            "name": "MKT board", "type": "simple",
            "columnConfig": { "constraintType": "none", "columns": [
                { "name": "Doing", "statuses": [{ "id": "3" }], "max": 8 }
            ] }
        });
        let layout = parse_configuration(&body).unwrap();
        assert_eq!(layout.columns[0].0.max, None);
        assert_eq!(layout.sub_query, None);
    }

    /// Cards go to the column that holds their status *id*, in the order they
    /// came, and a status no column holds is not on the board.
    #[test]
    fn a_card_finds_its_column_by_status_id() {
        let layout = parse_configuration(&serde_json::from_str(CONFIGURATION).unwrap()).unwrap();
        let s = stored();
        let issue = |key: &str, status: &str| {
            serde_json::json!({ "key": key, "fields": {
                "summary": format!("{key} title"),
                "status": { "id": status, "name": "whatever" },
                "issuetype": { "name": "Task" }
            } })
        };
        let tasks: Vec<(String, Task)> = [
            issue("CODE-3", "10074"),
            issue("CODE-1", "10064"),
            issue("CODE-2", "1"),
            issue("CODE-9", "99999"),
        ]
        .iter()
        .map(|i| {
            let status = string(&i["fields"]["status"], "id");
            (status, jira_task(i, &s.source, None, "tobias").unwrap())
        })
        .collect();

        let v = view(&s, "18", layout, Vec::new(), tasks, 4, None);
        let keys = |c: &BoardColumn| c.tasks.iter().map(|t| t.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&v.columns[0]), ["CODE-3", "CODE-2"], "rank order kept");
        assert_eq!(keys(&v.columns[1]), ["CODE-1"]);
        assert!(v.columns[2].tasks.is_empty());
        let placed: usize = v.columns.iter().map(|c| c.tasks.len()).sum();
        assert_eq!(placed, 3, "the unmapped status is not on the board");
        assert_eq!((v.read, v.total), (4, 4), "and the board was not cut short");
        assert_eq!(v.columns[0].tasks[0].filter, "ToDo");
        assert_eq!(v.columns[0].tasks[0].branch, "tobias/CODE-3-code-3-title");
        assert_eq!(
            v.url,
            "https://example.atlassian.net/secure/RapidBoard.jspa?rapidView=18"
        );
    }

    #[test]
    fn the_active_sprints_are_read_by_id_and_name() {
        let body = serde_json::json!({ "values": [
            { "id": 41, "name": "CODE Sprint 12", "state": "active" }
        ] });
        assert_eq!(
            parse_sprints(&body),
            [("41".to_string(), "CODE Sprint 12".to_string())]
        );
        assert!(parse_sprints(&serde_json::json!({ "values": [] })).is_empty());
    }

    /// The id goes into a URL path, so it is checked rather than escaped --
    /// and before any request is made.
    #[test]
    fn only_a_board_number_is_asked_for() {
        let s = stored();
        for bad in ["", "18/../../myself", "18?x", "-1", "eighteen"] {
            let e = board(&s, bad, "t").unwrap_err();
            assert!(e.contains("not a Jira board"), "{bad}: {e}");
        }
        let mut github = stored();
        github.source.kind = Kind::GitHub;
        assert!(board(&github, "18", "t").unwrap_err().contains("only Jira"));
    }
}
