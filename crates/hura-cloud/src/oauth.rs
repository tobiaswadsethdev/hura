//! Connecting a tracker through its own sign-in: GitHub (as a GitHub App,
//! with user tokens that act as you, so `@me` is still you) and Jira (with
//! Atlassian's OAuth 2.0). No token to paste, and one that renews itself.
//!
//! **A round trip through a browser.** A device asks `POST
//! /v1/connect/{provider}` for a URL, the person signs in and agrees there,
//! and the provider sends the browser back to the callback here with a code
//! and the state this made. The code is swapped for tokens, the tokens kept
//! on a tracker of the account's (one already there for the same site or
//! account is connected in place, keeping its filters), and the browser sent
//! back to the app, or told to go back to it.
//!
//! **Renewed before it runs out.** A GitHub user token lasts eight hours and
//! an Atlassian one one; before a tracker is read with one that has less
//! than five minutes left, it is refreshed. Atlassian's refresh tokens change
//! at every use, so a refresh holds the tracker's row while it runs: two at
//! once would leave one holding a token that no longer works. One whose
//! refresh is refused loses its token, and asks to be connected again.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use hura_core::tracker::{Kind, Source};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use sqlx::types::Json as Jsonb;
use uuid::Uuid;

use crate::Shared;
use crate::auth::Signed;
use crate::error::{Error, Result};
use crate::integrations::{self, escape, layout};
use crate::token;

const GITHUB_TOKEN: &str = "https://github.com/login/oauth/access_token";
const JIRA_TOKEN: &str = "https://auth.atlassian.com/oauth/token";
/// No `read:me`: who the token belongs to is never kept, so the service holds
/// no Atlassian personal data (and Atlassian's personal data reporting does
/// not apply). Jira's own `/myself` answers it for each read, in memory.
const JIRA_SCOPES: &str = "read:jira-work write:jira-work read:jira-user offline_access";
/// How long before it runs out an access token is renewed.
const EARLY: i64 = 5 * 60;

/// `GET /v1/connect`: which providers the service can connect.
pub async fn available(State(state): State<Shared>, _signed: Signed) -> Result<Json<Value>> {
    Ok(Json(json!({
        "github": integrations::github(&state).await?.is_some(),
        "jira": integrations::jira(&state).await?.is_some(),
    })))
}

#[derive(Deserialize)]
pub struct Start {
    /// `app`, to be sent back to the app with `hura://cloud/connected`, or
    /// `browser`, to be told to go back to it.
    #[serde(rename = "return", default)]
    return_to: String,
}

/// `POST /v1/connect/{provider}`: where to send the browser to connect it.
pub async fn start(
    State(state): State<Shared>,
    signed: Signed,
    Path(provider): Path<String>,
    Json(body): Json<Start>,
) -> Result<Json<Value>> {
    let return_to = if body.return_to == "app" {
        "app"
    } else {
        "browser"
    };
    let made = remember(&state.db, signed.account, &provider, return_to).await?;
    let url = match provider.as_str() {
        "github" => {
            let app = integrations::github(&state).await?.ok_or_else(not_set_up)?;
            format!(
                "https://github.com/login/oauth/authorize?client_id={}&state={made}&redirect_uri={}",
                encode(&app.client_id),
                encode(&format!("{}/v1/oauth/github/callback", state.url))
            )
        }
        "jira" => {
            let app = integrations::jira(&state).await?.ok_or_else(not_set_up)?;
            format!(
                "https://auth.atlassian.com/authorize?audience=api.atlassian.com&client_id={}&scope={}&redirect_uri={}&state={made}&response_type=code&prompt=consent",
                encode(&app.client_id),
                encode(JIRA_SCOPES),
                encode(&format!("{}/v1/oauth/jira/callback", state.url))
            )
        }
        _ => return Err(Error::not_found("no such provider")),
    };
    Ok(Json(json!({ "url": url })))
}

fn not_set_up() -> Error {
    Error::new(
        StatusCode::CONFLICT,
        "hura cloud is not set up to connect that yet",
    )
}

/// A state for a round trip, kept as its hash for ten minutes.
async fn remember(
    db: &sqlx::PgPool,
    account: Uuid,
    provider: &str,
    return_to: &str,
) -> Result<String> {
    let made = token::mint("")?;
    sqlx::query(
        "insert into oauth_states (state_hash, account_id, provider, return_to, expires_at)
         values ($1, $2, $3, $4, now() + interval '10 minutes')",
    )
    .bind(token::hash(&made))
    .bind(account)
    .bind(provider)
    .bind(return_to)
    .execute(db)
    .await?;
    Ok(made)
}

/// The account and where to send the browser, for a state that came back:
/// used up, so it cannot come back twice.
async fn recall(db: &sqlx::PgPool, made: &str, provider: &str) -> Result<(Uuid, String)> {
    sqlx::query_as(
        "delete from oauth_states
         where state_hash = $1 and provider = $2 and expires_at > now()
         returning account_id, return_to",
    )
    .bind(token::hash(made))
    .bind(provider)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| Error::bad("this sign-in has expired; start connecting again from the app"))
}

#[derive(Deserialize)]
pub struct Callback {
    #[serde(default)]
    code: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    setup_action: String,
    #[serde(default)]
    error_description: String,
}

/// `GET /v1/oauth/github/callback`: back from GitHub, either from signing in
/// (a code) or from installing the app on repositories (a setup action).
pub async fn github_callback(
    State(state): State<Shared>,
    Query(q): Query<Callback>,
) -> Result<Response> {
    if !q.error_description.is_empty() {
        return Ok(done(&state, "browser", "GitHub", Err(&q.error_description)));
    }
    if q.code.is_empty() && !q.setup_action.is_empty() {
        // Installed or reconfigured. With the state of a connection that sent
        // the person to install, back to where it began.
        let return_to = match recall(&state.db, &q.state, "github-install").await {
            Ok((_, r)) => r,
            Err(_) => "browser".into(),
        };
        return Ok(done(&state, &return_to, "GitHub", Ok(())));
    }
    let (account, return_to) = recall(&state.db, &q.state, "github").await?;
    let app = integrations::github(&state).await?.ok_or_else(not_set_up)?;
    let form = format!(
        "client_id={}&client_secret={}&code={}&redirect_uri={}",
        encode(&app.client_id),
        encode(&app.client_secret),
        encode(&q.code),
        encode(&format!("{}/v1/oauth/github/callback", state.url))
    );
    let tokens = exchange(
        &state,
        GITHUB_TOKEN,
        form,
        "application/x-www-form-urlencoded",
    )
    .await?;

    let (status, text) = state
        .web
        .send(
            "GET",
            "https://api.github.com/user",
            &[
                ("Authorization", format!("Bearer {}", tokens.access_token)),
                ("Accept", "application/vnd.github+json".into()),
            ],
            None,
        )
        .await
        .map_err(Error::internal)?;
    let login = (status == 200)
        .then(|| serde_json::from_str::<Value>(&text).ok())
        .flatten()
        .and_then(|v| v["login"].as_str().map(String::from))
        .ok_or_else(|| Error::internal(format!("GitHub's user answered {status}")))?;

    let mut tx = state.db.begin().await?;
    let existing: Option<Jsonb<Source>> = sqlx::query_scalar(
        "select source from trackers where account_id = $1
         and (connection->>'provider' = 'github' and lower(connection->>'login') = lower($2)
              or (connection is null and source->>'kind' = 'git-hub'))
         order by connection is null, created_at limit 1",
    )
    .bind(account)
    .bind(&login)
    .fetch_optional(&mut *tx)
    .await?;
    // One already there, pasted or connected before, is connected in place
    // and keeps what it reads.
    let source = match existing {
        Some(Jsonb(source)) => source,
        None => Source {
            kind: Kind::GitHub,
            name: free_name(
                &mut tx,
                account,
                &["github".into(), format!("github-{login}")],
            )
            .await?,
            ..Source::default()
        },
    };
    let connection =
        json!({ "provider": "github", "login": login, "expires_at": tokens.expires_at() });
    keep(&state, &mut tx, account, &source, &tokens, connection).await?;
    tx.commit().await?;

    // A user token reads only where the app is installed: with nowhere yet,
    // install it before going back.
    let (status, text) = state
        .web
        .send(
            "GET",
            "https://api.github.com/user/installations",
            &[
                ("Authorization", format!("Bearer {}", tokens.access_token)),
                ("Accept", "application/vnd.github+json".into()),
            ],
            None,
        )
        .await
        .map_err(Error::internal)?;
    let installed = status == 200
        && serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["total_count"].as_u64())
            .is_some_and(|n| n > 0);
    if !installed {
        let next = remember(&state.db, account, "github-install", &return_to).await?;
        let to = format!("{}/installations/new?state={next}", app.html_url);
        return Ok((StatusCode::FOUND, [(header::LOCATION, to)]).into_response());
    }
    Ok(done(&state, &return_to, "GitHub", Ok(())))
}

/// `GET /v1/oauth/jira/callback`: back from Atlassian with a code. Every Jira
/// site the person agreed to becomes a tracker, or connects the one already
/// there for it.
pub async fn jira_callback(
    State(state): State<Shared>,
    Query(q): Query<Callback>,
) -> Result<Response> {
    if !q.error_description.is_empty() {
        return Ok(done(&state, "browser", "Jira", Err(&q.error_description)));
    }
    let (account, return_to) = recall(&state.db, &q.state, "jira").await?;
    let app = integrations::jira(&state).await?.ok_or_else(not_set_up)?;
    let body = json!({
        "grant_type": "authorization_code",
        "client_id": app.client_id,
        "client_secret": app.client_secret,
        "code": q.code,
        "redirect_uri": format!("{}/v1/oauth/jira/callback", state.url),
    });
    let tokens = exchange(&state, JIRA_TOKEN, body.to_string(), "application/json").await?;
    let bearer = [
        ("Authorization", format!("Bearer {}", tokens.access_token)),
        ("Accept", "application/json".into()),
    ];

    let (status, text) = state
        .web
        .send(
            "GET",
            "https://api.atlassian.com/oauth/token/accessible-resources",
            &bearer,
            None,
        )
        .await
        .map_err(Error::internal)?;
    let sites: Vec<Value> = (status == 200)
        .then(|| serde_json::from_str(&text).ok())
        .flatten()
        .ok_or_else(|| Error::internal(format!("Atlassian's resources answered {status}")))?;
    if sites.is_empty() {
        return Ok(done(
            &state,
            &return_to,
            "Jira",
            Err("no Jira site was shared with hura"),
        ));
    }
    let mut tx = state.db.begin().await?;
    for site in &sites {
        let (Some(cloud_id), Some(url)) = (site["id"].as_str(), site["url"].as_str()) else {
            continue;
        };
        let url = url.trim_end_matches('/');
        let existing: Option<(String, Jsonb<Source>)> = sqlx::query_as(
            "select name, source from trackers where account_id = $1
             and (connection->>'cloud_id' = $2
                  or (connection is null and source->>'kind' = 'jira'
                      and rtrim(source->>'site', '/') = $3))
             order by connection is null, created_at limit 1",
        )
        .bind(account)
        .bind(cloud_id)
        .bind(url)
        .fetch_optional(&mut *tx)
        .await?;
        let source = match existing {
            // Its filters and the rest stay; it is only connected in place.
            Some((_, Jsonb(mut source))) => {
                source.email = None;
                source
            }
            None => {
                let short = url
                    .trim_start_matches("https://")
                    .split('.')
                    .next()
                    .unwrap_or("jira")
                    .to_string();
                Source {
                    kind: Kind::Jira,
                    name: free_name(
                        &mut tx,
                        account,
                        &["jira".into(), short.clone(), format!("jira-{short}")],
                    )
                    .await?,
                    site: Some(url.to_string()),
                    ..Source::default()
                }
            }
        };
        // Labelled with its site, not whoever connected it: nothing about the
        // person is kept.
        let host = url.trim_start_matches("https://");
        let connection = json!({
            "provider": "jira", "login": host, "cloud_id": cloud_id, "expires_at": tokens.expires_at(),
        });
        keep(&state, &mut tx, account, &source, &tokens, connection).await?;
    }
    tx.commit().await?;
    Ok(done(&state, &return_to, "Jira", Ok(())))
}

/// The first of `names` the account has no tracker called, or the last with a
/// number on it.
async fn free_name(db: &mut PgConnection, account: Uuid, names: &[String]) -> Result<String> {
    let taken: Vec<String> = sqlx::query_scalar("select name from trackers where account_id = $1")
        .bind(account)
        .fetch_all(&mut *db)
        .await?;
    if let Some(name) = names.iter().find(|n| !taken.contains(n)) {
        return Ok(name.clone());
    }
    let last = names.last().cloned().unwrap_or_else(|| "tracker".into());
    Ok((2..)
        .map(|i| format!("{last}-{i}"))
        .find(|n| !taken.contains(n))
        .unwrap_or(last))
}

/// A tracker connected with these tokens, added or connected in place.
async fn keep(
    state: &Shared,
    db: &mut PgConnection,
    account: Uuid,
    source: &Source,
    tokens: &Tokens,
    connection: Value,
) -> Result<()> {
    let key = state.keys.account(&state.db, account).await?;
    let access = key.seal_token(&source.name, &tokens.access_token)?;
    let refresh = match &tokens.refresh_token {
        Some(r) => Some(key.seal(&refresh_label(&source.name), r.as_bytes())?),
        None => None,
    };
    sqlx::query(
        "insert into trackers (account_id, name, source, token, connection, refresh_token)
         values ($1, $2, $3, $4, $5, $6)
         on conflict (account_id, name) do update set
             source = $3, token = $4, connection = $5, refresh_token = $6, updated_at = now()",
    )
    .bind(account)
    .bind(&source.name)
    .bind(Jsonb(source))
    .bind(access)
    .bind(Jsonb(connection))
    .bind(refresh)
    .execute(&mut *db)
    .await?;
    Ok(())
}

pub(crate) fn refresh_label(tracker: &str) -> String {
    format!("tracker-refresh\0{tracker}")
}

struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

impl Tokens {
    /// Unix seconds, or null for a token that does not run out.
    fn expires_at(&self) -> Option<i64> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();
        self.expires_in.map(|s| now + s)
    }
}

/// A code or a refresh token, for tokens.
async fn exchange(state: &Shared, url: &str, body: String, kind: &str) -> Result<Tokens> {
    let (status, text) = state
        .web
        .send(
            "POST",
            url,
            &[
                ("Content-Type", kind.into()),
                ("Accept", "application/json".into()),
            ],
            Some(body),
        )
        .await
        .map_err(Error::internal)?;
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    match v["access_token"].as_str() {
        Some(access) if status == 200 => Ok(Tokens {
            access_token: access.to_string(),
            refresh_token: v["refresh_token"].as_str().map(String::from),
            expires_in: v["expires_in"].as_i64(),
        }),
        _ => Err(Error::new(
            StatusCode::BAD_GATEWAY,
            format!(
                "the sign-in was not accepted: {}",
                v["error_description"]
                    .as_str()
                    .or(v["error"].as_str())
                    .unwrap_or("no reason given")
            ),
        )),
    }
}

/// The end of a round trip: back into the app, or a page saying to go back.
fn done(
    state: &Shared,
    return_to: &str,
    what: &str,
    result: std::result::Result<(), &str>,
) -> Response {
    let _ = state;
    if return_to == "app" {
        let to = match result {
            Ok(()) => format!("hura://cloud/connected?provider={}", what.to_lowercase()),
            Err(e) => format!("hura://cloud/connected?error={}", encode(e)),
        };
        return (StatusCode::FOUND, [(header::LOCATION, to)]).into_response();
    }
    let body = match result {
        Ok(()) => {
            format!("<p>{what} is connected. Go back to hura: its trackers now include it.</p>")
        }
        Err(e) => format!(
            "<p>{what} was not connected: {}</p><p>Go back to hura and try again.</p>",
            escape(e)
        ),
    };
    Html(layout(&format!("{what} connected"), &body)).into_response()
}

/// Renews the tokens of the account's connected trackers that run out soon.
/// Each holds its row while it renews; one refused loses its token.
pub async fn refresh_due(state: &Shared, account: Uuid) -> Result<()> {
    let due: Vec<String> = sqlx::query_scalar(
        "select name from trackers where account_id = $1 and connection is not null
         and refresh_token is not null
         and (connection->>'expires_at')::bigint < extract(epoch from now())::bigint + $2",
    )
    .bind(account)
    .bind(EARLY)
    .fetch_all(&state.db)
    .await?;
    for name in due {
        if let Err(e) = refresh(state, account, &name).await {
            eprintln!("refresh {account} {name}: {e:?}");
        }
    }
    Ok(())
}

async fn refresh(state: &Shared, account: Uuid, name: &str) -> Result<()> {
    let mut tx = state.db.begin().await?;
    let row: Option<(Jsonb<Value>, Option<Vec<u8>>)> = sqlx::query_as(
        "select connection, refresh_token from trackers
         where account_id = $1 and name = $2 for update",
    )
    .bind(account)
    .bind(name)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((Jsonb(mut connection), Some(sealed))) = row else {
        return Ok(());
    };
    // Renewed by someone else while this waited for the row.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    if connection["expires_at"]
        .as_i64()
        .is_some_and(|at| at > now + EARLY)
    {
        return Ok(());
    }
    let key = state.keys.account(&state.db, account).await?;
    let refresh_token =
        String::from_utf8(key.open(&refresh_label(name), &sealed)?).map_err(Error::internal)?;
    let renewed = match connection["provider"].as_str() {
        Some("github") => {
            let app = integrations::github(state).await?.ok_or_else(not_set_up)?;
            let form = format!(
                "client_id={}&client_secret={}&grant_type=refresh_token&refresh_token={}",
                encode(&app.client_id),
                encode(&app.client_secret),
                encode(&refresh_token)
            );
            exchange(
                state,
                GITHUB_TOKEN,
                form,
                "application/x-www-form-urlencoded",
            )
            .await
        }
        Some("jira") => {
            let app = integrations::jira(state).await?.ok_or_else(not_set_up)?;
            let body = json!({
                "grant_type": "refresh_token",
                "client_id": app.client_id,
                "client_secret": app.client_secret,
                "refresh_token": refresh_token,
            });
            exchange(state, JIRA_TOKEN, body.to_string(), "application/json").await
        }
        _ => return Ok(()),
    };
    match renewed {
        Ok(tokens) => {
            connection["expires_at"] = json!(tokens.expires_at());
            let refresh = match &tokens.refresh_token {
                Some(r) => key.seal(&refresh_label(name), r.as_bytes())?,
                None => sealed,
            };
            sqlx::query(
                "update trackers set token = $3, refresh_token = $4, connection = $5, updated_at = now()
                 where account_id = $1 and name = $2",
            )
            .bind(account)
            .bind(name)
            .bind(key.seal_token(name, &tokens.access_token)?)
            .bind(refresh)
            .bind(Jsonb(&connection))
            .execute(&mut *tx)
            .await?;
        }
        Err(e) => {
            // Refused: connecting again is the only way back.
            eprintln!("refresh {account} {name}: {e:?}");
            sqlx::query(
                "update trackers set token = null, refresh_token = null, updated_at = now()
                 where account_id = $1 and name = $2",
            )
            .bind(account)
            .bind(name)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Percent-encoding for a query string.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
