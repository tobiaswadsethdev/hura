//! Signing in with an emailed code: no password to keep, and no Apple device
//! needed.
//!
//! Asking for a code always answers the same, whether or not the address has
//! an account, so the endpoint does not say who has one. A code lasts ten
//! minutes, is used once, and is spent after five wrong tries; an address is
//! sent one a minute and five an hour at most. Codes, used or not, are
//! deleted after a day.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use subtle::ConstantTimeEq as _;
use uuid::Uuid;

use crate::Shared;
use crate::auth::{self, Access};
use crate::error::{Error, Result};
use crate::token::{self, ACCESS_SECONDS, REFRESH};

const CODE_SECONDS: i32 = 10 * 60;
const MAX_ATTEMPTS: i32 = 5;
const PER_MINUTE: i64 = 1;
const PER_HOUR: i64 = 5;

#[derive(Deserialize)]
pub struct Ask {
    email: String,
}

/// `POST /v1/sign-in/email`: send a code.
pub async fn send_code(State(state): State<Shared>, Json(body): Json<Ask>) -> Result<StatusCode> {
    let email = address(&body.email)?;
    // One request for an address at a time, so two at once cannot both find
    // the limit unreached: held until the code is written.
    // Codes are kept a day, for the limits above to count, and no longer.
    sqlx::query("delete from email_codes where created_at < now() - interval '1 day'")
        .execute(&state.db)
        .await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("select pg_advisory_xact_lock(hashtext($1))")
        .bind(&email)
        .execute(&mut *tx)
        .await?;
    let (minute, hour): (i64, i64) = sqlx::query_as(
        "select count(*) filter (where created_at > now() - interval '1 minute'),
                count(*) filter (where created_at > now() - interval '1 hour')
         from email_codes where email = $1",
    )
    .bind(&email)
    .fetch_one(&mut *tx)
    .await?;
    if minute >= PER_MINUTE || hour >= PER_HOUR {
        return Err(Error::new(
            StatusCode::TOO_MANY_REQUESTS,
            "a code was sent a moment ago: use that one, or try again in a minute",
        ));
    }

    let code = format!("{:06}", token::number(1_000_000)?);
    sqlx::query(
        "insert into email_codes (email, code_hash, expires_at)
         values ($1, $2, now() + $3 * interval '1 second')",
    )
    .bind(&email)
    .bind(code_hash(&email, &code))
    .bind(CODE_SECONDS)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    state
        .mail
        .send_code(&email, &code)
        .await
        .map_err(Error::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The device signing in, as its owner will see it in the list.
#[derive(Deserialize)]
pub struct NewDevice {
    name: String,
    /// `ios`, `macos`, `windows`, `linux`.
    platform: String,
}

#[derive(Deserialize)]
pub struct Verify {
    email: String,
    code: String,
    device: NewDevice,
}

/// A device just signed in: the refresh token it keeps, and its first access
/// token.
#[derive(Serialize)]
pub struct Session {
    account: Uuid,
    device: Uuid,
    refresh_token: String,
    #[serde(flatten)]
    access: Access,
}

/// `POST /v1/sign-in/email/verify`: a code for a signed-in device, into the
/// address's account, made if it has none.
pub async fn verify(
    State(state): State<Shared>,
    Json(body): Json<Verify>,
) -> Result<Json<Session>> {
    let email = address(&body.email)?;
    let (name, platform) = device_named(&body.device)?;
    let code: String = body.code.chars().filter(|c| !c.is_whitespace()).collect();

    let mut tx = state.db.begin().await?;
    let row: Option<(Uuid, Vec<u8>, i32)> = sqlx::query_as(
        "select id, code_hash, attempts from email_codes
         where email = $1 and used_at is null and expires_at > now()
         order by created_at desc limit 1 for update",
    )
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await?;
    let spent = || Error::bad("that code has been used or has expired: ask for a new one");
    let (id, hash, attempts) = row.ok_or_else(spent)?;
    if attempts >= MAX_ATTEMPTS {
        return Err(spent());
    }
    if !bool::from(hash.ct_eq(&code_hash(&email, &code))) {
        sqlx::query("update email_codes set attempts = attempts + 1 where id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(Error::bad("that code is not the one we sent"));
    }
    // Every code still out for the address goes with this one: whoever has
    // the inbox has signed in.
    sqlx::query("update email_codes set used_at = now() where email = $1 and used_at is null")
        .bind(&email)
        .execute(&mut *tx)
        .await?;

    let account = account_for(&mut tx, &email).await?;
    let session = start(&mut tx, account, name, platform).await?;
    tx.commit().await?;
    Ok(Json(session))
}

/// A device for a name and platform someone typed, or why not.
pub(crate) fn device_named(device: &NewDevice) -> Result<(&str, &str)> {
    let name = device.name.trim();
    let platform = device.platform.trim();
    if name.is_empty() || name.chars().count() > 100 || platform.is_empty() || platform.len() > 20 {
        return Err(Error::bad("a device needs a name and a platform"));
    }
    Ok((name, platform))
}

/// A device signed in to `account`, however it got in: its refresh token and
/// its first access token.
pub(crate) async fn start(
    db: &mut PgConnection,
    account: Uuid,
    name: &str,
    platform: &str,
) -> Result<Session> {
    let refresh_token = token::mint(REFRESH)?;
    let device: Uuid = sqlx::query_scalar(
        "insert into devices (account_id, name, platform, refresh_hash)
         values ($1, $2, $3, $4) returning id",
    )
    .bind(account)
    .bind(name)
    .bind(platform)
    .bind(token::hash(&refresh_token))
    .fetch_one(&mut *db)
    .await?;
    let access_token = auth::issue(db, device).await?;
    Ok(Session {
        account,
        device,
        refresh_token,
        access: Access {
            access_token,
            expires_in: ACCESS_SECONDS,
        },
    })
}

/// The account a verified address signs in to: the one it signed in to
/// before; or one that another way in (an Apple ID) verified the same address
/// for, which this joins; or a new one.
async fn account_for(db: &mut PgConnection, email: &str) -> Result<Uuid> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        "select account_id from sign_ins where provider = 'email' and subject = $1",
    )
    .bind(email)
    .fetch_optional(&mut *db)
    .await?;
    if let Some(account) = existing {
        return Ok(account);
    }
    let linked: Option<Uuid> = sqlx::query_scalar(
        "select account_id from sign_ins where email = $1 order by created_at limit 1",
    )
    .bind(email)
    .fetch_optional(&mut *db)
    .await?;
    let account = match linked {
        Some(account) => account,
        None => {
            sqlx::query_scalar("insert into accounts default values returning id")
                .fetch_one(&mut *db)
                .await?
        }
    };
    sqlx::query(
        "insert into sign_ins (account_id, provider, subject, email) values ($1, 'email', $2, $2)",
    )
    .bind(account)
    .bind(email)
    .execute(&mut *db)
    .await?;
    Ok(account)
}

/// An address as it is kept: trimmed and lowercased, or why it is not one.
fn address(raw: &str) -> Result<String> {
    let email = raw.trim().to_lowercase();
    let ok = email.len() <= 254
        && !email.contains(char::is_whitespace)
        && email.split_once('@').is_some_and(|(user, domain)| {
            !user.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        });
    if ok {
        Ok(email)
    } else {
        Err(Error::bad("that is not an email address"))
    }
}

/// A code is hashed with its address, so the same code sent to two people is
/// two different rows.
fn code_hash(email: &str, code: &str) -> Vec<u8> {
    token::hash(&format!("{email}\0{code}"))
}
