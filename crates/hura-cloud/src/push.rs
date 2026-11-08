//! Telling a phone, through the push relay (`docs/relay-api.md`).
//!
//! **The phone links this service as it links an `hurad`**: it asks the
//! relay for a push token, makes a key for the text, and hands both here with
//! `PUT /v1/push`. The service keeps them encrypted, sends `tracker_change`
//! events with the token, and encrypts every notification's text with the
//! key, so the relay and Apple see only that something changed.
//!
//! **Only to the service's own relay** (`HURA_CLOUD_RELAY`), whatever the
//! phone uses: an address a client chose would be one more way to make the
//! server send requests where it should not.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::TryRng as _;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::Shared;
use crate::auth::Signed;
use crate::error::{Error, Result};

pub const DEFAULT_RELAY: &str = "https://hura-relay.twadseth.workers.dev";

/// What became of one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    /// Handled: sent, a duplicate, or the phone is gone from the relay.
    Done,
    /// The token was refused: this route is finished until the phone links
    /// the service again.
    Refused(String),
    /// Worth trying again: the relay or APNs is busy or away.
    Later(String),
}

pub enum Relay {
    Https {
        base: String,
        agent: ureq::Agent,
    },
    /// Kept for the tests to read, answering with the status at the front of
    /// the queue (200 when empty).
    #[cfg(test)]
    Kept {
        sent: std::sync::Mutex<Vec<(String, Value)>>,
        statuses: std::sync::Mutex<Vec<u16>>,
    },
}

impl Relay {
    pub fn https(base: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .into();
        Relay::Https {
            base: base.trim_end_matches('/').to_string(),
            agent,
        }
    }

    /// Whether the relay takes `token`: what `hurad`'s `push-configure`
    /// checks before it keeps one.
    pub async fn check(&self, token: &str) -> std::result::Result<(), String> {
        match self.call("GET", "/v1/link", token, None).await {
            Ok(200) => Ok(()),
            Ok(401) | Ok(404) => Err("the relay does not know this push token".into()),
            Ok(status) => Err(format!("the relay answered {status}")),
            Err(e) => Err(e),
        }
    }

    pub async fn send(&self, token: &str, event: &Value) -> Sent {
        match self.call("POST", "/v1/events", token, Some(event)).await {
            Ok(200) | Ok(400) => Sent::Done,
            Ok(401) => Sent::Refused("the relay refused the push token".into()),
            Ok(status) => Sent::Later(format!("the relay answered {status}")),
            Err(e) => Sent::Later(e),
        }
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<&Value>,
    ) -> std::result::Result<u16, String> {
        match self {
            Relay::Https { base, agent } => {
                let (agent, url, token) =
                    (agent.clone(), format!("{base}{path}"), token.to_string());
                let (method, body) = (method.to_string(), body.map(Value::to_string));
                tokio::task::spawn_blocking(move || {
                    let mut request = ureq::http::Request::builder()
                        .method(method.as_str())
                        .uri(&url)
                        .header("Authorization", format!("Bearer {token}"));
                    if body.is_some() {
                        request = request.header("Content-Type", "application/json");
                    }
                    let request = request
                        .body(body.unwrap_or_default())
                        .map_err(|e| e.to_string())?;
                    agent
                        .run(request)
                        .map(|r| r.status().as_u16())
                        .map_err(|e| format!("could not reach the relay: {e}"))
                })
                .await
                .map_err(|e| e.to_string())?
            }
            #[cfg(test)]
            Relay::Kept { sent, statuses } => {
                if let Some(body) = body {
                    sent.lock().unwrap().push((token.to_string(), body.clone()));
                }
                let mut statuses = statuses.lock().unwrap();
                Ok(if statuses.is_empty() {
                    200
                } else {
                    statuses.remove(0)
                })
            }
        }
    }
}

/// A notification's text, as the phone decrypts it: what it shows, and the
/// ticket or pull request it opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub title: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

pub const KIND: &str = "tracker_change";

/// The `encrypted` field of an event: AES-256-GCM under the phone's key, bound
/// to the service's ID on the phone and the kind, as `docs/relay-api.md`
/// says. A body too long for the relay is halved until it fits.
pub fn encrypt(key: &[u8], server_id: &str, text: &Text) -> std::result::Result<String, String> {
    let key = UnboundKey::new(&AES_256_GCM, key)
        .map(LessSafeKey::new)
        .map_err(|_| "a push key is not 32 bytes")?;
    let aad = format!("hura-push-v1\n{server_id}\n{KIND}\n\n");
    let mut limit = 500;
    loop {
        let clipped = Text {
            title: clip(&text.title, 100),
            body: clip(&text.body, limit),
            ..text.clone()
        };
        let mut data = serde_json::to_vec(&clipped).map_err(|e| e.to_string())?;
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::SysRng
            .try_fill_bytes(&mut nonce)
            .map_err(|e| e.to_string())?;
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad.as_bytes()),
            &mut data,
        )
        .map_err(|_| "could not encrypt")?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&data);
        let encoded = URL_SAFE_NO_PAD.encode(out);
        if encoded.len() <= 3000 || limit < 20 {
            return Ok(encoded);
        }
        limit /= 2;
    }
}

/// At most `max` characters, the last an ellipsis when it was cut.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------- routes

#[derive(Deserialize)]
pub struct Route {
    server_id: String,
    push_token: String,
    /// 32 bytes, base64url.
    push_key: String,
}

#[derive(Serialize)]
pub struct RouteView {
    configured: bool,
    /// Why the relay stopped taking its token, once it has.
    broken: Option<String>,
}

/// `PUT /v1/push`: where to tell this device, replacing what was there. The
/// token is tried with the relay before it is kept.
pub async fn configure(
    State(state): State<Shared>,
    signed: Signed,
    Json(body): Json<Route>,
) -> Result<StatusCode> {
    let server_id = body.server_id.trim();
    let base32 = |c: char| c.is_ascii_lowercase() || ('2'..='7').contains(&c);
    if !(server_id.len() == 18 && server_id.starts_with("h_") && server_id[2..].chars().all(base32))
    {
        return Err(Error::bad("`server_id` is not one the relay would take"));
    }
    let key = URL_SAFE_NO_PAD
        .decode(body.push_key.trim())
        .ok()
        .filter(|k| k.len() == 32)
        .ok_or_else(|| Error::bad("`push_key` is not 32 bytes of base64url"))?;
    let token = body.push_token.trim();
    if !token.starts_with("hup_") {
        return Err(Error::bad("`push_token` is not a relay push token"));
    }
    state.relay.check(token).await.map_err(Error::bad)?;

    let sealer = state.keys.account(&state.db, signed.account).await?;
    sqlx::query(
        "insert into push_routes (device_id, account_id, server_id, push_token, push_key)
         values ($1, $2, $3, $4, $5)
         on conflict (device_id) do update set server_id = $3, push_token = $4, push_key = $5,
             broken = null, created_at = now()",
    )
    .bind(signed.device)
    .bind(signed.account)
    .bind(server_id)
    .bind(sealer.seal(&token_label(signed.device), token.as_bytes())?)
    .bind(sealer.seal(&key_label(signed.device), &key)?)
    .execute(&state.db)
    .await?;
    // The account's trackers are read from now on, starting soon: the first
    // read is the baseline.
    sqlx::query("insert into watches (account_id) values ($1) on conflict (account_id) do nothing")
        .bind(signed.account)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/push`: whether this device is told, and whether that still works.
pub async fn status(State(state): State<Shared>, signed: Signed) -> Result<Json<RouteView>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("select broken from push_routes where device_id = $1")
            .bind(signed.device)
            .fetch_optional(&state.db)
            .await?;
    Ok(Json(RouteView {
        configured: row.is_some(),
        broken: row.and_then(|(b,)| b),
    }))
}

/// `DELETE /v1/push`: stop telling this device.
pub async fn remove(State(state): State<Shared>, signed: Signed) -> Result<StatusCode> {
    sqlx::query("delete from push_routes where device_id = $1")
        .bind(signed.device)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// One device's route, opened.
pub struct Open {
    pub device: Uuid,
    pub server_id: String,
    pub token: String,
    pub key: Vec<u8>,
}

/// The routes of an account that still work, opened.
pub async fn routes(state: &Shared, account: Uuid) -> Result<Vec<Open>> {
    let rows: Vec<(Uuid, String, Vec<u8>, Vec<u8>)> = sqlx::query_as(
        "select device_id, server_id, push_token, push_key from push_routes
         where account_id = $1 and broken is null",
    )
    .bind(account)
    .fetch_all(&state.db)
    .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let sealer = state.keys.account(&state.db, account).await?;
    rows.into_iter()
        .map(|(device, server_id, token, key)| {
            Ok(Open {
                device,
                server_id,
                token: String::from_utf8(sealer.open(&token_label(device), &token)?)
                    .map_err(Error::internal)?,
                key: sealer.open(&key_label(device), &key)?,
            })
        })
        .collect()
}

pub async fn mark_broken(state: &Shared, device: Uuid, why: &str) -> Result<()> {
    sqlx::query("update push_routes set broken = $2 where device_id = $1")
        .bind(device)
        .bind(why)
        .execute(&state.db)
        .await?;
    Ok(())
}

fn token_label(device: Uuid) -> String {
    format!("push-token\0{device}")
}

fn key_label(device: Uuid) -> String {
    format!("push-key\0{device}")
}

/// An event for the relay.
pub fn event(event_id: &str, encrypted: String) -> Value {
    json!({ "event_id": event_id, "kind": KIND, "encrypted": encrypted })
}

#[cfg(test)]
pub(crate) fn decrypt(key: &[u8], server_id: &str, encrypted: &str) -> Text {
    let raw = URL_SAFE_NO_PAD.decode(encrypted).unwrap();
    let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).unwrap());
    let nonce = Nonce::try_assume_unique_for_key(&raw[..NONCE_LEN]).unwrap();
    let mut data = raw[NONCE_LEN..].to_vec();
    let aad = format!("hura-push-v1\n{server_id}\n{KIND}\n\n");
    let plain = key
        .open_in_place(nonce, Aad::from(aad.as_bytes()), &mut data)
        .unwrap();
    serde_json::from_slice(plain).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_encrypted_as_the_phone_opens_it() {
        let key = [3u8; 32];
        let text = Text {
            title: "P-1 · To Do → Done".into(),
            body: "Add it".into(),
            tracker: Some("work".into()),
            id: Some("10042".into()),
        };
        let sealed = encrypt(&key, "h_aaaaaaaaaaaaaaaa", &text).unwrap();
        assert_eq!(decrypt(&key, "h_aaaaaaaaaaaaaaaa", &sealed), text);
    }

    #[test]
    fn a_long_body_is_cut_until_it_fits() {
        let text = Text {
            title: "t".repeat(300),
            body: "b".repeat(5000),
            tracker: None,
            id: None,
        };
        let sealed = encrypt(&[1u8; 32], "h_aaaaaaaaaaaaaaaa", &text).unwrap();
        assert!(sealed.len() <= 3000);
        let opened = decrypt(&[1u8; 32], "h_aaaaaaaaaaaaaaaa", &sealed);
        assert_eq!(opened.title.chars().count(), 100);
        assert!(opened.title.ends_with('…'));
    }
}
