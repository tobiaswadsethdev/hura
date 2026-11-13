//! Signing in with Apple through a browser, for the desktop.
//!
//! **A round trip, back to the computer it began on.** The app waits on a
//! port of 127.0.0.1 and asks `POST /v1/sign-in/apple/start` for Apple's
//! address, with a secret of its own. The person signs in at Apple, which
//! posts an identity token back here (for the Services ID, with the nonce
//! this made); this checks it as the iOS app's is checked, and sends the
//! browser on to the app's port with a code. The app shows the code and its
//! secret to `POST /v1/sign-in/apple/finish` and is signed in.
//!
//! **Why the loopback, rather than the app asking until it is done**: then a
//! link to Apple that somebody else started, signed in to by you, would sign
//! *their* computer in to your account. Here the code only ever reaches the
//! computer whose browser signed in, and is no use without the secret only
//! the app that started has.
//!
//! No key of Apple's is needed: the token comes in the browser's post, so
//! there is no code to swap with Apple for one.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::{Form, Json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::Shared;
use crate::apple::{self, Identity};
use crate::error::{Error, Result};
use crate::integrations::layout;
use crate::oauth::encode;
use crate::sign_in::{self, NewDevice, Session};
use crate::token;

const AUTHORIZE: &str = "https://appleid.apple.com/auth/authorize";

#[derive(Deserialize)]
pub struct Start {
    /// The app's own, shown again to finish. Kept as its hash.
    secret: String,
    /// Where on 127.0.0.1 the app waits for the browser.
    port: u16,
    device: NewDevice,
}

/// `POST /v1/sign-in/apple/start`: where to send the browser.
pub async fn start(State(state): State<Shared>, Json(body): Json<Start>) -> Result<Json<Value>> {
    let services_id = state.apple.services_id.as_deref().ok_or_else(|| {
        Error::new(
            StatusCode::CONFLICT,
            "hura cloud is not set up to sign in with Apple from a browser yet",
        )
    })?;
    let (name, platform) = sign_in::device_named(&body.device)?;
    if body.secret.len() < 32 || body.port < 1024 {
        return Err(Error::bad("a browser sign-in needs a secret and a port"));
    }
    let made = token::mint("")?;
    let nonce = token::mint("")?;
    let mut tx = state.db.begin().await?;
    sqlx::query("delete from apple_web_sign_ins where expires_at < now()")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "insert into apple_web_sign_ins
         (state_hash, secret_hash, nonce, port, device_name, platform, expires_at)
         values ($1, $2, $3, $4, $5, $6, now() + interval '10 minutes')",
    )
    .bind(token::hash(&made))
    .bind(token::hash(&body.secret))
    .bind(&nonce)
    .bind(i32::from(body.port))
    .bind(name)
    .bind(platform)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    // Apple is given the nonce's hash, which it puts in the token as it is:
    // the same as the iOS app does, so one check serves both.
    let hashed: String = Sha256::digest(nonce.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let url = format!(
        "{AUTHORIZE}?client_id={}&redirect_uri={}&response_type={}&response_mode=form_post&scope={}&state={made}&nonce={hashed}",
        encode(services_id),
        encode(&callback(&state)),
        encode("code id_token"),
        encode("name email"),
    );
    Ok(Json(json!({ "url": url })))
}

fn callback(state: &Shared) -> String {
    format!("{}/v1/oauth/apple/callback", state.url)
}

/// What Apple posts back. `user`, the name and address the first time, is
/// not signed and not read: the token's verified address is the one kept.
#[derive(Deserialize)]
pub struct Back {
    #[serde(default)]
    state: String,
    #[serde(default)]
    id_token: String,
    #[serde(default)]
    error: String,
}

/// `POST /v1/oauth/apple/callback`: back from Apple, in the browser.
pub async fn back(State(state): State<Shared>, Form(form): Form<Back>) -> Result<Response> {
    let row: Option<(i32, String)> = sqlx::query_as(
        "select port, nonce from apple_web_sign_ins
         where state_hash = $1 and code_hash is null and expires_at > now()",
    )
    .bind(token::hash(&form.state))
    .fetch_optional(&state.db)
    .await?;
    let Some((port, nonce)) = row else {
        return Ok(Html(layout(
            "Sign in with Apple",
            "<p>This sign-in has expired, or was already used. Go back to hura and start again.</p>",
        ))
        .into_response());
    };
    let to_app = |query: String| {
        (
            StatusCode::SEE_OTHER,
            [(
                header::LOCATION,
                format!("http://127.0.0.1:{port}/apple?{query}"),
            )],
        )
            .into_response()
    };
    let failed = |why: &str| to_app(format!("error={}", encode(why)));

    if !form.error.is_empty() {
        let why = if form.error == "user_cancelled_authorize" {
            "cancelled"
        } else {
            "Apple did not sign you in"
        };
        return Ok(failed(why));
    }
    let services_id = state.apple.services_id.as_deref().unwrap_or_default();
    let who: Identity = match state
        .apple
        .verify(&form.id_token, &nonce, services_id)
        .await
    {
        Ok(who) => who,
        Err(_) => return Ok(failed("Apple's sign-in could not be checked; try again")),
    };
    let code = token::mint("")?;
    let kept = sqlx::query(
        "update apple_web_sign_ins set subject = $2, email = $3, code_hash = $4
         where state_hash = $1 and code_hash is null and expires_at > now()",
    )
    .bind(token::hash(&form.state))
    .bind(&who.subject)
    .bind(&who.email)
    .bind(token::hash(&code))
    .execute(&state.db)
    .await?;
    if kept.rows_affected() == 0 {
        return Ok(failed("this sign-in was already used; start again"));
    }
    Ok(to_app(format!("code={}", encode(&code))))
}

#[derive(Deserialize)]
pub struct Finish {
    secret: String,
    code: String,
}

/// A device signed in through a browser, and the address Apple gave for it,
/// if any, to say who it is signed in as.
#[derive(Serialize)]
pub struct Finished {
    #[serde(flatten)]
    session: Session,
    email: Option<String>,
}

/// `POST /v1/sign-in/apple/finish`: the code the browser brought back, and
/// the secret the app started with, for a signed-in device. Once.
pub async fn finish(
    State(state): State<Shared>,
    Json(body): Json<Finish>,
) -> Result<Json<Finished>> {
    let mut tx = state.db.begin().await?;
    let row: Option<(String, Option<String>, String, String)> = sqlx::query_as(
        "delete from apple_web_sign_ins
         where code_hash = $1 and secret_hash = $2 and subject is not null and expires_at > now()
         returning subject, email, device_name, platform",
    )
    .bind(token::hash(&body.code))
    .bind(token::hash(&body.secret))
    .fetch_optional(&mut *tx)
    .await?;
    let (subject, email, name, platform) = row.ok_or_else(apple::refused)?;
    let who = Identity { subject, email };
    let account = apple::account_for(&mut tx, &who).await?;
    let session = sign_in::start(&mut tx, account, &name, &platform).await?;
    tx.commit().await?;
    Ok(Json(Finished {
        session,
        email: who.email,
    }))
}
