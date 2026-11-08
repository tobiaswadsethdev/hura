//! The service's own apps on GitHub and Atlassian, which connecting a tracker
//! goes through, and the page that sets them up.
//!
//! **Set up from a page, not the environment.** A GitHub App is made by
//! GitHub's manifest flow: one button posts the app's whole description to
//! GitHub, the person confirms, and GitHub hands its credentials straight
//! back here, so its secrets are never copied anywhere. Atlassian has no such
//! flow, so its client ID and secret are pasted into the same page. Either is
//! kept encrypted with the master key.
//!
//! **The page is behind a token in the log.** While either app is not set
//! up, the service writes `setup: <url>/setup?token=…` to its log at start.
//! The token is made fresh at each start and is the only way in; nobody who
//! cannot read the log can set up an app for the service.

use axum::Form;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use subtle::ConstantTimeEq as _;

use crate::Shared;
use crate::error::{Error, Result};

/// The GitHub App, as GitHub's manifest conversion answers.
#[derive(Clone, Serialize, Deserialize)]
pub struct GitHubApp {
    pub id: u64,
    pub slug: String,
    pub client_id: String,
    pub client_secret: String,
    pub webhook_secret: String,
    pub pem: String,
    pub html_url: String,
}

/// The OAuth 2.0 (3LO) app on Atlassian's developer console.
#[derive(Clone, Serialize, Deserialize)]
pub struct JiraApp {
    pub client_id: String,
    pub client_secret: String,
}

pub async fn github(state: &Shared) -> Result<Option<GitHubApp>> {
    load(state, "github").await
}

pub async fn jira(state: &Shared) -> Result<Option<JiraApp>> {
    load(state, "jira").await
}

async fn load<T: serde::de::DeserializeOwned>(state: &Shared, kind: &str) -> Result<Option<T>> {
    let sealed: Option<Vec<u8>> =
        sqlx::query_scalar("select config from integrations where kind = $1")
            .bind(kind)
            .fetch_optional(&state.db)
            .await?;
    let Some(sealed) = sealed else {
        return Ok(None);
    };
    let plain = state
        .keys
        .open_service(&format!("integration\0{kind}"), &sealed)?;
    Ok(Some(
        serde_json::from_slice(&plain).map_err(Error::internal)?,
    ))
}

async fn keep<T: Serialize>(state: &Shared, kind: &str, config: &T) -> Result<()> {
    let plain = serde_json::to_vec(config).map_err(Error::internal)?;
    let sealed = state
        .keys
        .seal_service(&format!("integration\0{kind}"), &plain)?;
    sqlx::query(
        "insert into integrations (kind, config) values ($1, $2)
         on conflict (kind) do update set config = $2, created_at = now()",
    )
    .bind(kind)
    .bind(sealed)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Whether the setup page's token is the one this process made.
fn allowed(state: &Shared, token: &str) -> bool {
    bool::from(token.as_bytes().ct_eq(state.setup.as_bytes())) && !token.is_empty()
}

#[derive(Deserialize)]
pub struct Token {
    #[serde(default)]
    token: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    code: String,
}

/// `GET /setup?token=…`
pub async fn page(State(state): State<Shared>, Query(q): Query<Token>) -> Result<Response> {
    if !allowed(&state, &q.token) {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let url = &state.url;
    let token = escape(&q.token);
    let github = match github(&state).await? {
        Some(app) => format!(
            "<p>Set up: <a href=\"{0}\">{1}</a>. <a href=\"{0}/installations/new\">Install it on your repositories</a>, then connect GitHub in the app.</p>",
            escape(&app.html_url),
            escape(&app.slug)
        ),
        None => format!(
            "<p>One button: GitHub shows the app it is about to make, with the permissions and webhook below, and you confirm. Its credentials come straight back here.</p>
             <form action=\"https://github.com/settings/apps/new?state={token}\" method=\"post\">
               <input type=\"hidden\" name=\"manifest\" value=\"{}\">
               <button>Create the GitHub App</button>
             </form>
             <p class=\"hint\">To make it under an organisation instead, open its settings on GitHub and use <code>/organizations/&lt;org&gt;/settings/apps/new</code>.</p>",
            escape(&manifest(url).to_string())
        ),
    };
    let jira = match jira(&state).await? {
        Some(app) => format!(
            "<p>Set up, as client <code>{}</code>. Connect Jira in the app.</p>",
            escape(&app.client_id)
        ),
        None => format!(
            "<ol>
               <li>At <a href=\"https://developer.atlassian.com/console/myapps/\">developer.atlassian.com/console/myapps</a>, <b>Create → OAuth 2.0 integration</b>.</li>
               <li><b>Permissions</b>: add <b>Jira API</b> with <code>read:jira-work</code>, <code>write:jira-work</code> and <code>read:jira-user</code>.</li>
               <li><b>Authorization</b>: OAuth 2.0 (3LO), callback URL <code>{url}/v1/oauth/jira/callback</code>.</li>
               <li><b>Settings</b>: copy the client ID and secret here.</li>
             </ol>
             <form action=\"{url}/setup/jira?token={token}\" method=\"post\">
               <label>Client ID <input name=\"client_id\" required></label>
               <label>Secret <input name=\"client_secret\" type=\"password\" required></label>
               <button>Save</button>
             </form>"
        ),
    };
    Ok(Html(layout(
        "Set up hura cloud",
        &format!("<h2>GitHub</h2>{github}<h2>Jira</h2>{jira}"),
    ))
    .into_response())
}

/// The GitHub App, described for GitHub's manifest flow. Private to the
/// account that makes it until it is made public in its settings, which is
/// what letting other people connect GitHub will take.
fn manifest(url: &str) -> Value {
    json!({
        "name": "hura cloud",
        "url": "https://gethura.app",
        "description": "Your issues and pull requests in hura, and a notification when one changes.",
        "hook_attributes": { "url": format!("{url}/v1/webhooks/github"), "active": true },
        "redirect_url": format!("{url}/setup/github/done"),
        "callback_urls": [format!("{url}/v1/oauth/github/callback")],
        "setup_url": format!("{url}/v1/oauth/github/callback"),
        "setup_on_update": true,
        "public": false,
        "default_permissions": {
            "metadata": "read",
            "issues": "write",
            "pull_requests": "write",
            "contents": "write",
            "checks": "read",
            "statuses": "read"
        },
        "default_events": [
            "issues", "issue_comment", "pull_request", "pull_request_review",
            "pull_request_review_comment", "check_suite", "status"
        ]
    })
}

/// `GET /setup/github/done?code=…&state=…`: GitHub made the app; its
/// credentials, for the code it sent back, are kept.
pub async fn github_done(State(state): State<Shared>, Query(q): Query<Token>) -> Result<Response> {
    if !allowed(&state, &q.state) {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let code: String = q
        .code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if code.is_empty() {
        return Err(Error::bad("GitHub sent no code"));
    }
    let (status, text) = state
        .web
        .send(
            "POST",
            &format!("https://api.github.com/app-manifests/{code}/conversions"),
            &[("Accept", "application/vnd.github+json".into())],
            None,
        )
        .await
        .map_err(Error::internal)?;
    if status != 201 {
        return Err(Error::internal(format!(
            "GitHub's manifest conversion answered {status}: {text}"
        )));
    }
    let app: GitHubApp = serde_json::from_str(&text).map_err(Error::internal)?;
    keep(&state, "github", &app).await?;
    Ok(Html(layout(
        "GitHub App made",
        &format!(
            "<p><b>{0}</b> is made. Now <a href=\"{1}/installations/new\">install it</a> on the repositories hura should read, then connect GitHub in the app.</p>
             <p><a href=\"{2}/setup?token={3}\">Back to setup</a></p>",
            escape(&app.slug),
            escape(&app.html_url),
            state.url,
            escape(&q.state)
        ),
    ))
    .into_response())
}

#[derive(Deserialize)]
pub struct JiraForm {
    client_id: String,
    client_secret: String,
}

/// `POST /setup/jira?token=…`
pub async fn jira_save(
    State(state): State<Shared>,
    Query(q): Query<Token>,
    Form(form): Form<JiraForm>,
) -> Result<Response> {
    if !allowed(&state, &q.token) {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let app = JiraApp {
        client_id: form.client_id.trim().to_string(),
        client_secret: form.client_secret.trim().to_string(),
    };
    if app.client_id.is_empty() || app.client_secret.is_empty() {
        return Err(Error::bad("both the client ID and the secret are needed"));
    }
    keep(&state, "jira", &app).await?;
    Ok(Html(layout(
        "Jira set up",
        &format!(
            "<p>Saved. Connect Jira in the app.</p><p><a href=\"{}/setup?token={}\">Back to setup</a></p>",
            state.url,
            escape(&q.token)
        ),
    ))
    .into_response())
}

/// A page of the service's own, plain and readable on a phone.
pub fn layout(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">
         <title>{0}</title>
         <style>body{{font:16px/1.5 -apple-system,system-ui,sans-serif;max-width:640px;margin:2em auto;padding:0 1em;color:#1d1d1f}}
         h1{{font-size:1.5em}}h2{{font-size:1.1em;margin-top:2em}}code{{background:#f2f2f2;padding:0 .3em;border-radius:4px}}
         button{{font:inherit;padding:.5em 1.2em;border-radius:8px;border:0;background:#2d6a2d;color:#fff}}
         label{{display:block;margin:.6em 0}}input{{font:inherit;width:100%;padding:.4em;box-sizing:border-box}}.hint{{color:#666;font-size:.9em}}</style>
         </head><body><h1>{0}</h1>{1}</body></html>",
        escape(title),
        body
    )
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
