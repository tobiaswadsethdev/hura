//! The trackers an account keeps, and reading them.
//!
//! **Kept as the desktop keeps them** (`hura_client::trackers`): the same
//! checks on adding one, a name that is never changed, a token that is
//! stored but never shown. Each change answers with the whole list, as
//! [`Configured`], which says whether there is a token and never what it is.
//!
//! **Read with hura-core's tracker requests** (`hura_core::tracker::
//! request`), at `POST /v1/tracker`: where a request takes a tracker, it
//! names one of the account's, and `inbox` reads all of them. The answer is
//! `{"ok": ...}` or `{"error": ...}`.

use axum::Json;
use axum::extract::{Path, State};
use hura_core::tracker::{Configured, Source, Stored};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::types::Json as Jsonb;
use uuid::Uuid;

use crate::Shared;
use crate::auth::Signed;
use crate::error::{Error, Result};
use crate::plans;

#[derive(Deserialize)]
pub struct Add {
    source: Source,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Deserialize)]
pub struct Edit {
    source: Source,
}

#[derive(Deserialize)]
pub struct Token {
    token: String,
}

/// `GET /v1/trackers`
pub async fn list(State(state): State<Shared>, signed: Signed) -> Result<Json<Vec<Configured>>> {
    views(&state, signed.account).await
}

/// `POST /v1/trackers`: add one. Refused when it could not work, or when its
/// name is taken.
pub async fn add(
    State(state): State<Shared>,
    signed: Signed,
    Json(body): Json<Add>,
) -> Result<Json<Vec<Configured>>> {
    let source = checked(&body.source)?;
    let token = body
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let sealed = match token {
        Some(token) => Some(
            state
                .keys
                .account(&state.db, signed.account)
                .await?
                .seal_token(&source.name, token)?,
        ),
        None => None,
    };
    let mut tx = state.db.begin().await?;
    let before = plans::usage(&mut tx, signed.account).await?;
    let added = sqlx::query(
        "insert into trackers (account_id, name, source, token) values ($1, $2, $3, $4)
         on conflict (account_id, name) do nothing",
    )
    .bind(signed.account)
    .bind(&source.name)
    .bind(Jsonb(&source))
    .bind(sealed)
    .execute(&mut *tx)
    .await?;
    if added.rows_affected() == 0 {
        return Err(Error::new(
            axum::http::StatusCode::CONFLICT,
            format!("there is already a tracker called `{}`", source.name),
        ));
    }
    plans::allowed(&before, &plans::usage(&mut tx, signed.account).await?)?;
    tx.commit().await?;
    hooks(&state, signed.account, &source.name).await;
    views(&state, signed.account).await
}

/// `PUT /v1/trackers/{name}`: change one, keeping its token. Not its name,
/// which sessions started from its tickets record.
pub async fn edit(
    State(state): State<Shared>,
    signed: Signed,
    Path(name): Path<String>,
    Json(body): Json<Edit>,
) -> Result<Json<Vec<Configured>>> {
    // A Jira tracker connected through Atlassian has no email, which only
    // its connection makes right: checked as it is read.
    let cloud_id: Option<String> = sqlx::query_scalar(
        "select connection->>'cloud_id' from trackers where account_id = $1 and name = $2",
    )
    .bind(signed.account)
    .bind(&name)
    .fetch_optional(&state.db)
    .await?
    .flatten();
    let mut source = body.source.clone();
    source.cloud_id = cloud_id;
    let mut source = checked(&source)?;
    source.cloud_id = None;
    if source.name != name {
        return Err(Error::bad(format!(
            "a tracker cannot be renamed; `{name}` is still `{name}`"
        )));
    }
    let mut tx = state.db.begin().await?;
    let before = plans::usage(&mut tx, signed.account).await?;
    let changed = sqlx::query(
        "update trackers set source = $3, updated_at = now() where account_id = $1 and name = $2",
    )
    .bind(signed.account)
    .bind(&name)
    .bind(Jsonb(&source))
    .execute(&mut *tx)
    .await?;
    found(changed.rows_affected(), &name)?;
    plans::allowed(&before, &plans::usage(&mut tx, signed.account).await?)?;
    tx.commit().await?;
    hooks(&state, signed.account, &name).await;
    views(&state, signed.account).await
}

/// `PUT /v1/trackers/{name}/token`: store or replace one's token.
pub async fn set_token(
    State(state): State<Shared>,
    signed: Signed,
    Path(name): Path<String>,
    Json(body): Json<Token>,
) -> Result<Json<Vec<Configured>>> {
    let token = body.token.trim();
    if token.is_empty() {
        return Err(Error::bad("that token is empty"));
    }
    let connected: Option<bool> = sqlx::query_scalar(
        "select connection is not null from trackers where account_id = $1 and name = $2",
    )
    .bind(signed.account)
    .bind(&name)
    .fetch_optional(&state.db)
    .await?;
    if connected == Some(true) {
        return Err(Error::bad(format!(
            "`{name}` is connected through its own sign-in; connect it again to replace it"
        )));
    }
    let key = state.keys.account(&state.db, signed.account).await?;
    let changed = sqlx::query(
        "update trackers set token = $3, updated_at = now() where account_id = $1 and name = $2",
    )
    .bind(signed.account)
    .bind(&name)
    .bind(key.seal_token(&name, token)?)
    .execute(&state.db)
    .await?;
    found(changed.rows_affected(), &name)?;
    hooks(&state, signed.account, &name).await;
    views(&state, signed.account).await
}

/// `DELETE /v1/trackers/{name}`: take one away, token and all.
pub async fn forget(
    State(state): State<Shared>,
    signed: Signed,
    Path(name): Path<String>,
) -> Result<Json<Vec<Configured>>> {
    // Off its boards or its site first, while its token is still here to do
    // it with.
    if let Err(e) = crate::fizzy_hooks::remove(&state, signed.account, &name).await {
        eprintln!("forget {name}: fizzy webhooks: {e:?}");
    }
    if let Err(e) = crate::jira_hooks::remove(&state, signed.account, &name).await {
        eprintln!("forget {name}: jira webhook: {e:?}");
    }
    let gone = sqlx::query("delete from trackers where account_id = $1 and name = $2")
        .bind(signed.account)
        .bind(&name)
        .execute(&state.db)
        .await?;
    found(gone.rows_affected(), &name)?;
    views(&state, signed.account).await
}

/// `POST /v1/tracker`: one of hura-core's tracker requests, on the account's
/// trackers.
pub async fn call(
    State(state): State<Shared>,
    signed: Signed,
    Json(mut request): Json<Value>,
) -> Result<Json<Value>> {
    let answer = match prepare(&state, signed.account, &mut request).await? {
        Err(e) => Err(e),
        Ok(()) => {
            tokio::task::spawn_blocking(move || hura_core::tracker::request::handle(&request))
                .await
                .map_err(Error::internal)?
        }
    };
    Ok(Json(match answer {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "error": e }),
    }))
}

/// Puts the account's trackers where the request names them: the one called
/// `tracker`, and for `inbox`, all of them. A request that names a tracker it
/// does not have is answered as one that failed, like any other.
async fn prepare(
    state: &Shared,
    account: Uuid,
    request: &mut Value,
) -> Result<std::result::Result<(), String>> {
    let Some(fields) = request.as_object_mut() else {
        return Ok(Err("a request is an object with an `op`".into()));
    };
    let inbox = fields.get("op").and_then(Value::as_str) == Some("inbox");
    let named = match fields.get("tracker") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name.clone()),
        // A whole tracker, token and all, is what the phone sends its own
        // copy of hura-core. Here it would make the service a way to send any
        // token anywhere.
        Some(_) => return Ok(Err("`tracker` is the name of one of your trackers".into())),
    };
    if named.is_none() && !inbox {
        return Ok(Ok(()));
    }
    let trackers = stored(state, account).await?;
    if let Some(name) = named {
        match trackers.iter().find(|t| t.source.name == name) {
            Some(t) => fields.insert(
                "tracker".into(),
                serde_json::to_value(t).map_err(Error::internal)?,
            ),
            None => return Ok(Err(format!("no tracker called `{name}`"))),
        };
    }
    if inbox {
        fields.insert(
            "trackers".into(),
            serde_json::to_value(&trackers).map_err(Error::internal)?,
        );
    }
    Ok(Ok(()))
}

/// The account's trackers, tokens opened: any that run out soon renewed
/// first, and a Jira tracker connected through Atlassian given its cloud ID.
pub(crate) async fn stored(state: &Shared, account: Uuid) -> Result<Vec<Stored>> {
    crate::oauth::refresh_due(state, account).await?;
    /// Its name, source, sealed token and Atlassian cloud ID.
    type Row = (String, Jsonb<Source>, Option<Vec<u8>>, Option<String>);
    let rows: Vec<Row> = sqlx::query_as(
        "select name, source, token, connection->>'cloud_id' from trackers
         where account_id = $1 order by created_at",
    )
    .bind(account)
    .fetch_all(&state.db)
    .await?;
    let key = if rows.iter().any(|(_, _, token, _)| token.is_some()) {
        Some(state.keys.account(&state.db, account).await?)
    } else {
        None
    };
    rows.into_iter()
        .map(|(name, Jsonb(mut source), token, cloud_id)| {
            source.cloud_id = cloud_id;
            let token = match (token, &key) {
                (Some(t), Some(key)) => Some(key.open_token(&name, &t)?),
                _ => None,
            };
            Ok(Stored { source, token })
        })
        .collect()
}

async fn views(state: &Shared, account: Uuid) -> Result<Json<Vec<Configured>>> {
    /// Its source, whether it has a token, who it is connected as, and what
    /// its webhooks are.
    type Row = (Jsonb<Source>, bool, Option<String>, Option<String>);
    let rows: Vec<Row> = sqlx::query_as(
        "select source, token is not null,
                case connection->>'provider'
                    when 'github' then '@' || (connection->>'login')
                    else connection->>'login' end,
                webhooks
         from trackers where account_id = $1 order by created_at",
    )
    .bind(account)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(source, token_set, connected, webhooks)| Configured {
                source: source.0,
                token_set,
                connected,
                webhooks,
            })
            .collect(),
    ))
}

/// Trimmed, named, and able to work, or why not: `hura_client::trackers`'s
/// check.
fn checked(source: &Source) -> Result<Source> {
    let source = source.normalized();
    if let Some(problem) = source.problem() {
        return Err(Error::bad(problem));
    }
    // Refused here rather than on every read after, by `outbound`: a site
    // this service will never send a request to. A bare host is HTTPS.
    if let Some(site) = &source.site
        && site.contains("://")
        && !site.to_ascii_lowercase().starts_with("https://")
    {
        return Err(Error::bad(format!(
            "`{}`: hura cloud reads trackers over https only, and {site} is not",
            source.name
        )));
    }
    Ok(source)
}

/// A Fizzy tracker's webhooks, put on its boards or brought up to date as
/// it changes. A failure is said on its row, never as the change failing.
async fn hooks(state: &Shared, account: Uuid, name: &str) {
    if let Err(e) = crate::fizzy_hooks::sync(state, account, name).await {
        eprintln!("tracker {name}: fizzy webhooks: {e:?}");
    }
}

fn found(rows: u64, name: &str) -> Result<()> {
    if rows == 0 {
        Err(Error::not_found(format!("no tracker called `{name}`")))
    } else {
        Ok(())
    }
}
