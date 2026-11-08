//! hura cloud: accounts, and (soon) the trackers they keep, behind one HTTPS
//! API. See `docs/plans/hura-cloud.md` for what it is for and `docs/cloud.md`
//! for running it.
//!
//! **Signing in and connecting a tracker are separate.** An account is got
//! into by an emailed code (and by Sign in with Apple, next); each device that
//! signs in holds a long-lived refresh token, listed and revocable from any
//! other, which it swaps for access tokens that last an hour.
//!
//! **Every token is kept as its hash**, as `hurad` keeps its own: a copy of
//! the database is not a way into anybody's account.

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post};
use sqlx::PgPool;

mod auth;
pub mod config;
mod devices;
mod error;
pub mod mail;
mod sign_in;
mod token;

#[cfg(test)]
mod tests;

pub use mail::Mailer;

/// What every request is answered with.
pub struct State {
    pub db: PgPool,
    pub mail: Mailer,
}

pub type Shared = Arc<State>;

/// Brings the database up to this build's schema. Run at every start, so a
/// new image is deployed by starting it.
pub async fn migrate(db: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!().run(db).await
}

pub fn app(state: Shared) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/sign-in/email", post(sign_in::send_code))
        .route("/v1/sign-in/email/verify", post(sign_in::verify))
        .route("/v1/token", post(auth::refresh))
        .route("/v1/sign-out", post(devices::sign_out))
        .route("/v1/me", get(devices::me))
        .route("/v1/devices", get(devices::list))
        .route("/v1/devices/{id}", delete(devices::revoke))
        .with_state(state)
}

/// Up, and the database answers: what Coolify's health check asks.
async fn healthz(
    axum::extract::State(state): axum::extract::State<Shared>,
) -> error::Result<&'static str> {
    sqlx::query("select 1").execute(&state.db).await?;
    Ok("ok")
}
