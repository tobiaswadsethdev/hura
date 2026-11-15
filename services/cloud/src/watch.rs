//! Reading each account's trackers on a timer, and telling its devices what
//! changed: what makes a tracker notification arrive within a minute or two
//! rather than whenever iOS lets the app wake.
//!
//! **What changed is worked out once**, by `hura_core::tracker::changes`, the
//! comparison the desktop and the phone make for themselves when signed out,
//! against what was seen kept per account. A change is found once and told to
//! every device.
//!
//! **Only accounts with a device to tell are read**: one with a working push
//! route (see [`crate::push`]), or a desktop with its stream open (see
//! [`crate::live`]). Each is read at most as often as its plan
//! allows, a little later at random so they do not all go at once, and less
//! often while none of its trackers answers.
//!
//! **What was seen moves on only once the telling is done.** A relay that is
//! away means the same changes are found and sent again at the next read,
//! with the same event IDs, which the relay drops as duplicates for a device
//! that already had them. A route whose token the relay refuses is marked
//! broken and left out until the phone links the service again.

use std::time::Duration;

use hura_core::tracker::Inbox;
use hura_core::tracker::changes::{self, Change, Seen};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::Shared;
use crate::error::{Error, Result};
use crate::push::{self, Sent, Text};
use crate::token;

/// How often the queue is looked at.
const EVERY: Duration = Duration::from_secs(15);
/// Accounts taken per look. Each is leased for [`LEASE`], so another instance
/// leaves it alone and one that dies mid-read does not lose it for good.
const BATCH: i64 = 20;
const LEASE: &str = "5 minutes";
/// Past this many changes in one read, one notification says how many: the
/// same line the apps draw.
const SUMMARISE_OVER: usize = 3;

/// Forever, from the start of the process.
pub async fn run(state: Shared) {
    loop {
        if let Err(e) = tick(&state).await {
            eprintln!("watch: {e:?}");
        }
        tokio::time::sleep(EVERY).await;
    }
}

/// Reads the accounts that are due. Answers how many.
pub async fn tick(state: &Shared) -> Result<usize> {
    let due: Vec<Uuid> = sqlx::query_scalar(&format!(
        "update watches set next_read_at = now() + interval '{LEASE}'
         where account_id in (
             select w.account_id from watches w
             where w.next_read_at <= now()
               and (w.account_id = any($1)
                    or exists (select 1 from push_routes p
                               where p.account_id = w.account_id and p.broken is null))
             order by w.next_read_at
             limit {BATCH}
             for update skip locked)
         returning account_id"
    ))
    .bind(state.live.accounts())
    .fetch_all(&state.db)
    .await?;
    if !due.is_empty() {
        eprintln!("watch: reading {} account(s)", due.len());
    }
    for account in &due {
        if let Err(e) = watch(state, *account).await {
            eprintln!("watch {account}: {e:?}");
        }
    }
    Ok(due.len())
}

/// One account: read, tell, and say when it is next due.
async fn watch(state: &Shared, account: Uuid) -> Result<()> {
    // Now and then, the webhooks that make the next read come sooner. Never
    // a reason not to read.
    if let Err(e) = crate::fizzy_hooks::refresh_due(state, account).await {
        eprintln!("watch {account}: fizzy webhooks: {e:?}");
    }
    if let Err(e) = crate::jira_hooks::refresh_due(state, account).await {
        eprintln!("watch {account}: jira webhooks: {e:?}");
    }
    let trackers = crate::trackers::stored(state, account).await?;
    let answered = !trackers.is_empty();
    let inbox = if trackers.is_empty() {
        Inbox::default()
    } else {
        tokio::task::spawn_blocking(move || hura_core::tracker::inbox(&trackers, "hura"))
            .await
            .map_err(Error::internal)?
    };
    let failed = answered && inbox.read.is_empty() && inbox.pulls_read.is_empty();
    if failed {
        eprintln!(
            "watch {account}: no tracker answered: {}",
            inbox.warnings.join("; ")
        );
    }
    let told = announce(state, account, &inbox).await?;
    schedule(state, account, failed, told).await
}

/// Records a read for an account, and tells its devices what changed since
/// the last. Answers whether every device that could be told was; when not,
/// what was seen stays where it was so the next read finds the same changes.
pub(crate) async fn announce(state: &Shared, account: Uuid, inbox: &Inbox) -> Result<bool> {
    let before: Option<Vec<u8>> =
        sqlx::query_scalar("select seen from watches where account_id = $1")
            .bind(account)
            .fetch_optional(&state.db)
            .await?
            .flatten();
    let seen: Option<Seen> = before
        .as_deref()
        .and_then(|b| serde_json::from_slice(b).ok());
    let (next, found) = changes::record(seen.as_ref(), inbox);

    if !found.is_empty() {
        eprintln!("watch {account}: {} change(s)", found.len());
        let since = String::from_utf8_lossy(before.as_deref().unwrap_or_default()).into_owned();
        state.live.tell(account, &batch(&since, &found), &found);
        let mut later = false;
        for route in push::routes(state, account).await? {
            for text in texts(&found) {
                let id = event_id(account, route.device, &since, &text);
                let sealed =
                    push::encrypt(&route.key, &route.server_id, &text).map_err(Error::internal)?;
                match state
                    .relay
                    .send(&route.token, &push::event(&id, sealed))
                    .await
                {
                    Sent::Done => {}
                    Sent::Refused(why) => {
                        push::mark_broken(state, route.device, &why).await?;
                        break;
                    }
                    Sent::Later(why) => {
                        eprintln!("watch {account}: {why}");
                        later = true;
                    }
                }
            }
        }
        if later {
            return Ok(false);
        }
    }

    sqlx::query("update watches set seen = $2, last_read_at = now() where account_id = $1")
        .bind(account)
        .bind(serde_json::to_vec(&next).map_err(Error::internal)?)
        .execute(&state.db)
        .await?;
    Ok(true)
}

/// When the account is next read: as often as its plan allows, with up to
/// half a minute's jitter; a minute from now when the telling should be tried
/// again; and doubling up to an hour while none of its trackers answers.
async fn schedule(state: &Shared, account: Uuid, failed: bool, told: bool) -> Result<()> {
    let (every, failures): (i32, i32) = sqlx::query_as(
        "select p.min_read_seconds, w.failures from watches w
         join accounts a on a.id = w.account_id join plans p on p.id = a.plan
         where w.account_id = $1",
    )
    .bind(account)
    .fetch_one(&state.db)
    .await?;
    let failures = if failed { failures + 1 } else { 0 };
    let jitter = token::number(30)? as i32;
    let seconds = if !told {
        60
    } else if failures > 0 {
        (every * (1 << failures.min(6))).min(3600)
    } else {
        every + jitter
    };
    sqlx::query(
        // The earlier of the two: a webhook that made the account due while
        // this read held it asked for another read, and keeps it. (The
        // lease is later than any interval, so it never wins.)
        "update watches set failures = $2,
             next_read_at = least(next_read_at, now() + $3 * interval '1 second')
         where account_id = $1",
    )
    .bind(account)
    .bind(failures)
    .bind(seconds)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// What the phone shows: each change on its own, or past a few, one line
/// saying how many, as the apps do when they read for themselves.
fn texts(found: &[Change]) -> Vec<Text> {
    if found.len() > SUMMARISE_OVER {
        return vec![Text {
            title: format!(
                "{} changes in your tickets, filters and pull requests",
                found.len()
            ),
            body: found
                .iter()
                .map(|c| format!("{} · {}", c.key, c.what))
                .collect::<Vec<_>>()
                .join("\n"),
            tracker: None,
            id: None,
        }];
    }
    found
        .iter()
        .map(|c| Text {
            title: format!("{} · {}", c.key, c.what),
            body: c.title.clone(),
            tracker: Some(c.tracker.clone()),
            id: Some(c.id.clone()),
        })
        .collect()
}

/// The same for the same changes found from the same place: what a stream
/// was told, so a retry does not tell it again.
fn batch(since: &str, found: &[Change]) -> String {
    let mut hash = Sha256::new();
    hash.update(since.as_bytes());
    for c in found {
        for part in [&c.tracker, &c.id, &c.what] {
            hash.update(part.as_bytes());
            hash.update([0]);
        }
    }
    hash.finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The same for the same change found from the same place, so a retry is a
/// duplicate to the relay rather than a second notification.
fn event_id(account: Uuid, device: Uuid, since: &str, text: &Text) -> String {
    let mut hash = Sha256::new();
    for part in [
        account.to_string().as_str(),
        device.to_string().as_str(),
        since,
        &text.title,
        &text.body,
        text.tracker.as_deref().unwrap_or_default(),
        text.id.as_deref().unwrap_or_default(),
    ] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    let digest = hash.finalize();
    let hex: String = digest[..20].iter().map(|b| format!("{b:02x}")).collect();
    format!("tc-{hex}")
}
