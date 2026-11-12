//! Whether a Jira filter holds an issue a webhook told of: what makes an
//! issue that only a filter shows a notification within seconds, too.
//!
//! **A delivery that is not plainly yours is queued** (see
//! [`crate::webhooks::jira`]) when its tracker has filters: an issue not
//! assigned to you, not reported by you and not in your last read may still
//! have just come into one. A webhook's own JQL cannot say what a filter
//! says, so Jira is asked here, with the filters as written.
//!
//! **A tracker's queue is one search.** Every few seconds what has waited
//! [`SETTLE`] is taken, and each tracker's issues are asked about at once:
//! `issuekey in (…) AND ((filter) OR (filter) …)`, a hundred keys at a time.
//! However busy a site is, an account costs a search a pass at most. The
//! wait is for Jira's search, which lags a change by a few seconds; a match
//! makes the account due, and the read finds what changed as any read does.
//!
//! **Anything that goes wrong leaves it to the timer.** A filter Jira
//! refuses is left out and the rest asked again; a site that says to slow
//! down is said in the log, which is where to look for how far this goes.

use std::time::{Duration, Instant};

use hura_core::tracker::Kind;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::Shared;
use crate::error::Result;

/// How often the queue is looked at.
const EVERY: Duration = Duration::from_secs(5);
/// How long an issue waits before it is asked about.
const SETTLE: &str = "5 seconds";
/// Keys in one search: JQL has a length limit, and this stays far below it.
const KEYS: usize = 100;

/// Forever, from the start of the process.
pub async fn run(state: Shared) {
    loop {
        if let Err(e) = tick(&state).await {
            eprintln!("jira filters: {e:?}");
        }
        tokio::time::sleep(EVERY).await;
    }
}

/// What one pass did, for the log.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Pass {
    pub issues: usize,
    pub trackers: usize,
    pub searches: usize,
    pub due: usize,
    pub failed: usize,
    pub slowed: usize,
}

/// Asks about whatever has waited long enough.
pub async fn tick(state: &Shared) -> Result<Pass> {
    let started = Instant::now();
    let taken: Vec<(Uuid, String, String)> = sqlx::query_as(&format!(
        "delete from jira_checks where queued_at <= now() - interval '{SETTLE}'
         returning account_id, tracker, issue_key"
    ))
    .fetch_all(&state.db)
    .await?;
    let mut pass = Pass {
        issues: taken.len(),
        ..Pass::default()
    };
    if taken.is_empty() {
        return Ok(pass);
    }
    // Only accounts the watcher reads, and not one already due: its read
    // comes anyway.
    let accounts: Vec<Uuid> = taken.iter().map(|(a, ..)| *a).collect();
    let wanted: Vec<Uuid> = sqlx::query_scalar(
        "select w.account_id from watches w
         where w.account_id = any($1) and w.next_read_at > now()
           and (w.account_id = any($2)
                or exists (select 1 from push_routes p
                           where p.account_id = w.account_id and p.broken is null))",
    )
    .bind(&accounts)
    .bind(state.live.accounts())
    .fetch_all(&state.db)
    .await?;

    let mut groups: Vec<(Uuid, String, Vec<String>)> = Vec::new();
    for (account, tracker, key) in taken {
        if !wanted.contains(&account) {
            continue;
        }
        match groups
            .iter_mut()
            .find(|(a, t, _)| *a == account && *t == tracker)
        {
            Some((_, _, keys)) => keys.push(key),
            None => groups.push((account, tracker, vec![key])),
        }
    }
    pass.trackers = groups.len();
    let mut due: Vec<Uuid> = Vec::new();
    for (account, tracker, keys) in groups {
        if due.contains(&account) {
            continue;
        }
        match holds(state, account, &tracker, &keys, &mut pass).await {
            Ok(true) => due.push(account),
            Ok(false) => {}
            Err(e) => {
                pass.failed += 1;
                eprintln!("jira filters {account} {tracker}: {e:?}");
            }
        }
    }
    pass.due = due.len();
    if !due.is_empty() {
        sqlx::query("update watches set next_read_at = now() where account_id = any($1)")
            .bind(&due)
            .execute(&state.db)
            .await?;
    }
    eprintln!(
        "jira filters: {} issue(s) on {} tracker(s): {} search(es), {} due, {} failed, {} slowed down, {}ms",
        pass.issues,
        pass.trackers,
        pass.searches,
        pass.due,
        pass.failed,
        pass.slowed,
        started.elapsed().as_millis()
    );
    Ok(pass)
}

/// Whether any of a tracker's filters holds any of these issues.
async fn holds(
    state: &Shared,
    account: Uuid,
    name: &str,
    keys: &[String],
    pass: &mut Pass,
) -> Result<bool> {
    let tracker = crate::trackers::stored(state, account)
        .await?
        .into_iter()
        .find(|t| t.source.name == name && t.source.kind == Kind::Jira);
    let Some(tracker) = tracker else {
        return Ok(false);
    };
    let (Some(cloud_id), Some(token)) = (&tracker.source.cloud_id, &tracker.token) else {
        return Ok(false);
    };
    let mut filters: Vec<(String, String)> = tracker
        .source
        .jql_filters
        .iter()
        .map(|f| (f.name.clone(), without_order(&f.jql).to_string()))
        .collect();
    let url = format!("https://api.atlassian.com/ex/jira/{cloud_id}/rest/api/3/search/jql");
    for chunk in keys.chunks(KEYS) {
        if filters.is_empty() {
            break;
        }
        {
            let jql = query(chunk, filters.iter().map(|(_, j)| j.as_str()));
            pass.searches += 1;
            match search(state, &url, token, &jql).await {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err((429, why)) => {
                    pass.slowed += 1;
                    eprintln!("jira filters {account} {name}: slowed down: {why}");
                    return Ok(false);
                }
                // One filter Jira refuses fails them all together: each is
                // tried alone, and only the ones that work are asked about
                // the next hundred.
                Err((400, why)) if filters.len() > 1 => {
                    eprintln!("jira filters {account} {name}: {why}; trying each alone");
                    let mut working = Vec::new();
                    for (filter, jql) in &filters {
                        pass.searches += 1;
                        match search(state, &url, token, &query(chunk, [jql.as_str()])).await {
                            Ok(true) => return Ok(true),
                            Ok(false) => working.push((filter.clone(), jql.clone())),
                            Err((_, why)) => {
                                eprintln!("jira filters {account} {name} · {filter}: {why}")
                            }
                        }
                    }
                    filters = working;
                }
                Err((_, why)) => {
                    pass.failed += 1;
                    eprintln!("jira filters {account} {name}: {why}");
                    return Ok(false);
                }
            }
        }
    }
    Ok(false)
}

/// One search, asking only whether it found anything.
async fn search(
    state: &Shared,
    url: &str,
    token: &str,
    jql: &str,
) -> std::result::Result<bool, (u16, String)> {
    let body = json!({ "jql": jql, "maxResults": 1, "fields": ["key"] });
    let (status, text) = state
        .web
        .send(
            "POST",
            url,
            &[
                ("Authorization", format!("Bearer {token}")),
                ("Accept", "application/json".into()),
                ("Content-Type", "application/json".into()),
            ],
            Some(body.to_string()),
        )
        .await
        .map_err(|e| (0, e))?;
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    if (200..300).contains(&status) {
        return Ok(v["issues"].as_array().is_some_and(|i| !i.is_empty()));
    }
    let said = v["errorMessages"]
        .as_array()
        .and_then(|m| m.first())
        .and_then(Value::as_str)
        .unwrap_or("no reason given");
    Err((status, format!("{status}: {said}")))
}

/// `issuekey in (…) AND ((a) OR (b))`: these issues, in any of the filters.
/// A filter of nothing but an order holds every issue, and leaves only the
/// keys.
fn query<'a>(keys: &[String], filters: impl IntoIterator<Item = &'a str>) -> String {
    let filters: Vec<&str> = filters.into_iter().collect();
    let keys = keys.join(", ");
    if filters.iter().any(|f| f.trim().is_empty()) {
        return format!("issuekey in ({keys})");
    }
    let any = filters
        .iter()
        .map(|f| format!("({f})"))
        .collect::<Vec<_>>()
        .join(" OR ");
    format!("issuekey in ({keys}) AND ({any})")
}

/// A filter without its `ORDER BY`, which cannot stand inside parentheses.
/// Only outside quotes: `summary ~ "order by"` is a query, not an order.
fn without_order(jql: &str) -> &str {
    let bytes = jql.as_bytes();
    let word = |at: usize, w: &[u8]| {
        bytes.len() >= at + w.len()
            && bytes[at..at + w.len()].eq_ignore_ascii_case(w)
            && !bytes
                .get(at + w.len())
                .is_some_and(u8::is_ascii_alphanumeric)
    };
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(_) if c == b'\\' => i += 1,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => quote = Some(c),
            None if (i == 0 || !bytes[i - 1].is_ascii_alphanumeric()) && word(i, b"order") => {
                let mut by = i + 5;
                while bytes.get(by).is_some_and(u8::is_ascii_whitespace) {
                    by += 1;
                }
                if by > i + 5 && word(by, b"by") {
                    return jql[..i].trim();
                }
            }
            None => {}
        }
        i += 1;
    }
    jql.trim()
}

/// An issue key as Jira makes them, `ACME-12`: nothing else is put in a
/// query.
pub(crate) fn is_key(s: &str) -> bool {
    let Some((project, number)) = s.rsplit_once('-') else {
        return false;
    };
    project.starts_with(|c: char| c.is_ascii_alphabetic())
        && project
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
        && s.len() <= 64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_loses_its_order_but_not_a_quoted_one() {
        assert_eq!(
            without_order("project = A ORDER BY updated DESC"),
            "project = A"
        );
        assert_eq!(
            without_order("summary ~ \"order by\" order  by rank"),
            "summary ~ \"order by\""
        );
        assert_eq!(without_order("ORDER BY created"), "");
        assert_eq!(without_order("reorder = x"), "reorder = x");
        assert_eq!(without_order("orderly = 1"), "orderly = 1");
    }

    #[test]
    fn the_issues_are_asked_about_in_any_filter_at_once() {
        let keys = ["A-1".to_string(), "B-2".to_string()];
        assert_eq!(
            query(&keys, ["status = Ready", "labels = x OR labels = y"]),
            "issuekey in (A-1, B-2) AND ((status = Ready) OR (labels = x OR labels = y))"
        );
        assert_eq!(
            query(&keys, ["status = Ready", ""]),
            "issuekey in (A-1, B-2)"
        );
    }

    #[test]
    fn only_an_issue_key_goes_in_a_query() {
        assert!(is_key("ACME-12"));
        assert!(is_key("A_B2-1"));
        assert!(!is_key("ACME-"));
        assert!(!is_key("1A-2"));
        assert!(!is_key("A-1) OR (project = B"));
    }
}
