//! Who is asking: an access token in `Authorization: Bearer`, and swapping a
//! refresh token for a new one.

use axum::Json;
use axum::extract::{FromRequestParts, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::Shared;
use crate::error::{Error, Result};
use crate::token::{self, ACCESS, ACCESS_SECONDS};

/// A request from a signed-in device.
pub struct Signed {
    pub account: Uuid,
    pub device: Uuid,
}

impl FromRequestParts<Shared> for Signed {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &Shared) -> Result<Self> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(Error::unauthorized)?;
        let row: Option<(Uuid, Uuid)> = sqlx::query_as(
            "select d.account_id, d.id from access_tokens t join devices d on d.id = t.device_id
             where t.token_hash = $1 and t.expires_at > now()",
        )
        .bind(token::hash(token.trim()))
        .fetch_optional(&state.db)
        .await?;
        let (account, device) = row.ok_or_else(Error::unauthorized)?;
        Ok(Signed { account, device })
    }
}

/// A new access token for a device, and its own expired ones cleared away.
pub async fn issue(db: &mut PgConnection, device: Uuid) -> Result<String> {
    sqlx::query("delete from access_tokens where device_id = $1 and expires_at <= now()")
        .bind(device)
        .execute(&mut *db)
        .await?;
    let access = token::mint(ACCESS)?;
    sqlx::query(
        "insert into access_tokens (token_hash, device_id, expires_at)
         values ($1, $2, now() + $3 * interval '1 second')",
    )
    .bind(token::hash(&access))
    .bind(device)
    .bind(ACCESS_SECONDS)
    .execute(&mut *db)
    .await?;
    Ok(access)
}

#[derive(Deserialize)]
pub struct Refresh {
    refresh_token: String,
}

#[derive(Serialize)]
pub struct Access {
    pub access_token: String,
    /// Seconds.
    pub expires_in: i32,
}

/// `POST /v1/token`: a refresh token for an access token. One that is not
/// known answers 401, which a device reads as having been signed out.
pub async fn refresh(
    State(state): State<Shared>,
    Json(body): Json<Refresh>,
) -> Result<Json<Access>> {
    let mut tx = state.db.begin().await?;
    let device: Option<Uuid> = sqlx::query_scalar(
        "update devices set last_seen_at = now() where refresh_hash = $1 returning id",
    )
    .bind(token::hash(body.refresh_token.trim()))
    .fetch_optional(&mut *tx)
    .await?;
    let device = device.ok_or_else(Error::unauthorized)?;
    let access_token = issue(&mut tx, device).await?;
    tx.commit().await?;
    Ok(Json(Access {
        access_token,
        expires_in: ACCESS_SECONDS,
    }))
}
