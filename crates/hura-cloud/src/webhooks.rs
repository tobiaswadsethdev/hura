//! GitHub's webhooks: a tracker read the moment something on it changes,
//! rather than at its next turn.
//!
//! **A delivery only says when to read.** Its signature is checked against
//! the GitHub App's webhook secret, and every GitHub login in it (the
//! author, the assignees, the reviewers asked, whoever acted) is matched
//! against the accounts with GitHub connected as that login. Those accounts
//! are made due at once, and the watcher reads them within its next pass,
//! with the same comparison as any read: what a delivery says is never
//! trusted to say what changed.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use ring::hmac;
use serde_json::Value;

use crate::Shared;
use crate::error::{Error, Result};
use crate::integrations;

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
