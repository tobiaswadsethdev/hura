//! Paying for a plan, through Stripe.
//!
//! **Stripe keeps the cards, the invoices and the subscription**; this keeps
//! which Stripe customer an account is, its subscription as Stripe last
//! said, and the plan that follows from it. Subscribing is Stripe Checkout
//! and changing or cancelling is Stripe's customer portal, both pages of
//! Stripe's that a device sends the browser to.
//!
//! **The plan follows Stripe's webhook**, never the browser coming back: a
//! subscription that is active, trialing or past due (while Stripe retries
//! the card) is on the paid plan, and any other is back on `free`. Which
//! account a subscription is for rides on it as metadata, put there at
//! checkout, so its events find the account whatever order they come in.
//! Each is checked against the webhook's signing secret, and one older than
//! the last applied is ignored.
//!
//! **Configured by environment**: `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`
//! and `STRIPE_PRICE_ID` (the subscription's price), and `STRIPE_PLAN`, the
//! plan it buys (`pro` unless set). Without them there is nothing to buy.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Html;
use ring::hmac;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::Shared;
use crate::auth::Signed;
use crate::error::{Error, Result};
use crate::integrations::layout;
use crate::oauth::encode;

const API: &str = "https://api.stripe.com/v1";
/// How old a webhook's signature may be, as Stripe's own libraries allow.
const TOLERANCE: u64 = 5 * 60;

pub struct Stripe {
    pub secret_key: String,
    pub webhook_secret: String,
    pub price: String,
    pub plan: String,
}

impl Stripe {
    /// From the environment, or none when it is not all there.
    pub fn from_env() -> Option<Stripe> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Some(Stripe {
            secret_key: var("STRIPE_SECRET_KEY")?,
            webhook_secret: var("STRIPE_WEBHOOK_SECRET")?,
            price: var("STRIPE_PRICE_ID")?,
            plan: var("STRIPE_PLAN").unwrap_or_else(|| "pro".into()),
        })
    }
}

fn stripe(state: &Shared) -> Result<&Stripe> {
    state.stripe.as_ref().ok_or_else(|| {
        Error::new(
            StatusCode::CONFLICT,
            "hura cloud is not set up to take payments",
        )
    })
}

/// A request to Stripe's API, form-encoded as it takes them.
async fn call(state: &Shared, path: &str, form: &[(&str, String)]) -> Result<Value> {
    let key = &stripe(state)?.secret_key;
    let body = form
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let (status, text) = state
        .web
        .send(
            "POST",
            &format!("{API}{path}"),
            &[
                ("Authorization", format!("Bearer {key}")),
                (
                    "Content-Type",
                    "application/x-www-form-urlencoded".to_string(),
                ),
            ],
            Some(body),
        )
        .await
        .map_err(Error::internal)?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status != 200 {
        return Err(Error::internal(format!(
            "Stripe {path} answered {status}: {}",
            v["error"]["message"].as_str().unwrap_or(&text)
        )));
    }
    Ok(v)
}

/// The account's customer at Stripe and whether its subscription is live.
async fn customer(state: &Shared, account: Uuid) -> Result<(Option<String>, bool)> {
    let (customer, live): (Option<String>, Option<bool>) = sqlx::query_as(
        "select stripe_customer,
                subscription->>'status' in ('active', 'trialing', 'past_due')
         from accounts where id = $1",
    )
    .bind(account)
    .fetch_one(&state.db)
    .await?;
    Ok((customer, live.unwrap_or(false)))
}

/// `POST /v1/billing/checkout`: where to send the browser to subscribe.
pub async fn checkout(State(state): State<Shared>, signed: Signed) -> Result<Json<Value>> {
    let stripe = stripe(&state)?;
    let (known, live) = customer(&state, signed.account).await?;
    if live {
        return Err(Error::new(
            StatusCode::CONFLICT,
            "this account is subscribed already; manage it instead",
        ));
    }
    let account = signed.account.to_string();
    let mut form = vec![
        ("mode", "subscription".to_string()),
        ("line_items[0][price]", stripe.price.clone()),
        ("line_items[0][quantity]", "1".into()),
        ("client_reference_id", account.clone()),
        ("subscription_data[metadata][account]", account),
        ("success_url", format!("{}/billing/done", state.url)),
        (
            "cancel_url",
            format!("{}/billing/done?cancelled=1", state.url),
        ),
    ];
    match known {
        Some(customer) => form.push(("customer", customer)),
        None => {
            let email: Option<String> = sqlx::query_scalar(
                "select email from sign_ins where account_id = $1 and email is not null
                 order by created_at limit 1",
            )
            .bind(signed.account)
            .fetch_optional(&state.db)
            .await?;
            if let Some(email) = email {
                form.push(("customer_email", email));
            }
        }
    }
    let session = call(&state, "/checkout/sessions", &form).await?;
    let url = session["url"]
        .as_str()
        .ok_or_else(|| Error::internal("Stripe's checkout had no URL"))?;
    Ok(Json(json!({ "url": url })))
}

/// `POST /v1/billing/portal`: where to send the browser to change, cancel or
/// see the invoices of a subscription.
pub async fn portal(State(state): State<Shared>, signed: Signed) -> Result<Json<Value>> {
    stripe(&state)?;
    let (known, _) = customer(&state, signed.account).await?;
    let customer = known.ok_or_else(|| Error::bad("this account has never subscribed"))?;
    let session = call(
        &state,
        "/billing_portal/sessions",
        &[
            ("customer", customer),
            ("return_url", format!("{}/billing/done", state.url)),
        ],
    )
    .await?;
    let url = session["url"]
        .as_str()
        .ok_or_else(|| Error::internal("Stripe's portal had no URL"))?;
    Ok(Json(json!({ "url": url })))
}

/// Ends the account's subscription at once, if it has a live one: before
/// the account is deleted, so nobody is charged for an account that is gone.
pub async fn cancel(state: &Shared, account: Uuid) -> Result<()> {
    let id: Option<String> = sqlx::query_scalar(
        "select subscription->>'id' from accounts where id = $1
         and subscription->>'status' not in ('canceled', 'incomplete_expired')",
    )
    .bind(account)
    .fetch_optional(&state.db)
    .await?
    .flatten();
    let Some(id) = id else { return Ok(()) };
    let key = &stripe(state)?.secret_key;
    let (status, text) = state
        .web
        .send(
            "DELETE",
            &format!("{API}/subscriptions/{}", encode(&id)),
            &[("Authorization", format!("Bearer {key}"))],
            None,
        )
        .await
        .map_err(Error::internal)?;
    // Already gone at Stripe is as good as cancelled here.
    if status == 200 || status == 404 {
        return Ok(());
    }
    Err(Error::internal(format!(
        "Stripe would not cancel {id} ({status}): {text}"
    )))
}

/// `GET /billing/done`: where Stripe sends the browser back to.
pub async fn done(
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Html<String> {
    let body = if q.contains_key("cancelled") {
        "<p>Nothing was bought. Go back to hura.</p>"
    } else {
        "<p>Done. Go back to hura: your plan changes as soon as Stripe confirms it, usually within seconds.</p>"
    };
    Html(layout("hura cloud", body))
}

/// `POST /v1/webhooks/stripe`: a subscription made, changed or ended.
pub async fn webhook(
    State(state): State<Shared>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode> {
    let Some(stripe) = state.stripe.as_ref() else {
        return Ok(StatusCode::NOT_FOUND);
    };
    let header = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(Error::unauthorized)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    if !signed_by(&stripe.webhook_secret, header, &body, now) {
        return Err(Error::unauthorized());
    }
    let event: Value = serde_json::from_slice(&body).map_err(|_| Error::bad("not JSON"))?;
    let kind = event["type"].as_str().unwrap_or_default();
    if !kind.starts_with("customer.subscription.") {
        return Ok(StatusCode::OK);
    }
    let sub = &event["data"]["object"];
    let Some(account) = sub["metadata"]["account"]
        .as_str()
        .and_then(|a| Uuid::parse_str(a).ok())
    else {
        eprintln!("stripe: {kind} with no account");
        return Ok(StatusCode::OK);
    };
    let status = if kind == "customer.subscription.deleted" {
        "canceled"
    } else {
        sub["status"].as_str().unwrap_or_default()
    };
    let paid = matches!(status, "active" | "trialing" | "past_due");
    let kept = json!({
        "id": sub["id"],
        "status": status,
        "current_period_end": sub["current_period_end"]
            .as_i64()
            .or_else(|| sub["items"]["data"][0]["current_period_end"].as_i64()),
        "cancel_at_period_end": sub["cancel_at_period_end"].as_bool().unwrap_or(false),
    });
    let at = event["created"].as_i64().unwrap_or_default();
    let changed = sqlx::query(
        "update accounts set
             stripe_customer = coalesce($2, stripe_customer),
             subscription = $3, subscription_at = $4,
             plan = case when $5 then $6 else 'free' end
         where id = $1 and (subscription_at is null or subscription_at <= $4)",
    )
    .bind(account)
    .bind(sub["customer"].as_str())
    .bind(sqlx::types::Json(&kept))
    .bind(at)
    .bind(paid)
    .bind(&stripe.plan)
    .execute(&state.db)
    .await?;
    eprintln!(
        "stripe: {kind} {account} {status}{}",
        if changed.rows_affected() == 0 {
            " (older than what was kept, or no such account)"
        } else {
            ""
        }
    );
    Ok(StatusCode::OK)
}

/// Whether `header` (`t=...,v1=...`) signs `body` with `secret`, made within
/// the tolerance of `now`.
fn signed_by(secret: &str, header: &str, body: &[u8], now: u64) -> bool {
    let mut at = None;
    let mut signatures = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", t)) => at = t.parse::<u64>().ok(),
            Some(("v1", s)) => signatures.push(s),
            _ => {}
        }
    }
    let Some(at) = at else { return false };
    if now.abs_diff(at) > TOLERANCE {
        return false;
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let mut signed = format!("{at}.").into_bytes();
    signed.extend_from_slice(body);
    signatures.iter().any(|s| {
        crate::webhooks::hex(s).is_some_and(|sig| hmac::verify(&key, &signed, &sig).is_ok())
    })
}

/// A signature header for `body`, as Stripe makes them: for the tests.
#[cfg(test)]
pub(crate) fn sign(secret: &str, body: &str, at: u64) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let tag = hmac::sign(&key, format!("{at}.{body}").as_bytes());
    let hex: String = tag.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    format!("t={at},v1={hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_is_checked_with_its_time() {
        let header = sign("whsec_x", "{}", 1000);
        assert!(signed_by("whsec_x", &header, b"{}", 1100));
        assert!(
            !signed_by("whsec_y", &header, b"{}", 1100),
            "another secret"
        );
        assert!(!signed_by("whsec_x", &header, b"{ }", 1100), "another body");
        assert!(!signed_by("whsec_x", &header, b"{}", 1000 + 301), "too old");
        assert!(!signed_by("whsec_x", "v1=00", b"{}", 1000), "no time");
    }
}
