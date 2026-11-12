//! The webhook on a Jira site connected through Atlassian's OAuth: what
//! makes a change to one of your issues a notification within seconds
//! rather than at the account's next read.
//!
//! **One per connected site, made with its own token.** Atlassian keeps an
//! OAuth app's webhooks per person and site (five at most), lets them say
//! only a little JQL (no `currentUser()`, no saved filter) and sends all of
//! an app's to one address. So each asks for every issue and comment event
//! on the site, with an empty filter, and a delivery is matched here: to its
//! accounts by the webhook IDs it names, and to what is theirs by its issue
//! being assigned to or reported by whoever connected, or having been in
//! their last read (a filter's, too). Anything else is left to the timer,
//! which still finds a new issue that only a filter would show.
//!
//! **Renewed every day**, as part of the account's read: Jira stops sending
//! a webhook thirty days after it was made or last renewed, and one that is
//! gone (deleted, or expired while nobody was reading) is made again.
//! Removing the tracker removes it. A connection made before hura asked for
//! `manage:jira-webhook` cannot make one, and its row says to connect again.
//!
//! Only a tracker connected through Atlassian has one: an API token cannot
//! register a webhook, and is read on the timer as before.

use hura_core::tracker::{Kind, Stored};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::Shared;
use crate::error::Result;

/// How long a look at a tracker's webhook holds.
const RECHECK: &str = "1 day";

/// What Jira sends: whatever changes an issue, or its conversation.
pub const EVENTS: &[&str] = &[
    "jira:issue_created",
    "jira:issue_updated",
    "jira:issue_deleted",
    "comment_created",
    "comment_updated",
    "comment_deleted",
];

/// Where every webhook of the app sends: Atlassian allows one address.
pub(crate) fn address(state: &Shared) -> String {
    format!("{}/v1/webhooks/jira", state.url)
}

fn api(cloud_id: &str) -> String {
    format!("https://api.atlassian.com/ex/jira/{cloud_id}/rest/api/3")
}

/// A request Jira refused or did not answer: its status, 0 for none, and
/// what it said.
struct Failed(u16, String);

async fn call(
    state: &Shared,
    method: &str,
    url: &str,
    token: &str,
    body: Option<Value>,
) -> std::result::Result<Value, Failed> {
    let mut headers = vec![
        ("Authorization", format!("Bearer {token}")),
        ("Accept", "application/json".to_string()),
    ];
    if body.is_some() {
        headers.push(("Content-Type", "application/json".to_string()));
    }
    let (status, text) = state
        .web
        .send(method, url, &headers, body.map(|b| b.to_string()))
        .await
        .map_err(|e| Failed(0, e))?;
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    if (200..300).contains(&status) {
        return Ok(v);
    }
    let said = v["errorMessages"]
        .as_array()
        .and_then(|m| m.first())
        .and_then(Value::as_str)
        .or(v["message"].as_str())
        .unwrap_or("no reason given");
    Err(Failed(status, format!("{status}: {said}")))
}

/// A connected Jira tracker's webhook: made, or brought up to date. Nothing
/// for any other tracker. What happened is said on its row rather than
/// failing whatever asked.
pub async fn sync(state: &Shared, account: Uuid, name: &str) -> Result<()> {
    let tracker = crate::trackers::stored(state, account)
        .await?
        .into_iter()
        .find(|t| t.source.name == name);
    let Some(Stored {
        source,
        token: Some(token),
    }) = tracker.filter(|t| t.source.kind == Kind::Jira)
    else {
        return Ok(());
    };
    let Some(cloud_id) = source.cloud_id else {
        return Ok(());
    };
    let said = match look(state, account, name, &cloud_id, &token).await? {
        Ok(()) => "a webhook tells hura of changes at once".to_string(),
        Err(Failed(401 | 403, _)) => {
            "connect Jira again to let hura add a webhook; until then it is read every minute or so"
                .to_string()
        }
        Err(Failed(_, e)) => format!("webhook not added: {e}"),
    };
    sqlx::query(
        "update trackers set webhooks = $3, webhooks_checked_at = now()
         where account_id = $1 and name = $2",
    )
    .bind(account)
    .bind(name)
    .bind(said)
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn look(
    state: &Shared,
    account: Uuid,
    name: &str,
    cloud_id: &str,
    token: &str,
) -> Result<std::result::Result<(), Failed>> {
    let base = api(cloud_id);
    let me = match call(state, "GET", &format!("{base}/myself"), token, None).await {
        Ok(v) => match v["accountId"].as_str() {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Ok(Err(Failed(0, "Jira did not say who the token is".into()))),
        },
        Err(e) => return Ok(Err(e)),
    };
    let listed = match listed(state, &base, token).await {
        Ok(listed) => listed,
        Err(e) => return Ok(Err(e)),
    };
    let row: Option<(String, i64)> = sqlx::query_as(
        "select cloud_id, webhook_id from jira_hooks where account_id = $1 and tracker = $2",
    )
    .bind(account)
    .bind(name)
    .fetch_optional(&state.db)
    .await?;
    // On another site, the tracker was connected to this one since: that
    // one's is left to expire.
    let ours = row
        .as_ref()
        .filter(|(site, id)| site == cloud_id && listed.iter().any(|(l, _)| l == id))
        .map(|(_, id)| *id);

    // Left behind by a look that did not finish, or a row that went with
    // its tracker: each counts towards the five. One another hura account
    // keeps (the same person, connected twice) is that account's.
    let to = address(state);
    let candidates: Vec<i64> = listed
        .iter()
        .filter(|(id, url)| url == &to && Some(*id) != ours)
        .map(|(id, _)| *id)
        .collect();
    let kept: Vec<i64> = sqlx::query_scalar(
        "select webhook_id from jira_hooks where cloud_id = $1 and webhook_id = any($2)
           and not (account_id = $3 and tracker = $4)",
    )
    .bind(cloud_id)
    .bind(&candidates)
    .bind(account)
    .bind(name)
    .fetch_all(&state.db)
    .await?;
    let stale: Vec<i64> = candidates
        .into_iter()
        .filter(|id| !kept.contains(id))
        .collect();
    if !stale.is_empty() {
        let body = json!({ "webhookIds": stale });
        let _ = call(
            state,
            "DELETE",
            &format!("{base}/webhook"),
            token,
            Some(body),
        )
        .await;
    }

    let (id, expires) = match ours {
        Some(id) => {
            let body = json!({ "webhookIds": [id] });
            match call(
                state,
                "PUT",
                &format!("{base}/webhook/refresh"),
                token,
                Some(body),
            )
            .await
            {
                Ok(v) => (id, v["expirationDate"].as_i64()),
                Err(e) => return Ok(Err(e)),
            }
        }
        None => {
            let body = json!({
                "url": to,
                "webhooks": [{ "jqlFilter": "", "events": EVENTS }],
            });
            let made =
                match call(state, "POST", &format!("{base}/webhook"), token, Some(body)).await {
                    Ok(v) => v,
                    Err(e) => return Ok(Err(e)),
                };
            let result = &made["webhookRegistrationResult"][0];
            let Some(id) = result["createdWebhookId"].as_i64() else {
                let why = result["errors"]
                    .as_array()
                    .map(|e| {
                        e.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| "Jira did not say why".into());
                return Ok(Err(Failed(0, why)));
            };
            (id, None)
        }
    };
    let kept = sqlx::query(
        "insert into jira_hooks (account_id, tracker, cloud_id, webhook_id, jira_account, expires_at)
         values ($1, $2, $3, $4, $5,
                 coalesce(to_timestamp($6::bigint / 1000.0), now() + interval '30 days'))
         on conflict (account_id, tracker) do update
             set cloud_id = $3, webhook_id = $4, jira_account = $5, expires_at = excluded.expires_at
             where jira_hooks.webhook_id = $4 or $7",
    )
    .bind(account)
    .bind(name)
    .bind(cloud_id)
    .bind(id)
    .bind(&me)
    .bind(expires)
    // A new one replaces the row only when the row's is gone from Jira;
    // otherwise another look made one meanwhile, and this one goes.
    .bind(row.is_some() && ours.is_none())
    .execute(&state.db)
    .await?;
    if kept.rows_affected() == 0 {
        let body = json!({ "webhookIds": [id] });
        let _ = call(
            state,
            "DELETE",
            &format!("{base}/webhook"),
            token,
            Some(body),
        )
        .await;
    }
    Ok(Ok(()))
}

/// Every webhook the app has on the site for this token's person, with
/// where each sends.
async fn listed(
    state: &Shared,
    base: &str,
    token: &str,
) -> std::result::Result<Vec<(i64, String)>, Failed> {
    let mut out = Vec::new();
    loop {
        let url = format!("{base}/webhook?startAt={}&maxResults=100", out.len());
        let page = call(state, "GET", &url, token, None).await?;
        let values = page["values"].as_array().cloned().unwrap_or_default();
        out.extend(values.iter().filter_map(|w| {
            Some((
                w["id"].as_i64()?,
                w["url"].as_str().unwrap_or("").to_string(),
            ))
        }));
        if values.is_empty() || page["isLast"].as_bool() != Some(false) {
            return Ok(out);
        }
    }
}

/// Take a tracker's webhook off its site, as it is removed. One that cannot
/// be reached is left to expire.
pub async fn remove(state: &Shared, account: Uuid, name: &str) -> Result<()> {
    let row: Option<(String, i64)> = sqlx::query_as(
        "select cloud_id, webhook_id from jira_hooks where account_id = $1 and tracker = $2",
    )
    .bind(account)
    .bind(name)
    .fetch_optional(&state.db)
    .await?;
    let Some((cloud_id, id)) = row else {
        return Ok(());
    };
    let token = crate::trackers::stored(state, account)
        .await?
        .into_iter()
        .find(|t| t.source.name == name)
        .and_then(|t| t.token);
    if let Some(token) = token {
        let body = json!({ "webhookIds": [id] });
        let url = format!("{}/webhook", api(&cloud_id));
        let _ = call(state, "DELETE", &url, &token, Some(body)).await;
    }
    Ok(())
}

/// Look again at an account's connected Jira trackers whose last look is
/// old.
pub async fn refresh_due(state: &Shared, account: Uuid) -> Result<()> {
    let due: Vec<String> = sqlx::query_scalar(&format!(
        "select name from trackers
         where account_id = $1 and source->>'kind' = 'jira'
           and connection->>'cloud_id' is not null and token is not null
           and (webhooks_checked_at is null
                or webhooks_checked_at < now() - interval '{RECHECK}')"
    ))
    .bind(account)
    .fetch_all(&state.db)
    .await?;
    for name in due {
        sync(state, account, &name).await?;
    }
    Ok(())
}

/// Whether a delivery is about something of this person's: an issue
/// assigned to or reported by them, or one their last read held.
pub(crate) fn concerns(delivery: &Value, me: &str, seen: impl Fn(&str) -> bool) -> bool {
    let issue = &delivery["issue"];
    let fields = &issue["fields"];
    let people = [&fields["assignee"], &fields["reporter"]];
    if people.iter().any(|p| p["accountId"].as_str() == Some(me)) {
        return true;
    }
    // Assigned away from them: the changelog says who it was.
    let was_mine = delivery["changelog"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|i| i["field"].as_str() == Some("assignee") && i["from"].as_str() == Some(me));
    was_mine || issue["key"].as_str().is_some_and(seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delivery_concerns_whoever_the_issue_is_or_was_assigned_to() {
        let none = |_: &str| false;
        let assigned =
            json!({ "issue": { "key": "A-1", "fields": { "assignee": { "accountId": "me" } } } });
        assert!(concerns(&assigned, "me", none));
        assert!(!concerns(&assigned, "you", none));

        let reported = json!({ "issue": { "key": "A-1", "fields": { "assignee": null, "reporter": { "accountId": "me" } } } });
        assert!(concerns(&reported, "me", none));

        let taken = json!({ "issue": { "key": "A-1", "fields": { "assignee": { "accountId": "you" } } },
            "changelog": { "items": [{ "field": "assignee", "from": "me", "to": "you" }] } });
        assert!(concerns(&taken, "me", none));

        // A comment on an issue a filter holds.
        let comment =
            json!({ "issue": { "key": "B-7", "fields": { "summary": "x" } }, "comment": {} });
        assert!(!concerns(&comment, "me", none));
        assert!(concerns(&comment, "me", |key| key == "B-7"));
    }
}
