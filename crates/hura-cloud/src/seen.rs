//! "Seen", shared between the account's devices: looking at the inbox on one
//! clears the badge and the tracker notifications on the others.
//!
//! **What was seen** is what was waiting on you (tickets assigned to you,
//! pull requests asking for your review) when you last looked at the inbox,
//! as the keys the apps count their badge by, `tracker/id`, and when. The
//! last device to look sets it.
//!
//! **The other phones are told quietly**: a background push through the
//! relay (`inbox_seen`), with nothing to show, which wakes the app to ask
//! for this and clear the notifications it showed before then. iOS spaces
//! background pushes out as it sees fit, so a phone may only catch up when
//! it is next opened, which asks too.

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Shared;
use crate::auth::Signed;
use crate::error::{Error, Result};
use crate::push::{self, Sent};
use crate::token;

const MAX_ITEMS: usize = 1000;

/// `GET /v1/seen`: `{items, at}`, `at` in Unix seconds or null before the
/// inbox has been looked at anywhere.
pub async fn get(State(state): State<Shared>, signed: Signed) -> Result<Json<Value>> {
    let row: Option<(Vec<String>, i64)> = sqlx::query_as(
        "select items, extract(epoch from seen_at)::bigint from inbox_seen where account_id = $1",
    )
    .bind(signed.account)
    .fetch_optional(&state.db)
    .await?;
    Ok(Json(match row {
        Some((items, at)) => json!({ "items": items, "at": at }),
        None => json!({ "items": [], "at": null }),
    }))
}

#[derive(Deserialize)]
pub struct Looked {
    items: Vec<String>,
}

/// `PUT /v1/seen` `{items}`: the inbox was just looked at on this device,
/// with these waiting. When that is not what was seen already, the
/// account's other phones are told.
pub async fn put(
    State(state): State<Shared>,
    signed: Signed,
    Json(body): Json<Looked>,
) -> Result<Json<Value>> {
    let mut items: Vec<String> = body
        .items
        .into_iter()
        .map(|i| i.trim().to_string())
        .filter(|i| !i.is_empty())
        .collect();
    if items.len() > MAX_ITEMS || items.iter().any(|i| i.len() > 300 || i.contains('\0')) {
        return Err(Error::bad(
            "that is not a list of tickets and pull requests",
        ));
    }
    items.sort();
    items.dedup();

    let before: Option<Vec<String>> =
        sqlx::query_scalar("select items from inbox_seen where account_id = $1")
            .bind(signed.account)
            .fetch_optional(&state.db)
            .await?;
    if before.as_ref() == Some(&items) {
        return get(State(state), signed).await;
    }
    sqlx::query(
        "insert into inbox_seen (account_id, items, seen_at, device_id) values ($1, $2, now(), $3)
         on conflict (account_id) do update set items = $2, seen_at = now(), device_id = $3",
    )
    .bind(signed.account)
    .bind(&items)
    .bind(signed.device)
    .execute(&state.db)
    .await?;

    // Best effort: a phone that misses this asks when it is next opened.
    for route in push::routes(&state, signed.account).await? {
        if route.device == signed.device {
            continue;
        }
        let event =
            json!({ "event_id": format!("seen-{}", token::mint("")?), "kind": "inbox_seen" });
        if let Sent::Refused(why) = state.relay.send(&route.token, &event).await {
            push::mark_broken(&state, route.device, &why).await?;
        }
    }
    get(State(state), signed).await
}
