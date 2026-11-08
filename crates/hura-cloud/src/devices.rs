//! The account, and the devices signed in to it: listed and signed out from
//! any of them.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Serialize;
use uuid::Uuid;

use crate::Shared;
use crate::auth::Signed;
use crate::error::{Error, Result};

#[derive(Serialize)]
pub struct Me {
    account: Uuid,
    plan: String,
    sign_ins: Vec<SignIn>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct SignIn {
    /// `email` or `apple`.
    provider: String,
    email: Option<String>,
}

/// `GET /v1/me`: the account and its ways in.
pub async fn me(State(state): State<Shared>, signed: Signed) -> Result<Json<Me>> {
    let plan: String = sqlx::query_scalar("select plan from accounts where id = $1")
        .bind(signed.account)
        .fetch_one(&state.db)
        .await?;
    let sign_ins = sqlx::query_as(
        "select provider, email from sign_ins where account_id = $1 order by created_at",
    )
    .bind(signed.account)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(Me {
        account: signed.account,
        plan,
        sign_ins,
    }))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Device {
    id: Uuid,
    name: String,
    platform: String,
    /// Unix seconds.
    created_at: i64,
    /// When it last swapped its refresh token, in Unix seconds: roughly when
    /// it was last used.
    last_seen_at: i64,
    /// The device asking.
    current: bool,
}

/// `GET /v1/devices`: every device signed in to the account, newest first.
pub async fn list(State(state): State<Shared>, signed: Signed) -> Result<Json<Vec<Device>>> {
    let devices = sqlx::query_as(
        "select id, name, platform,
                extract(epoch from created_at)::bigint as created_at,
                extract(epoch from last_seen_at)::bigint as last_seen_at,
                id = $2 as current
         from devices where account_id = $1 order by devices.created_at desc",
    )
    .bind(signed.account)
    .bind(signed.device)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(devices))
}

/// `DELETE /v1/devices/{id}`: sign a device out. Its tokens stop working at
/// once.
pub async fn revoke(
    State(state): State<Shared>,
    signed: Signed,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    let gone = sqlx::query("delete from devices where id = $1 and account_id = $2")
        .bind(id)
        .bind(signed.account)
        .execute(&state.db)
        .await?;
    if gone.rows_affected() == 0 {
        return Err(Error::not_found("no such device on this account"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/sign-out`: sign out the device asking.
pub async fn sign_out(State(state): State<Shared>, signed: Signed) -> Result<StatusCode> {
    sqlx::query("delete from devices where id = $1")
        .bind(signed.device)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
