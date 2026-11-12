//! GitHub's, Fizzy's and Jira's webhooks: a tracker read the moment something on it
//! changes, rather than at its next turn.
//!
//! **A delivery only says when to read.** Its signature is checked against
//! the GitHub App's webhook secret, and every GitHub login in it (the
//! author, the assignees, the reviewers asked, whoever acted) is matched
//! against the accounts with GitHub connected as that login. Those accounts
//! are made due at once, and the watcher reads them within its next pass,
//! with the same comparison as any read: what a delivery says is never
//! trusted to say what changed.
//!
//! **A Fizzy delivery names its webhook in its address** (see
//! [`crate::fizzy_hooks`]), which says whose it is; its signature is checked
//! against that webhook's own secret, and that account is made due.
//!
//! **A Jira delivery carries a token** signed with the OAuth app's client
//! secret, and names the webhooks it matched (see [`crate::jira_hooks`]):
//! each is one account's, made due when the issue is theirs.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hura_core::tracker::changes::Seen;
use ring::hmac;
use serde_json::Value;

use crate::Shared;
use crate::error::{Error, Result};
use crate::integrations;
use uuid::Uuid;

/// `POST /v1/webhooks/github`
pub async fn github(
    State(state): State<Shared>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode> {
    let Some(app) = integrations::github(&state).await? else {
        return Ok(StatusCode::NOT_FOUND);
    };
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("sha256="))
        .and_then(hex)
        .ok_or_else(Error::unauthorized)?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, app.webhook_secret.as_bytes());
    hmac::verify(&key, &body, &signature).map_err(|_| Error::unauthorized())?;

    let payload: Value = serde_json::from_slice(&body).map_err(|_| Error::bad("not JSON"))?;
    let mut logins = Vec::new();
    collect(&payload, &mut logins);
    logins.sort();
    logins.dedup();
    if logins.is_empty() {
        return Ok(StatusCode::ACCEPTED);
    }
    sqlx::query(
        "update watches set next_read_at = now()
         where account_id in (
             select account_id from trackers
             where connection->>'provider' = 'github' and lower(connection->>'login') = any($1))",
    )
    .bind(&logins)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::ACCEPTED)
}

/// `POST /v1/webhooks/fizzy/{id}`
pub async fn fizzy(
    State(state): State<Shared>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode> {
    let hook: Option<(Uuid, Vec<u8>)> =
        sqlx::query_as("select account_id, secret from fizzy_hooks where id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    // Gone with its tracker: Fizzy turns off a webhook that keeps failing.
    let Some((account, sealed)) = hook else {
        return Ok(StatusCode::NOT_FOUND);
    };
    // Fizzy's is the bare hex, with no `sha256=` in front.
    let signature = headers
        .get("x-webhook-signature")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| hex(v.trim()))
        .ok_or_else(Error::unauthorized)?;
    let secret = state
        .keys
        .account(&state.db, account)
        .await?
        .open(&crate::fizzy_hooks::label(id), &sealed)?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, &secret);
    hmac::verify(&key, &body, &signature).map_err(|_| Error::unauthorized())?;
    sqlx::query("update watches set next_read_at = now() where account_id = $1")
        .bind(account)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::ACCEPTED)
}

/// `POST /v1/webhooks/jira`
pub async fn jira(
    State(state): State<Shared>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode> {
    let Some(app) = integrations::jira(&state).await? else {
        return Ok(StatusCode::NOT_FOUND);
    };
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(Error::unauthorized)?;
    if !signed_by(token.trim(), app.client_secret.as_bytes(), now()) {
        return Err(Error::unauthorized());
    }
    let delivery: Value = serde_json::from_slice(&body).map_err(|_| Error::bad("not JSON"))?;
    let matched: Vec<i64> = delivery["matchedWebhookIds"]
        .as_array()
        .map(|ids| ids.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    // Only a 200 is taken as delivered: anything else is sent again.
    if matched.is_empty() {
        return Ok(StatusCode::OK);
    }
    /// Its account and tracker, who connected it, what the account last
    /// saw, and whether the tracker has filters.
    type Row = (Uuid, String, String, Option<Vec<u8>>, bool);
    let rows: Vec<Row> = sqlx::query_as(
        "select h.account_id, h.tracker, h.jira_account, w.seen,
                jsonb_array_length(coalesce(t.source->'jql_filters', '[]'::jsonb)) > 0
         from jira_hooks h
         join trackers t on t.account_id = h.account_id and t.name = h.tracker
         left join watches w on w.account_id = h.account_id
         where h.webhook_id = any($1)",
    )
    .bind(&matched)
    .fetch_all(&state.db)
    .await?;
    // A deleted issue is in no filter: if it was yours, the last read had it.
    let key = delivery["issue"]["key"]
        .as_str()
        .filter(|_| delivery["webhookEvent"].as_str() != Some("jira:issue_deleted"))
        .filter(|k| crate::jira_filters::is_key(k));
    let mut due = Vec::new();
    let mut ask: (Vec<Uuid>, Vec<String>) = Default::default();
    for (account, tracker, me, seen, filters) in rows {
        let seen: Option<Seen> = seen.as_deref().and_then(|s| serde_json::from_slice(s).ok());
        if crate::jira_hooks::concerns(&delivery, &me, |key| {
            seen.as_ref().is_some_and(|s| s.has_ticket(&tracker, key))
        }) {
            due.push(account);
        } else if filters && key.is_some() {
            // Perhaps one of its filters holds it now: asked of Jira shortly.
            ask.0.push(account);
            ask.1.push(tracker);
        }
    }
    if !due.is_empty() {
        sqlx::query("update watches set next_read_at = now() where account_id = any($1)")
            .bind(&due)
            .execute(&state.db)
            .await?;
    }
    if let Some(key) = key
        && !ask.0.is_empty()
    {
        sqlx::query(
            "insert into jira_checks (account_id, tracker, issue_key)
             select account, tracker, $3 from unnest($1::uuid[], $2::text[]) as a(account, tracker)
             on conflict do nothing",
        )
        .bind(&ask.0)
        .bind(&ask.1)
        .bind(key)
        .execute(&state.db)
        .await?;
    }
    Ok(StatusCode::OK)
}

/// Whether a token is a JWT signed with `secret` (HS256) that has not run
/// out: what Atlassian puts on a delivery to an OAuth app.
pub(crate) fn signed_by(token: &str, secret: &[u8], now: i64) -> bool {
    let mut parts = token.split('.');
    let (Some(head), Some(claims), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let decode = |s: &str| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok();
    let json = |s: &str| decode(s).and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let (Some(head), Some(claims), Some(signature)) = (json(head), json(claims), decode(signature))
    else {
        return false;
    };
    if head["alg"].as_str() != Some("HS256") {
        return false;
    }
    let signed = &token[..token.rfind('.').unwrap_or_default()];
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    if hmac::verify(&key, signed.as_bytes(), &signature).is_err() {
        return false;
    }
    // A minute's grace for the clocks.
    claims["exp"].as_i64().is_none_or(|exp| exp + 60 >= now)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// Every `login` anywhere in a delivery, lowercased: each kind of event puts
/// the people it is about in a different place, and this is all of them.
fn collect(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, v) in map {
                if k == "login"
                    && let Some(s) = v.as_str()
                {
                    out.push(s.to_lowercase());
                }
                collect(v, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect(v, out)),
        _ => {}
    }
}

fn hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(alg: &str, claims: Value, secret: &[u8]) -> String {
        let part = |v: Value| URL_SAFE_NO_PAD.encode(v.to_string());
        let signed = format!(
            "{}.{}",
            part(serde_json::json!({ "alg": alg, "typ": "JWT" })),
            part(claims)
        );
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
        let tag = hmac::sign(&key, signed.as_bytes());
        format!("{signed}.{}", URL_SAFE_NO_PAD.encode(tag.as_ref()))
    }

    #[test]
    fn a_jira_delivery_is_signed_with_the_client_secret() {
        let claims = serde_json::json!({ "iss": "atlassian", "exp": 1_000 });
        assert!(signed_by(
            &jwt("HS256", claims.clone(), b"secret"),
            b"secret",
            900
        ));
        assert!(!signed_by(
            &jwt("HS256", claims.clone(), b"other"),
            b"secret",
            900
        ));
        assert!(
            !signed_by(&jwt("HS256", claims.clone(), b"secret"), b"secret", 2_000),
            "run out"
        );
        assert!(!signed_by(&jwt("none", claims, b"secret"), b"secret", 900));
        assert!(!signed_by("not.a.token", b"secret", 900));
        assert!(!signed_by("", b"secret", 900));
    }
}
