//! How often a client may ask, before it is told to wait.
//!
//! **A bucket per client and kind of request**, refilled at a steady rate
//! and allowed a burst. Kept in memory: the service is one process, and a
//! restart forgetting them costs nothing.
//!
//! **Who the client is**: its address, from the header the proxy in front
//! sets (`CF-Connecting-IP` behind Cloudflare, `HURA_CLOUD_CLIENT_IP_HEADER`
//! to change it) or from the connection when there is none. Never its bearer
//! token, which anybody could make up a new one of for every request; the
//! limits are loose enough for a few people behind one address.
//!
//! **What is counted**: signing in and swapping tokens the most tightly,
//! since that is where guessing would happen (an address is also sent five
//! codes an hour at most, in [`crate::sign_in`]); tracker requests, which
//! each read a tracker on the account's behalf, next; anything else loosely.
//! Webhooks are not: they come from GitHub's and Atlassian's few addresses,
//! are checked by their signature, and only make an account due.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Instant;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::Shared;
use crate::error::Error;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Kind {
    SignIn,
    Tracker,
    Other,
}

impl Kind {
    /// Requests a minute, and how many at once.
    fn rate(self) -> (f64, f64) {
        match self {
            Kind::SignIn => (20.0, 10.0),
            Kind::Tracker => (240.0, 120.0),
            Kind::Other => (600.0, 200.0),
        }
    }
}

pub struct Limits {
    on: bool,
    header: String,
    buckets: Mutex<HashMap<(Kind, String), (f64, Instant)>>,
}

impl Limits {
    /// Limits, with the client's address read from `header` when it is set.
    pub fn new(header: String) -> Self {
        Limits {
            on: true,
            header: header.to_ascii_lowercase(),
            buckets: Mutex::default(),
        }
    }

    /// None: for tests that make many requests and are not about this.
    #[cfg(test)]
    pub(crate) fn off() -> Self {
        Limits {
            on: false,
            ..Limits::new(String::new())
        }
    }

    /// Takes one from the bucket, or answers how many seconds until there is
    /// one.
    fn take(&self, kind: Kind, client: String) -> Result<(), u64> {
        if !self.on {
            return Ok(());
        }
        let (per_minute, burst) = kind.rate();
        let per_second = per_minute / 60.0;
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if buckets.len() > 100_000 {
            // Full ones are the same as none.
            buckets.retain(|(k, _), (left, at)| {
                let (m, b) = k.rate();
                *left + now.duration_since(*at).as_secs_f64() * m / 60.0 < b
            });
        }
        let (left, at) = buckets.entry((kind, client)).or_insert((burst, now));
        *left = (*left + now.duration_since(*at).as_secs_f64() * per_second).min(burst);
        *at = now;
        if *left >= 1.0 {
            *left -= 1.0;
            Ok(())
        } else {
            Err(((1.0 - *left) / per_second).ceil() as u64)
        }
    }
}

fn kind(path: &str) -> Option<Kind> {
    if path.starts_with("/v1/webhooks/") || path == "/healthz" {
        None
    } else if path.starts_with("/v1/sign-in/")
        || path == "/v1/token"
        || path == "/v1/oauth/apple/callback"
    {
        Some(Kind::SignIn)
    } else if path == "/v1/tracker" {
        Some(Kind::Tracker)
    } else {
        Some(Kind::Other)
    }
}

/// The client a request is counted against: its address.
fn client(limits: &Limits, request: &Request) -> String {
    let from_proxy = (!limits.header.is_empty())
        .then(|| request.headers().get(limits.header.as_str()))
        .flatten()
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string());
    let from_connection = || {
        request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip().to_string())
    };
    from_proxy
        .or_else(from_connection)
        .unwrap_or_else(|| "unknown".into())
}

/// The middleware: on to the route, or 429 with how long to wait.
pub async fn limit(State(state): State<Shared>, request: Request, next: Next) -> Response {
    let Some(kind) = kind(request.uri().path()) else {
        return next.run(request).await;
    };
    let who = client(&state.limits, &request);
    match state.limits.take(kind, who) {
        Ok(()) => next.run(request).await,
        Err(wait) => {
            let mut response = Error::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too many requests: wait a moment and try again",
            )
            .into_response();
            if let Ok(v) = HeaderValue::from_str(&wait.max(1).to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, v);
            }
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_is_allowed_and_then_the_steady_rate() {
        let limits = Limits::new(String::new());
        for _ in 0..10 {
            assert!(limits.take(Kind::SignIn, "a".into()).is_ok());
        }
        let wait = limits.take(Kind::SignIn, "a".into()).unwrap_err();
        assert!((1..=3).contains(&wait), "{wait}");
        // Somebody else, and another kind, are counted apart.
        assert!(limits.take(Kind::SignIn, "b".into()).is_ok());
        assert!(limits.take(Kind::Other, "a".into()).is_ok());
    }

    #[test]
    fn webhooks_and_the_health_check_are_not_counted() {
        assert_eq!(kind("/v1/webhooks/github"), None);
        assert_eq!(kind("/healthz"), None);
        assert_eq!(kind("/v1/sign-in/email/verify"), Some(Kind::SignIn));
        assert_eq!(kind("/v1/oauth/apple/callback"), Some(Kind::SignIn));
        assert_eq!(kind("/v1/tracker"), Some(Kind::Tracker));
        assert_eq!(kind("/v1/trackers"), Some(Kind::Other));
    }
}
