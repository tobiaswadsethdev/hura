//! What the service is told by its environment, which is how Coolify (or
//! anybody running their own) configures it.
//!
//! - `DATABASE_URL`: Postgres. Required.
//! - `LISTEN`: the address to serve on, `0.0.0.0:8080` unless set. TLS is the
//!   proxy's in front of it.
//! - `RESEND_API_KEY`: sends sign-in codes through Resend, from `MAIL_FROM`
//!   (`hura <sign-in@gethura.app>` unless set), whose domain has to be
//!   verified on that Resend account.
//! - `HURA_CLOUD_MASTER_KEY`: 32 bytes in base64 (`openssl rand -base64 32`),
//!   which wraps each account's key for its trackers' tokens. Required. Lost,
//!   every stored tracker token is lost with it: keep a copy beside the
//!   database's backups, and not in them.
//! - `HURA_CLOUD_RELAY`: the push relay tracker notifications go through,
//!   `https://hura-relay.twadseth.workers.dev` unless set. The only one: the
//!   phone links this service on it, and an address a client chose is not
//!   one the service would send to.
//! - `HURA_CLOUD_MAIL=log`: prints codes instead, for running it on a laptop.
//!   One of the two is required, so a deployment missing its key fails to
//!   start rather than signing nobody in.

use std::net::SocketAddr;

use crate::Mailer;
use crate::crypto::Keys;

pub struct Config {
    pub database_url: String,
    pub listen: SocketAddr,
    pub mail: Mailer,
    pub keys: Keys,
    pub relay: String,
}

pub const DEFAULT_FROM: &str = "hura <sign-in@gethura.app>";

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let database_url = var("DATABASE_URL").ok_or("DATABASE_URL is not set")?;
        let listen = var("LISTEN")
            .unwrap_or_else(|| "0.0.0.0:8080".into())
            .parse()
            .map_err(|e| format!("LISTEN: {e}"))?;
        let mail = match (var("RESEND_API_KEY"), var("HURA_CLOUD_MAIL").as_deref()) {
            (Some(key), _) => {
                Mailer::resend(key, var("MAIL_FROM").unwrap_or_else(|| DEFAULT_FROM.into()))
            }
            (None, Some("log")) => Mailer::Log,
            _ => {
                return Err(
                    "set RESEND_API_KEY to send sign-in codes, or HURA_CLOUD_MAIL=log to print them".into(),
                );
            }
        };
        let keys =
            Keys::from_base64(&var("HURA_CLOUD_MASTER_KEY").ok_or(
                "HURA_CLOUD_MASTER_KEY is not set; make one with `openssl rand -base64 32`",
            )?)?;
        let relay = var("HURA_CLOUD_RELAY").unwrap_or_else(|| crate::push::DEFAULT_RELAY.into());
        Ok(Config {
            database_url,
            listen,
            mail,
            keys,
            relay,
        })
    }
}
