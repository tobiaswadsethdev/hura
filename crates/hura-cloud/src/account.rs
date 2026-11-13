//! An account's own data: all of it, to take away, and the account deleted
//! with everything under it.
//!
//! **The export is everything kept about the account** except what would be
//! a way into something: a tracker's token and refresh token, a device's
//! refresh token and a phone's push token are said to be there and never
//! shown. Everything else is as it is stored, what was last seen on the
//! trackers included.
//!
//! **Deleting is at once and for good.** Webhooks the service made on Jira
//! and Fizzy are taken off first, while their tokens are still here to do
//! it with; then the account goes, and with it (the schema cascades) every
//! device, tracker, token and route. Codes still out for its addresses go
//! too. Backups keep it until they are rotated out.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::Value;

use crate::Shared;
use crate::auth::Signed;
use crate::error::{Error, Result};

/// `GET /v1/me/export`: the account, as JSON.
pub async fn export(State(state): State<Shared>, signed: Signed) -> Result<Json<Value>> {
    let exported: Value = sqlx::query_scalar(
        "select json_build_object(
            'exported_at', now(),
            'account', (select json_build_object('id', id, 'plan', plan, 'created_at', created_at)
                        from accounts where id = $1),
            'sign_ins', coalesce((select json_agg(json_build_object(
                    'provider', provider, 'subject', subject, 'email', email,
                    'created_at', created_at) order by created_at)
                from sign_ins where account_id = $1), '[]'),
            'devices', coalesce((select json_agg(json_build_object(
                    'id', id, 'name', name, 'platform', platform,
                    'created_at', created_at, 'last_seen_at', last_seen_at) order by created_at)
                from devices where account_id = $1), '[]'),
            'notifications', coalesce((select json_agg(json_build_object(
                    'device', device_id, 'relay_server_id', server_id, 'push_token', 'kept, not shown',
                    'broken', broken, 'created_at', created_at) order by created_at)
                from push_routes where account_id = $1), '[]'),
            'trackers', coalesce((select json_agg(json_build_object(
                    'name', name, 'source', source, 'connection', connection,
                    'token', case when token is null then null else 'kept, not shown' end,
                    'webhooks', webhooks, 'created_at', created_at, 'updated_at', updated_at)
                    order by created_at)
                from trackers where account_id = $1), '[]'),
            'watch', (select json_build_object(
                    'last_read_at', last_read_at, 'next_read_at', next_read_at,
                    'failures', failures,
                    'seen', case when seen is null then null else convert_from(seen, 'UTF8')::json end)
                from watches where account_id = $1),
            'inbox_seen', (select json_build_object('items', items, 'seen_at', seen_at)
                from inbox_seen where account_id = $1)
        )",
    )
    .bind(signed.account)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(exported))
}

#[derive(Deserialize)]
pub struct Delete {
    /// `delete`, so a request sent by mistake does not take the account.
    confirm: String,
}

/// `DELETE /v1/me`: the account and everything under it, now.
pub async fn delete(
    State(state): State<Shared>,
    signed: Signed,
    Json(body): Json<Delete>,
) -> Result<StatusCode> {
    if body.confirm != "delete" {
        return Err(Error::bad("say `delete` to delete the account"));
    }
    let trackers: Vec<String> =
        sqlx::query_scalar("select name from trackers where account_id = $1")
            .bind(signed.account)
            .fetch_all(&state.db)
            .await?;
    for name in trackers {
        if let Err(e) = crate::fizzy_hooks::remove(&state, signed.account, &name).await {
            eprintln!("delete {}: {name}: fizzy webhooks: {e:?}", signed.account);
        }
        if let Err(e) = crate::jira_hooks::remove(&state, signed.account, &name).await {
            eprintln!("delete {}: {name}: jira webhook: {e:?}", signed.account);
        }
    }
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "delete from email_codes where email in
         (select email from sign_ins where account_id = $1 and email is not null)",
    )
    .bind(signed.account)
    .execute(&mut *tx)
    .await?;
    sqlx::query("delete from accounts where id = $1")
        .bind(signed.account)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    eprintln!("deleted account {}", signed.account);
    Ok(StatusCode::NO_CONTENT)
}
