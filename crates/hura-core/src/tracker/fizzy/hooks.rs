//! Webhooks on a Fizzy tracker's boards: what lets hura cloud read a card
//! the moment it changes, rather than at the tracker's next turn.
//!
//! **One per board.** Fizzy hangs a webhook on a board, not an account, and
//! only an account's owner or an admin may add one; so each board the token
//! can see in an account it runs gets one, and an account it is only a
//! member of gets none and is read on the timer as before.
//!
//! **Each has its own secret**, which Fizzy makes and hands back once, as the
//! webhook is made: every delivery is signed with it (`X-Webhook-Signature`,
//! HMAC-SHA256 of the body, in hex). Keeping it, and checking deliveries
//! against it, is hura cloud's; this only makes, checks and removes them.

use serde_json::{Value, json};

use super::{auth, connect, is_id, pages, send};
use crate::tracker::{Stored, answer, raw, string};

/// Everything Fizzy will send about a board: any of them can change what is
/// yours, or the status or conversation of a card that is.
pub const ACTIONS: &[&str] = &[
    "card_assigned",
    "card_closed",
    "card_postponed",
    "card_auto_postponed",
    "card_board_changed",
    "card_published",
    "card_reopened",
    "card_sent_back_to_triage",
    "card_triaged",
    "card_unassigned",
    "comment_created",
];

/// What a webhook is called on the board's settings page.
const NAME: &str = "hura";

/// A board the tracker reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    /// The account's number, without its slash: `897362094`.
    pub account: String,
    pub account_name: String,
    pub id: String,
    pub name: String,
    /// Whether the token may put a webhook on it.
    pub admin: bool,
}

/// A webhook just made, with the secret its deliveries are signed with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    pub id: String,
    pub secret: String,
}

/// Where a webhook made earlier stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Active,
    /// Turned off by Fizzy after deliveries kept failing, or by somebody.
    Inactive,
    /// Removed, or its board was.
    Gone,
}

/// Every board in every account the tracker reads.
pub fn boards(stored: &Stored) -> Result<Vec<Board>, String> {
    let conn = connect(stored)?;
    let mut out = Vec::new();
    for account in &conn.accounts {
        let url = format!("{}{}/boards", conn.site, account.slug);
        let (list, _) = pages(&url, conn.token).map_err(|e| format!("{}: {e}", account.name))?;
        out.extend(list.iter().filter_map(|b| {
            let id = string(b, "id");
            is_id(&id).then(|| Board {
                account: account.slug.trim_matches('/').to_string(),
                account_name: account.name.clone(),
                name: string(b, "name"),
                id,
                admin: account.admin,
            })
        }));
    }
    Ok(out)
}

/// Put a webhook on a board, sending every action to `url`.
pub fn create(stored: &Stored, account: &str, board: &str, url: &str) -> Result<Hook, String> {
    let (base, token) = hooks_url(stored, account, board)?;
    let made = send(
        "POST",
        &base,
        token,
        Some(&json!({ "webhook": {
            "name": NAME,
            "url": url,
            "subscribed_actions": ACTIONS,
        } })),
    )?;
    let hook = Hook {
        id: string(&made, "id"),
        secret: string(&made, "signing_secret"),
    };
    if !is_id(&hook.id) || hook.secret.is_empty() {
        return Err("fizzy made the webhook but did not say what its secret is".into());
    }
    Ok(hook)
}

/// Whether a webhook is still there, and on.
pub fn state(stored: &Stored, account: &str, board: &str, id: &str) -> Result<State, String> {
    let (base, token) = hooks_url(stored, account, board)?;
    check(id)?;
    let (code, body) = raw(
        &format!("{base}/{id}"),
        &auth(token),
        crate::tracker::JSON,
        &[],
        None,
    )?;
    if code == 404 {
        return Ok(State::Gone);
    }
    let hook = answer(&String::from_utf8_lossy(&body), code)?;
    Ok(match hook.get("active").and_then(Value::as_bool) {
        Some(false) => State::Inactive,
        _ => State::Active,
    })
}

/// Turn a webhook Fizzy turned off back on.
pub fn activate(stored: &Stored, account: &str, board: &str, id: &str) -> Result<(), String> {
    let (base, token) = hooks_url(stored, account, board)?;
    check(id)?;
    send("POST", &format!("{base}/{id}/activation"), token, None)?;
    Ok(())
}

/// Take a webhook away. One already gone is gone.
pub fn delete(stored: &Stored, account: &str, board: &str, id: &str) -> Result<(), String> {
    let (base, token) = hooks_url(stored, account, board)?;
    check(id)?;
    let (code, body) = raw(
        &format!("{base}/{id}"),
        &auth(token),
        crate::tracker::JSON,
        &[],
        Some(("DELETE", "", crate::tracker::JSON)),
    )?;
    if code == 404 {
        return Ok(());
    }
    answer(&String::from_utf8_lossy(&body), code).map(|_| ())
}

/// `https://app.fizzy.do/897362094/boards/{board}/webhooks`, and the token.
fn hooks_url<'a>(
    stored: &'a Stored,
    account: &str,
    board: &str,
) -> Result<(String, &'a str), String> {
    if !is_id(account) {
        return Err(format!("`{account}` is not a fizzy account"));
    }
    if !is_id(board) {
        return Err(format!("`{board}` is not a fizzy board"));
    }
    let token = stored
        .token
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| format!("{}: no token yet", stored.source.name))?;
    Ok((
        format!(
            "{}/{account}/boards/{board}/webhooks",
            super::site(&stored.source)
        ),
        token,
    ))
}

fn check(id: &str) -> Result<(), String> {
    if is_id(id) {
        Ok(())
    } else {
        Err(format!("`{id}` is not a fizzy webhook"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::{Kind, Source};

    fn stored() -> Stored {
        Stored {
            source: Source {
                kind: Kind::Fizzy,
                name: "fizzy".into(),
                ..Default::default()
            },
            token: Some("t".into()),
        }
    }

    #[test]
    fn only_ids_go_into_a_path() {
        assert!(create(&stored(), "../x", "b", "https://x").is_err());
        assert!(create(&stored(), "1", "b/../c", "https://x").is_err());
        assert!(state(&stored(), "1", "b", "a?b").is_err());
        assert!(delete(&stored(), "1", "b", "").is_err());
        let (url, _) = hooks_url(&stored(), "897362094", "03f5v9zk").unwrap();
        assert_eq!(
            url,
            "https://app.fizzy.do/897362094/boards/03f5v9zk/webhooks"
        );
    }

    #[test]
    fn every_action_is_subscribed_to() {
        assert_eq!(ACTIONS.len(), 11);
        assert!(ACTIONS.contains(&"comment_created"));
    }
}
