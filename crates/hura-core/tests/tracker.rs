//! Reading tickets, against a tracker on loopback.
//!
//! The parsers are unit-tested against captured answers; what this covers is
//! everything between them and the wire -- the curl configuration, the
//! credential going in on stdin, one search per filter.
//!
//! It stands up thirty lines of HTTP on `127.0.0.1` rather than mocking the
//! module, because the question worth asking is "does curl send what we
//! think", and that does not survive being answered by a fake in the same
//! process.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Sender, channel};

use hura_core::tracker::{Filter, Kind, Source, Stored, board, inbox};

/// One request the stand-in received.
#[derive(Debug)]
struct Seen {
    path: String,
    auth: String,
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
    let _ = body;
    let _ = seen.send(Seen { path, auth });
}

/// Every filter is its own search, each row says which filter found it, and a
/// filter that fails is a warning that leaves the others -- and the record of
/// which ones were read -- alone.
#[test]
fn each_filter_is_its_own_search_and_a_failing_one_is_only_a_warning() {
    let (tx, rx) = channel();
    // `/myself`, then one search per filter.
    let port = answering(4, tx, |_, path| {
        if path.contains("/myself") {
            return (200, r#"{"accountId":"me-1"}"#.to_string());
        }
        if path.contains("Ready") {
            let body = r#"{"issues":[{"key":"PROJ-12","fields":{
                "summary":"Add changelog","status":{"name":"To Do"},
                "issuetype":{"name":"Story"},"updated":"2026-09-28T10:00:00.000+0000",
                "comment":{"total":1,"comments":[
                  {"created":"2026-09-28T09:00:00.000+0000",
                   "author":{"accountId":"me-1","displayName":"Tobias"}}]}}}]}"#;
            return (200, body.to_string());
        }
        if path.contains("broken") {
            return (
                400,
                r#"{"errorMessages":["the JQL is not valid"]}"#.to_string(),
            );
        }
        (200, r#"{"issues":[]}"#.to_string())
    });
    let source = Source {
        kind: Kind::Jira,
        name: "work".into(),
        site: Some(format!("http://127.0.0.1:{port}")),
        email: Some("you@example.com".into()),
        filters: vec![
            Filter {
                name: "ready to start".into(),
                query: "status = Ready".into(),
            },
            Filter {
                name: "assigned to me".into(),
                query: "assignee = currentUser()".into(),
            },
            Filter {
                name: "typo".into(),
                query: "broken".into(),
            },
        ],
        ..Default::default()
    };

    let got = inbox(
        &[Stored {
            source,
            token: Some("a-token".into()),
        }],
        "tobias",
    );
    assert_eq!(got.tasks.len(), 1, "{got:?}");
    let t = &got.tasks[0];
    assert_eq!(t.filter, "ready to start");
    assert_eq!(t.comments, Some(1));
    assert!(t.last_comment_mine, "the comment is the credential owner's");

    let read: Vec<&str> = got.read.iter().map(|r| r.filter.as_str()).collect();
    assert_eq!(read, ["ready to start", "assigned to me"]);
    assert_eq!(got.warnings.len(), 1, "{:?}", got.warnings);
    assert!(
        got.warnings[0].starts_with("work · typo:"),
        "{:?}",
        got.warnings
    );

    let seen: Vec<Seen> = rx.try_iter().collect();
    // The token arrived, as Jira Cloud's Basic of email and token -- through
    // curl's stdin, since it is in no argument this process passed.
    assert!(
        seen.iter().all(|s| s.auth.starts_with("Basic ")),
        "{seen:?}"
    );
    let paths: Vec<String> = seen.into_iter().map(|s| s.path).collect();
    assert_eq!(
        paths.iter().filter(|p| p.contains("/myself")).count(),
        1,
        "{paths:?}"
    );
    assert!(
        paths
            .iter()
            .all(|p| !p.contains("/search/jql") || p.contains("comment"))
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
/// and no search is attempted: each would fail the same way.
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
        filters: vec![
            Filter {
                name: "a".into(),
                query: "x = 1".into(),
            },
            Filter {
                name: "b".into(),
                query: "y = 2".into(),
            },
        ],
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

/// A scrum board is its running sprint: the configuration, the active
/// sprints, then the sprint's issues a page at a time until the total is in,
/// each card in the column that holds its status.
#[test]
fn a_scrum_board_reads_its_sprint_a_page_at_a_time_into_its_columns() {
    let (tx, rx) = channel();
    let port = answering(5, tx, |_, path| {
        if path.contains("/myself") {
            return (200, r#"{"accountId":"me-1"}"#.to_string());
        }
        if path.contains("/configuration") {
            let body = r#"{"name":"Team","type":"scrum","columnConfig":{"columns":[
                {"name":"To Do","statuses":[{"id":"1"}]},
                {"name":"Doing","statuses":[{"id":"3"}]}]}}"#;
            return (200, body.to_string());
        }
        if path.contains("/sprint?state=active") {
            return (
                200,
                r#"{"values":[{"id":41,"name":"Sprint 12"}]}"#.to_string(),
            );
        }
        let issue = |key: &str, status: &str| {
            format!(
                r#"{{"key":"{key}","fields":{{"summary":"t","status":{{"id":"{status}","name":"s"}},"issuetype":{{"name":"Task"}}}}}}"#
            )
        };
        if path.contains("startAt=0") {
            let body = format!(
                r#"{{"total":3,"issues":[{},{}]}}"#,
                issue("T-1", "3"),
                issue("T-2", "1")
            );
            return (200, body);
        }
        (
            200,
            format!(r#"{{"total":3,"issues":[{}]}}"#, issue("T-3", "1")),
        )
    });

    let got = board(&jira_at(port), "7", "tobias").unwrap();
    assert_eq!(got.name, "Team");
    assert_eq!(got.sprints, ["Sprint 12"]);
    assert_eq!((got.read, got.total), (3, 3));
    let keys = |i: usize| {
        got.columns[i]
            .tasks
            .iter()
            .map(|t| t.key.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(0), ["T-2", "T-3"]);
    assert_eq!(keys(1), ["T-1"]);

    let paths: Vec<String> = rx.try_iter().map(|s| s.path).collect();
    let issues: Vec<&String> = paths
        .iter()
        .filter(|p| p.contains("/board/7/issue"))
        .collect();
    assert_eq!(issues.len(), 2, "{paths:?}");
    assert!(issues[0].contains("sprint%20in%20%2841%29"), "{paths:?}");
    assert!(issues[0].contains("ORDER%20BY%20Rank"), "{paths:?}");
    assert!(issues[1].contains("startAt=2"), "{paths:?}");
}

/// With no sprint running there is nothing on a scrum board, and nothing is
/// searched for: the board says why instead.
#[test]
fn a_scrum_board_with_no_sprint_running_says_so_and_searches_nothing() {
    let (tx, rx) = channel();
    let port = answering(3, tx, |_, path| {
        if path.contains("/myself") {
            return (200, r#"{"accountId":"me-1"}"#.to_string());
        }
        if path.contains("/configuration") {
            let body = r#"{"name":"Team","type":"scrum","columnConfig":{"columns":[
                {"name":"To Do","statuses":[{"id":"1"}]}]}}"#;
            return (200, body.to_string());
        }
        (200, r#"{"values":[]}"#.to_string())
    });

    let got = board(&jira_at(port), "7", "tobias").unwrap();
    assert!(got.note.unwrap().contains("no sprint"));
    assert_eq!(got.columns.len(), 1, "the columns are still drawn");
    assert!(got.columns[0].tasks.is_empty());
    let paths: Vec<String> = rx.try_iter().map(|s| s.path).collect();
    assert!(paths.iter().all(|p| !p.contains("/issue")), "{paths:?}");
}
