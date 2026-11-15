//! Signing in with Apple, from the iOS app (here) and from a browser for
//! the desktop ([`crate::apple_web`]).
//!
//! **The app does the signing in**, with AuthenticationServices, and hands
//! over the identity token Apple gave it: a JWT that Apple signed, saying
//! which Apple ID (`sub`), for which app (`aud`, the bundle ID), and the
//! address if the person shared it. This checks the signature against
//! Apple's published keys and the claims against this service, and signs a
//! device in. No key of Apple's is needed for that, nor for a browser's
//! token, which Apple posts back with the Services ID as its `aud`.
//!
//! **A nonce ties the token to this sign-in**: the app makes one, gives
//! Apple its SHA-256, and sends the nonce itself here, so a token seen
//! elsewhere cannot be replayed.
//!
//! **Which account**: the one this Apple ID signed in to before; or the one a
//! verified address of its already belongs to, which this joins; or a new
//! one. An address Apple relays ("hide my email") matches nothing, and is
//! kept only so the account can say where it came from.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::State;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::PgConnection;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::Shared;
use crate::error::{Error, Result};
use crate::sign_in::{self, NewDevice, Session};

pub const ISSUER: &str = "https://appleid.apple.com";
const KEYS: &str = "https://appleid.apple.com/auth/keys";
/// How long Apple's keys are used before they are fetched again. A token
/// signed with a key not among them fetches them at once, so a new key of
/// Apple's is picked up without waiting.
const KEYS_FOR: Duration = Duration::from_secs(60 * 60);
/// But not more often than this, whatever key a token names: a token is not
/// checked until its key is found, so anybody could otherwise make every
/// request a fetch from Apple, and every sign-in wait behind it.
const KEYS_AT_MOST_EVERY: Duration = Duration::from_secs(60);

/// Apple's published keys, and the app the tokens must be for.
pub struct Apple {
    /// The bundle ID: what `aud` must be for the iOS app's tokens.
    pub audience: String,
    /// The Services ID: what it must be for a browser's, and without one a
    /// browser cannot sign in.
    pub services_id: Option<String>,
    keys: Keys,
}

enum Keys {
    Fetched {
        agent: ureq::Agent,
        cache: Mutex<Option<(Instant, Vec<Jwk>)>>,
    },
    #[cfg(test)]
    Fixed(Vec<Jwk>),
}

#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kid: String,
    n: String,
    e: String,
}

impl Apple {
    pub fn new(audience: String, services_id: Option<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .into();
        Apple {
            audience,
            services_id,
            keys: Keys::Fetched {
                agent,
                cache: Mutex::new(None),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn fixed(audience: &str, kid: &str, n: &str) -> Self {
        Apple {
            audience: audience.into(),
            services_id: Some(format!("{audience}.signin")),
            keys: Keys::Fixed(vec![Jwk {
                kid: kid.into(),
                n: n.into(),
                e: "AQAB".into(),
            }]),
        }
    }

    /// Apple's key by its ID: from the cache while it is fresh and holds it,
    /// otherwise fetched.
    async fn key(&self, kid: &str) -> Result<Jwk> {
        match &self.keys {
            #[cfg(test)]
            Keys::Fixed(keys) => keys
                .iter()
                .find(|k| k.kid == kid)
                .cloned()
                .ok_or_else(refused),
            Keys::Fetched { agent, cache } => {
                let mut cache = cache.lock().await;
                if let Some((at, keys)) = cache.as_ref() {
                    let known = keys.iter().find(|k| k.kid == kid);
                    if at.elapsed() < KEYS_FOR
                        && let Some(k) = known
                    {
                        return Ok(k.clone());
                    }
                    if at.elapsed() < KEYS_AT_MOST_EVERY {
                        return known.cloned().ok_or_else(refused);
                    }
                }
                let agent = agent.clone();
                let text = tokio::task::spawn_blocking(move || {
                    agent
                        .get(KEYS)
                        .call()
                        .and_then(|mut r| r.body_mut().read_to_string())
                })
                .await
                .map_err(Error::internal)?
                .map_err(|e| Error::internal(format!("Apple's keys: {e}")))?;
                #[derive(Deserialize)]
                struct Set {
                    keys: Vec<Jwk>,
                }
                let set: Set = serde_json::from_str(&text).map_err(Error::internal)?;
                let found = set.keys.iter().find(|k| k.kid == kid).cloned();
                *cache = Some((Instant::now(), set.keys));
                found.ok_or_else(refused)
            }
        }
    }

    /// The Apple ID a token for `audience` is for, and its address when Apple
    /// says it is verified; or why the token is not one to sign in with.
    pub(crate) async fn verify(
        &self,
        token: &str,
        nonce: &str,
        audience: &str,
    ) -> Result<Identity> {
        let mut parts = token.trim().split('.');
        let (Some(head), Some(body), Some(signature), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(refused());
        };
        let decode = |s: &str| URL_SAFE_NO_PAD.decode(s).map_err(|_| refused());
        let header: Value = serde_json::from_slice(&decode(head)?).map_err(|_| refused())?;
        if header["alg"] != "RS256" {
            return Err(refused());
        }
        let kid = header["kid"].as_str().ok_or_else(refused)?;
        let key = self.key(kid).await?;
        RsaPublicKeyComponents {
            n: decode(&key.n)?,
            e: decode(&key.e)?,
        }
        .verify(
            &RSA_PKCS1_2048_8192_SHA256,
            format!("{head}.{body}").as_bytes(),
            &decode(signature)?,
        )
        .map_err(|_| refused())?;

        let claims: Claims = serde_json::from_slice(&decode(body)?).map_err(|_| refused())?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let expected_nonce: String = Sha256::digest(nonce.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if claims.iss != ISSUER
            || claims.aud != audience
            || claims.exp < now
            || claims.nonce.as_deref() != Some(expected_nonce.as_str())
            || claims.sub.is_empty()
        {
            return Err(refused());
        }
        let verified = matches!(&claims.email_verified, Some(Value::Bool(true)))
            || matches!(&claims.email_verified, Some(Value::String(s)) if s == "true");
        Ok(Identity {
            subject: claims.sub,
            email: claims
                .email
                .filter(|_| verified)
                .map(|e| e.trim().to_lowercase()),
        })
    }
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    exp: u64,
    sub: String,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    email: Option<String>,
    /// A boolean, or the string `"true"`, as Apple has sent both.
    #[serde(default)]
    email_verified: Option<Value>,
}

pub(crate) struct Identity {
    pub subject: String,
    pub email: Option<String>,
}

pub(crate) fn refused() -> Error {
    Error::new(
        axum::http::StatusCode::UNAUTHORIZED,
        "Apple's sign-in could not be checked; try again",
    )
}

#[derive(Deserialize)]
pub struct AppleSignIn {
    identity_token: String,
    nonce: String,
    device: NewDevice,
}

/// `POST /v1/sign-in/apple`: an identity token from Sign in with Apple for a
/// signed-in device.
pub async fn sign_in(
    State(state): State<Shared>,
    Json(body): Json<AppleSignIn>,
) -> Result<Json<Session>> {
    let (name, platform) = sign_in::device_named(&body.device)?;
    let who = state
        .apple
        .verify(&body.identity_token, &body.nonce, &state.apple.audience)
        .await?;
    let mut tx = state.db.begin().await?;
    let account = account_for(&mut tx, &who).await?;
    let session = sign_in::start(&mut tx, account, name, platform).await?;
    tx.commit().await?;
    Ok(Json(session))
}

/// The account an Apple ID signs in to: the one it signed in to before; one
/// whose verified address it shares, which it joins; or a new one.
pub(crate) async fn account_for(db: &mut PgConnection, who: &Identity) -> Result<Uuid> {
    let before: Option<Uuid> = sqlx::query_scalar(
        "select account_id from sign_ins where provider = 'apple' and subject = $1",
    )
    .bind(&who.subject)
    .fetch_optional(&mut *db)
    .await?;
    if let Some(account) = before {
        return Ok(account);
    }
    let shared = match who.email.as_deref() {
        // A relayed address is made for this app, and matches nobody's.
        Some(email) if !email.ends_with("@privaterelay.appleid.com") => {
            sqlx::query_scalar(
                "select account_id from sign_ins where email = $1 order by created_at limit 1",
            )
            .bind(email)
            .fetch_optional(&mut *db)
            .await?
        }
        _ => None,
    };
    let account = match shared {
        Some(account) => account,
        None => {
            sqlx::query_scalar("insert into accounts default values returning id")
                .fetch_one(&mut *db)
                .await?
        }
    };
    sqlx::query(
        "insert into sign_ins (account_id, provider, subject, email) values ($1, 'apple', $2, $3)",
    )
    .bind(account)
    .bind(&who.subject)
    .bind(&who.email)
    .execute(&mut *db)
    .await?;
    Ok(account)
}
