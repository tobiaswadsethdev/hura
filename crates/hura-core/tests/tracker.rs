//! Reading tickets, against a tracker on loopback.
//!
//! The parsers are unit-tested against captured answers; what this covers is
//! everything between them and the wire: the curl configuration, the
//! credential going in on stdin, the one search a Jira tracker makes.
//!
//! It stands up thirty lines of HTTP on `127.0.0.1` rather than mocking the
//! module, because the question worth asking is "does curl send what we
//! think", and that does not survive being answered by a fake in the same
//! process.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Sender, channel};

use hura_core::tracker::markdown::Markdown;
use hura_core::tracker::{
    Editable, FieldChange, Kind, Relation, Source, Stored, comment, delete_comment, inbox, save,
    transition,
};

/// One request the stand-in received.
#[derive(Debug)]
struct Seen {
    method: String,
    path: String,
    auth: String,
    body: String,
}

/// A tracker on loopback that answers `requests` requests with `respond`.
fn answering(requests: usize, seen: Sender<Seen>, respond: fn(&str, &str) -> (u16, String)) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for _ in 0..requests {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            serve(stream, &seen, respond);
        }
    });
    port
}

fn serve(mut stream: TcpStream, seen: &Sender<Seen>, respond: fn(&str, &str) -> (u16, String)) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut length = 0usize;
    let mut auth = String::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some(v) = header.strip_prefix("Content-Length: ") {
            length = v.trim().parse().unwrap_or(0);
        }
        if let Some(v) = header.strip_prefix("Authorization: ") {
            auth = v.trim().to_string();
        }
    }
    let mut body = vec![0u8; length];
    if length > 0 {
        reader.read_exact(&mut body).unwrap();
    }
    let body = String::from_utf8_lossy(&body).into_owned();

    let (status, payload) = respond(&method, &path);
    let answer = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
        payload.len()
    );
    let _ = stream.write_all(answer.as_bytes());
    let _ = stream.flush();
    let _ = seen.send(Seen {
        method,
        path,
        auth,
        body,
    });
}

/// One search for what is yours, after one question about who you are: the
/// answer to that question is what puts each ticket in its group.
#[test]
fn a_jira_tracker_is_one_search_grouped_by_whose_it_is() {
    let (tx, rx) = channel();
    // `/myself`, then the search.
    let port = answering(2, tx, |_, path| {
        if path.contains("/myself") {
            return (200, r#"{"accountId":"me-1"}"#.to_string());
        }
        let body = r#"{"issues":[
            {"key":"PROJ-12","fields":{
              "summary":"Add changelog","status":{"name":"To Do"},
              "issuetype":{"name":"Story"},"updated":"2026-09-28T10:00:00.000+0000",
              "assignee":{"accountId":"me-1"},
              "comment":{"total":1,"comments":[
                {"created":"2026-09-28T09:00:00.000+0000",
                 "author":{"accountId":"me-1","displayName":"Tobias"}}]}}},
            {"key":"PROJ-13","fields":{
              "summary":"Reported, handed on","status":{"name":"To Do"},
              "issuetype":{"name":"Bug"},"assignee":{"accountId":"them-2"}}}]}"#;
        (200, body.to_string())
    });
    let source = Source {
        kind: Kind::Jira,
        name: "work".into(),
        site: Some(format!("http://127.0.0.1:{port}")),
        email: Some("you@example.com".into()),
        ..Default::default()
    };

    let got = inbox(
        &[Stored {
            source,
            token: Some("a-token".into()),
        }],
        "tobias",
    );
    assert!(got.warnings.is_empty(), "{:?}", got.warnings);
    let groups: Vec<(&str, Relation)> = got
        .tasks
        .iter()
        .map(|t| (t.key.as_str(), t.relation))
        .collect();
    assert_eq!(
        groups,
        [
            ("PROJ-12", Relation::Assigned),
            ("PROJ-13", Relation::Created)
        ]
    );
    assert_eq!(got.tasks[0].comments, Some(1));
    assert!(
        got.tasks[0].last_comment_mine,
        "the comment is the credential owner's"
    );
    assert_eq!(got.read, ["work"]);
    assert!(
        got.pulls.is_empty() && got.pulls_read.is_empty(),
        "Jira has none"
    );

    let seen: Vec<Seen> = rx.try_iter().collect();
    // The token arrived, as Jira Cloud's Basic of email and token, through
    // curl's stdin, since it is in no argument this process passed.
    assert!(
        seen.iter().all(|s| s.auth.starts_with("Basic ")),
        "{seen:?}"
    );
    let paths: Vec<String> = seen.into_iter().map(|s| s.path).collect();
    assert_eq!(paths.len(), 2, "{paths:?}");
    let search = paths.iter().find(|p| p.contains("/search/jql")).unwrap();
    assert!(
        search.contains("reporter%20%3D%20currentUser%28%29"),
        "{search}"
    );
    assert!(
        search.contains("assignee") && search.contains("comment"),
        "{search}"
    );
}

/// A tracker with no token yet is a warning naming it, and nothing is asked:
/// a request without a credential is a 401 on a timer.
#[test]
fn a_tracker_without_a_token_is_a_warning_and_no_request() {
    let got = inbox(
        &[Stored {
            source: Source {
                kind: Kind::Jira,
                name: "work".into(),
                site: Some("http://127.0.0.1:9".into()),
                email: Some("you@example.com".into()),
                ..Default::default()
            },
            token: None,
        }],
        "tobias",
    );
    assert!(got.tasks.is_empty());
    assert!(got.read.is_empty(), "nothing was read");
    assert_eq!(got.warnings.len(), 1, "{:?}", got.warnings);
    assert!(got.warnings[0].contains("no token"), "{:?}", got.warnings);
}

/// A token Jira will not accept is one warning, found by the first request,
/// and the search is not attempted: it would fail the same way, after the
/// same wait.
#[test]
fn a_refused_token_is_one_warning_and_no_searches() {
    let (tx, rx) = channel();
    let port = answering(1, tx, |_, _| {
        (
            401,
            r#"{"errorMessages":["Client must be authenticated"]}"#.to_string(),
        )
    });
    let source = Source {
        kind: Kind::Jira,
        name: "work".into(),
        site: Some(format!("http://127.0.0.1:{port}")),
        email: Some("you@example.com".into()),
        ..Default::default()
    };
    let got = inbox(
        &[Stored {
            source,
            token: Some("wrong".into()),
        }],
        "tobias",
    );
    assert!(got.read.is_empty());
    assert_eq!(got.warnings.len(), 1, "{:?}", got.warnings);
    assert!(got.warnings[0].starts_with("work:"), "{:?}", got.warnings);
    let paths: Vec<String> = rx.try_iter().map(|s| s.path).collect();
    assert!(paths.iter().all(|p| p.contains("/myself")), "{paths:?}");
}

fn jira_at(port: u16) -> Stored {
    Stored {
        source: Source {
            kind: Kind::Jira,
            name: "work".into(),
            site: Some(format!("http://127.0.0.1:{port}")),
            email: Some("you@example.com".into()),
            ..Default::default()
        },
        token: Some("a-token".into()),
    }
}

/// A save, a move and a comment, as they leave: the method, the path, and a
/// body in the shape Jira takes -- through curl, with the token on stdin.
#[test]
fn a_save_a_move_and_a_comment_go_out_as_jira_takes_them() {
    let (tx, rx) = channel();
    let port = answering(4, tx, |method, _| match method {
        "POST" => (201, r#"{"id":"1"}"#.to_string()),
        _ => (204, String::new()),
    });
    let jira = jira_at(port);

    save(
        &jira,
        "CODE-7",
        &[
            FieldChange {
                id: "summary".into(),
                edit: Editable::Text {
                    value: "A \"quoted\" title".into(),
                },
            },
            FieldChange {
                id: "description".into(),
                edit: Editable::Doc {
                    value: Markdown {
                        text: "**bold**".into(),
                        lost: vec![],
                    },
                },
            },
        ],
    )
    .unwrap();
    transition(&jira, "CODE-7", "21").unwrap();
    comment(&jira, "CODE-7", "on it").unwrap();
    delete_comment(&jira, "CODE-7", "10001").unwrap();

    let seen: Vec<Seen> = rx.try_iter().collect();
    let line: Vec<String> = seen
        .iter()
        .map(|s| format!("{} {}", s.method, s.path))
        .collect();
    assert_eq!(
        line,
        [
            "PUT /rest/api/3/issue/CODE-7",
            "POST /rest/api/3/issue/CODE-7/transitions",
            "POST /rest/api/3/issue/CODE-7/comment",
            "DELETE /rest/api/3/issue/CODE-7/comment/10001",
        ]
    );
    assert!(seen.iter().all(|s| s.auth.starts_with("Basic ")));

    let put: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(
        put["fields"]["summary"], "A \"quoted\" title",
        "survives the curl config"
    );
    assert_eq!(put["fields"]["description"]["type"], "doc");
    let moved: serde_json::Value = serde_json::from_str(&seen[1].body).unwrap();
    assert_eq!(moved, serde_json::json!({ "transition": { "id": "21" } }));
    let said: serde_json::Value = serde_json::from_str(&seen[2].body).unwrap();
    assert_eq!(said["body"]["content"][0]["content"][0]["text"], "on it");
    assert!(seen[3].body.is_empty(), "a delete has no body");
}

/// Jira's refusal of a write is per field, and is what the window says.
#[test]
fn a_refused_save_says_which_field_and_why() {
    let (tx, _rx) = channel();
    let port = answering(1, tx, |_, _| {
        (
            400,
            r#"{"errorMessages":[],"errors":{"customfield_10016":"Number value expected"}}"#
                .to_string(),
        )
    });
    let e = save(
        &jira_at(port),
        "CODE-7",
        &[FieldChange {
            id: "customfield_10016".into(),
            edit: Editable::Number { value: Some(3.0) },
        }],
    )
    .unwrap_err();
    assert!(
        e.contains("customfield_10016: Number value expected"),
        "{e}"
    );
}
