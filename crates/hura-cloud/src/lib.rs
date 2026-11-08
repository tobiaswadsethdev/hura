//! hura cloud: accounts, and the trackers they keep, behind one HTTPS API. See `docs/plans/hura-cloud.md` for what it is for and `docs/cloud.md`
//! for running it.
//!
//! **Signing in and connecting a tracker are separate.** An account is got
//! into by an emailed code (and by Sign in with Apple, next); each device that
//! signs in holds a long-lived refresh token, listed and revocable from any
//! other, which it swaps for access tokens that last an hour.
//!
//! **Every token of ours is kept as its hash**, as `hurad` keeps its own: a
//! copy of the database is not a way into anybody's account. A tracker's
//! token, which has to be sent on, is encrypted instead (see [`crypto`]).

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post, put};
use sqlx::PgPool;

mod auth;
pub mod config;
pub mod crypto;
mod devices;
mod error;
pub mod mail;
mod outbound;
pub mod push;
mod sign_in;
mod token;
mod trackers;
pub mod watch;

#[cfg(test)]
mod tests;

pub use mail::Mailer;

/// What every request is answered with.
pub struct State {
    pub db: PgPool,
    pub mail: Mailer,
    pub keys: crypto::Keys,
    pub relay: push::Relay,
}

pub type Shared = Arc<State>;

/// Brings the database up to this build's schema. Run at every start, so a
/// new image is deployed by starting it.
pub async fn migrate(db: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!().run(db).await
}

/// Sends every tracker request over HTTPS to public addresses only (see
/// [`outbound`]), rather than through `curl`. Once per process.
pub fn install_transport() {
    hura_core::tracker::use_transport(Box::new(outbound::Public(
        hura_core::tracker::ureq::Ureq::new(),
    )));
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
        .route("/v1/trackers", get(trackers::list).post(trackers::add))
        .route(
            "/v1/trackers/{name}",
            put(trackers::edit).delete(trackers::forget),
        )
        .route("/v1/trackers/{name}/token", put(trackers::set_token))
        .route("/v1/tracker", post(trackers::call))
        .route(
            "/v1/push",
            get(push::status).put(push::configure).delete(push::remove),
        )
        .with_state(state)
}

/// Up, and the database answers: what Coolify's health check asks.
async fn healthz(
    axum::extract::State(state): axum::extract::State<Shared>,
) -> error::Result<&'static str> {
    sqlx::query("select 1").execute(&state.db).await?;
    Ok("ok")
}
