//! hura cloud's answers, as a client shows them: the account, and the devices
//! signed in to it. The service itself is `crates/hura-cloud`; the client
//! that asks it is `hura_client::cloud`. Here so the desktop's TypeScript is
//! generated from them like everything else on its bridge.

use serde::{Deserialize, Serialize};

/// The account a device is signed in to.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "CloudAccount"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub account: String,
    pub plan: String,
    pub sign_ins: Vec<SignIn>,
}

/// One way into the account.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "CloudSignIn"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignIn {
    /// `email` or `apple`.
    pub provider: String,
    pub email: Option<String>,
}

/// A device signed in to the account.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "CloudDevice"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// `ios`, `macos`, `windows`, `linux`.
    pub platform: String,
    /// Unix seconds.
    pub created_at: f64,
    /// When it last renewed its access token, in Unix seconds: about when it
    /// was last used.
    pub last_seen_at: f64,
    /// The device asking.
    pub current: bool,
}

/// Whether this machine is signed in, and as whom: what the window shows
/// without asking the service.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "CloudStatus"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub email: String,
}
