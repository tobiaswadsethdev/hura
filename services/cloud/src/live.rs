//! Telling a desktop, over a connection it keeps open: `GET /v1/events`, a
//! stream of server-sent events, one `tracker_change` for each read of the
//! account's trackers that found something.
//!
//! **Only while it is open.** A desktop whose window is closed is not told,
//! and does not need to be: it was never going to show an OS notification
//! then, and its inbox is read again when it opens. A phone is the device
//! that has to hear about a ticket while it is not looking (see
//! [`crate::push`]).
//!
//! **An account with a stream open is read**, as one with a push route is
//! (see [`crate::watch`]). The streams are this process's own; a second
//! instance of the service would read the account and tell only the streams
//! open on it, which is a reason to run one until this is shared.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use hura_core::tracker::changes::Change;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::Shared;
use crate::auth::Signed;
use crate::error::Result;

/// The streams open, by account.
#[derive(Default)]
pub struct Live {
    accounts: Mutex<HashMap<Uuid, broadcast::Sender<Value>>>,
    /// What each account was last told, by a key for the batch: a read whose
    /// telling the relay put off finds the same changes again, and a stream
    /// that already had them should not have them twice.
    told: Mutex<HashMap<Uuid, String>>,
}

impl Live {
    /// A new listener for an account's changes.
    pub fn subscribe(&self, account: Uuid) -> broadcast::Receiver<Value> {
        let mut accounts = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        accounts
            .entry(account)
            .or_insert_with(|| broadcast::channel(32).0)
            .subscribe()
    }

    /// The accounts with a stream open right now. Those whose last stream
    /// has closed are forgotten on the way.
    pub fn accounts(&self) -> Vec<Uuid> {
        let mut accounts = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        accounts.retain(|_, sender| sender.receiver_count() > 0);
        accounts.keys().copied().collect()
    }

    /// Tells every stream open for the account, once for each `batch`.
    /// Nobody listening is not a failure: a stream is told only while it is
    /// there.
    pub fn tell(&self, account: Uuid, batch: &str, found: &[Change]) {
        {
            let mut told = self.told.lock().unwrap_or_else(|e| e.into_inner());
            if told.get(&account).is_some_and(|b| b == batch) {
                return;
            }
            told.insert(account, batch.to_string());
        }
        let accounts = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = accounts.get(&account) {
            let _ = sender.send(json!({ "changes": found }));
        }
    }
}

/// `GET /v1/events`: the account's changes as they are found, until the
/// connection closes. A comment every fifteen seconds keeps proxies (and
/// Cloudflare's, which closes a quiet connection after a hundred) from
/// deciding it is dead.
pub async fn events(
    State(state): State<Shared>,
    signed: Signed,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
    let receiver = state.live.subscribe(signed.account);
    // Read from now on, starting soon; the first read is the baseline.
    sqlx::query("insert into watches (account_id) values ($1) on conflict (account_id) do nothing")
        .bind(signed.account)
        .execute(&state.db)
        .await?;
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(value) => {
                    let event = Event::default()
                        .event("tracker_change")
                        .data(value.to_string());
                    return Some((Ok(event), receiver));
                }
                // Too slow to keep up: what was missed is in the inbox.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
