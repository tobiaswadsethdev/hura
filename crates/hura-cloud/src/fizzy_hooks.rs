//! Webhooks on a Fizzy tracker's boards (see `hura_core::tracker::
//! fizzy_hooks`): what makes a change to a card a notification within
//! seconds rather than at the account's next read.
//!
//! **Made by the service, with the tracker's own token.** Adding a Fizzy
//! tracker, giving it a new token or changing which account it reads puts a
//! webhook on every board the token runs, addressed to
//! `/v1/webhooks/fizzy/{id}` with an ID made here; Fizzy makes each one's
//! secret and shows it once, and it is kept encrypted with the account's key.
//! A board in an account the token is only a member of gets none, since only
//! an owner or an admin may add one, and is read on the timer as before.
//!
//! **Looked at again every few hours**, as part of the account's read: a
//! board added since gets one, one Fizzy turned off after failed deliveries
//! is turned back on, one somebody removed is made again, and one whose
//! board is gone is forgotten. Removing the tracker removes its webhooks.
//!
//! What a look found is kept in words on the tracker's row, for the settings
//! screen: `webhooks on 3 boards`, or why not.

use hura_core::tracker::fizzy_hooks::{self, Hook, State as HookState};
use hura_core::tracker::{Kind, Stored};
use uuid::Uuid;

use crate::Shared;
use crate::error::{Error, Result};

/// How long a look at a tracker's boards holds.
const RECHECK: &str = "6 hours";

/// The label a webhook's secret is sealed under.
pub(crate) fn label(id: Uuid) -> String {
    format!("fizzy-hook\0{id}")
}

/// A webhook made earlier: its ID here, its account, board and Fizzy's ID.
type Row = (Uuid, String, String, String);

/// What one look decided, worked out away from the database.
#[derive(Default)]
struct Look {
    made: Vec<(Uuid, String, String, Hook)>,
    gone: Vec<Uuid>,
    said: Option<String>,
}

/// Put webhooks on a tracker's boards, or bring them up to date. Nothing for
/// a tracker that is not Fizzy's. A tracker that cannot be read says so on
/// its row rather than failing whatever asked.
pub async fn sync(state: &Shared, account: Uuid, name: &str) -> Result<()> {
    let tracker = crate::trackers::stored(state, account)
        .await?
        .into_iter()
        .find(|t| t.source.name == name);
    let Some(tracker) = tracker.filter(|t| t.source.kind == Kind::Fizzy) else {
        return Ok(());
    };
    let rows: Vec<Row> = sqlx::query_as(
        "select id, fizzy_account, board_id, webhook_id from fizzy_hooks
         where account_id = $1 and tracker = $2",
    )
    .bind(account)
    .bind(name)
    .fetch_all(&state.db)
    .await?;
    let base = format!("{}/v1/webhooks/fizzy", state.url);
    let held = tracker.clone();
    let look = tokio::task::spawn_blocking(move || look(&held, &rows, &base))
        .await
        .map_err(Error::internal)?;

    // The gone first: a webhook made again for the same board takes its
    // place in the one-per-board rule.
    if !look.gone.is_empty() {
        sqlx::query("delete from fizzy_hooks where id = any($1)")
            .bind(&look.gone)
            .execute(&state.db)
            .await?;
    }
    let key = state.keys.account(&state.db, account).await?;
    for (id, fizzy_account, board, hook) in look.made {
        let kept = sqlx::query(
            "insert into fizzy_hooks
                 (id, account_id, tracker, fizzy_account, board_id, webhook_id, secret)
             values ($1, $2, $3, $4, $5, $6, $7)
             on conflict (account_id, tracker, fizzy_account, board_id) do nothing",
        )
        .bind(id)
        .bind(account)
        .bind(name)
        .bind(&fizzy_account)
        .bind(&board)
        .bind(&hook.id)
        .bind(key.seal(&label(id), hook.secret.as_bytes())?)
        .execute(&state.db)
        .await?;
        if kept.rows_affected() == 0 {
            // Another look made one for this board meanwhile: this one is
            // not wanted on the board.
            let tracker = tracker.clone();
            let _ = tokio::task::spawn_blocking(move || {
                fizzy_hooks::delete(&tracker, &fizzy_account, &board, &hook.id)
            })
            .await;
        }
    }
    sqlx::query(
        "update trackers set webhooks = $3, webhooks_checked_at = now()
         where account_id = $1 and name = $2",
    )
    .bind(account)
    .bind(name)
    .bind(&look.said)
    .execute(&state.db)
    .await?;
    Ok(())
}

fn look(tracker: &Stored, rows: &[Row], base: &str) -> Look {
    let mut look = Look::default();
    if tracker.token.is_none() {
        return look;
    }
    let boards = match fizzy_hooks::boards(tracker) {
        Ok(boards) => boards,
        Err(e) => {
            look.said = Some(format!("webhooks not looked at: {e}"));
            return look;
        }
    };

    // On a board that is gone, or in an account the tracker no longer reads.
    for (id, account, board, hook) in rows {
        if !boards
            .iter()
            .any(|b| &b.account == account && &b.id == board)
        {
            let _ = fizzy_hooks::delete(tracker, account, board, hook);
            look.gone.push(*id);
        }
    }

    let mut on = 0;
    let mut members: Vec<String> = Vec::new();
    let mut failed: Option<String> = None;
    for board in &boards {
        if !board.admin {
            if !members.contains(&board.account_name) {
                members.push(board.account_name.clone());
            }
            continue;
        }
        let row = rows
            .iter()
            .find(|(_, a, b, _)| a == &board.account && b == &board.id);
        let make = match row {
            None => true,
            Some((id, account, board_id, hook)) => {
                match fizzy_hooks::state(tracker, account, board_id, hook) {
                    Ok(HookState::Active) => {
                        on += 1;
                        false
                    }
                    Ok(HookState::Inactive) => {
                        match fizzy_hooks::activate(tracker, account, board_id, hook) {
                            Ok(()) => on += 1,
                            Err(e) => failed = Some(e),
                        }
                        false
                    }
                    Ok(HookState::Gone) => {
                        look.gone.push(*id);
                        true
                    }
                    Err(e) => {
                        failed = Some(e);
                        false
                    }
                }
            }
        };
        // Once one could not be made the rest will not be either: a token
        // that can only read is the usual reason.
        if make && failed.is_none() {
            let made = crate::token::id()
                .map_err(|e| format!("{e:?}"))
                .and_then(|id| {
                    let url = format!("{base}/{id}");
                    fizzy_hooks::create(tracker, &board.account, &board.id, &url)
                        .map(|hook| (id, hook))
                });
            match made {
                Ok((id, hook)) => {
                    look.made
                        .push((id, board.account.clone(), board.id.clone(), hook));
                    on += 1;
                }
                Err(e) => failed = Some(e),
            }
        }
    }
    look.said = said(on, boards.is_empty(), &members, failed.as_deref());
    look
}

/// What a look found, in a line.
fn said(on: usize, no_boards: bool, members: &[String], failed: Option<&str>) -> Option<String> {
    if no_boards {
        return Some("no boards to put webhooks on".into());
    }
    let mut parts = Vec::new();
    if on > 0 {
        parts.push(format!(
            "webhooks on {on} board{}",
            if on == 1 { "" } else { "s" }
        ));
    }
    if let Some(e) = failed {
        parts.push(format!("webhooks could not be added: {e}"));
    }
    if !members.is_empty() {
        parts.push(format!(
            "only an owner or admin of {} can add webhooks, so it is read every minute or so",
            members.join(", ")
        ));
    }
    Some(parts.join("; "))
}

/// Take a tracker's webhooks off its boards, as it is removed. Whatever
/// cannot be reached is left to Fizzy, which turns off a webhook whose
/// deliveries keep failing.
pub async fn remove(state: &Shared, account: Uuid, name: &str) -> Result<()> {
    let rows: Vec<Row> = sqlx::query_as(
        "select id, fizzy_account, board_id, webhook_id from fizzy_hooks
         where account_id = $1 and tracker = $2",
    )
    .bind(account)
    .bind(name)
    .fetch_all(&state.db)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let tracker = crate::trackers::stored(state, account)
        .await?
        .into_iter()
        .find(|t| t.source.name == name);
    if let Some(tracker) = tracker {
        tokio::task::spawn_blocking(move || {
            for (_, account, board, hook) in &rows {
                let _ = fizzy_hooks::delete(&tracker, account, board, hook);
            }
        })
        .await
        .map_err(Error::internal)?;
    }
    Ok(())
}

/// Look again at an account's Fizzy trackers whose last look is old.
pub async fn refresh_due(state: &Shared, account: Uuid) -> Result<()> {
    let due: Vec<String> = sqlx::query_scalar(&format!(
        "select name from trackers
         where account_id = $1 and source->>'kind' = 'fizzy' and token is not null
           and (webhooks_checked_at is null
                or webhooks_checked_at < now() - interval '{RECHECK}')"
    ))
    .bind(account)
    .fetch_all(&state.db)
    .await?;
    for name in due {
        sync(state, account, &name).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_is_said_in_a_line() {
        assert_eq!(said(3, false, &[], None).unwrap(), "webhooks on 3 boards");
        assert_eq!(
            said(0, false, &["Acme".into()], None).unwrap(),
            "only an owner or admin of Acme can add webhooks, so it is read every minute or so"
        );
        assert_eq!(
            said(1, false, &[], Some("401: insufficient scope")).unwrap(),
            "webhooks on 1 board; webhooks could not be added: 401: insufficient scope"
        );
        assert_eq!(
            said(0, true, &[], None).unwrap(),
            "no boards to put webhooks on"
        );
    }
}
