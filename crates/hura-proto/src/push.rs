//! Notifications on a phone: what a client tells `hurad` so the server can
//! reach it through the hura relay, and what it is told back.
//!
//! The relay (`docs/relay-api.md`) holds the APNs key, and an `hurad` holds
//! only a push token that can make one phone show a notification and nothing
//! more. The phone app registers with the relay, links this server to itself,
//! and hands the link's token over with [`PushRoute`].
//!
//! **A route belongs to the connection's token, not to the server.** Each
//! paired client configures its own, and revoking the pairing removes it: a
//! phone that is no longer allowed to drive this server is no longer told what
//! its sessions are doing either.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Where this connection's notifications go. What [`crate::Request::PushConfigure`]
/// carries.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRoute {
    /// `https`, or plain `http` on a loopback address for development.
    pub relay_url: String,
    /// `hup_…`, from the relay's `POST /v1/servers`. Checked with the relay
    /// before it is kept.
    pub push_token: String,
    /// 32 random bytes, base64url without padding, that the client made for
    /// this route. With one, notification text is encrypted end to end and
    /// the relay sees only the kind.
    #[serde(default)]
    pub push_key: Option<String>,
    /// The `h_…` id the client linked this server under with the relay. An
    /// `hurad` has no id of its own, so the client says which it chose. It is
    /// part of what an encrypted notification is bound to, so it has to be
    /// the one the relay will put in the payload.
    pub server_id: String,
    /// Kinds switched off. Missing kinds are on.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, boolean>"))]
    pub prefs: BTreeMap<String, bool>,
}

/// Written by hand so the push token and key never reach a log line through a
/// `{request:?}`. The token is a login to somebody's phone.
impl std::fmt::Debug for PushRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushRoute")
            .field("relay_url", &self.relay_url)
            .field("push_token", &"…")
            .field("push_key", &self.push_key.as_ref().map(|_| "…"))
            .field("server_id", &self.server_id)
            .field("prefs", &self.prefs)
            .finish()
    }
}

/// This connection's route as the server holds it, without its token.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushView {
    /// False when this connection has no route; everything else is then empty.
    pub configured: bool,
    pub relay_url: String,
    /// The label the relay holds for the link, which is what the phone shows when
    /// it cannot decrypt.
    pub label: String,
    /// Every kind a client can switch, and whether it is on.
    #[cfg_attr(feature = "ts", ts(type = "Record<string, boolean>"))]
    pub prefs: BTreeMap<String, bool>,
    pub encrypted: bool,
    /// Why the route has stopped, when the relay rejected its token after it was
    /// configured. Configuring again repairs it.
    pub broken: Option<String>,
    /// Epoch seconds of the last notification Radix took.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_success_at: Option<u64>,
    pub last_error: Option<String>,
    /// Notifications waiting to be delivered.
    pub queued: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_without_key_or_prefs_reads() {
        let r: PushRoute = serde_json::from_str(
            r#"{"relay_url":"https://r.example","push_token":"hup_x","server_id":"h_aaaabbbbccccdddd"}"#,
        )
        .unwrap();
        assert_eq!(r.push_key, None);
        assert!(r.prefs.is_empty());
    }

    #[test]
    fn debug_never_shows_the_token_or_the_key() {
        let r = PushRoute {
            relay_url: "https://r.example".into(),
            push_token: "hup_secret".into(),
            push_key: Some("keymaterial".into()),
            server_id: "h_aaaabbbbccccdddd".into(),
            prefs: BTreeMap::new(),
        };
        let shown = format!("{r:?}");
        assert!(!shown.contains("hup_secret"), "{shown}");
        assert!(!shown.contains("keymaterial"), "{shown}");
    }
}
