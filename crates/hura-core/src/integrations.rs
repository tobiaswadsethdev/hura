//! Everything the server holds on a session's behalf, in one answer.
//!
//! The MCP servers and what each is doing, the secret *names* the store has,
//! and the skills a client has uploaded. Three things that were three
//! documented procedures -- a `docker run` line, a `-e` argument and a path in a
//! config file -- and are now one screen with buttons on it.
//!
//! **One reply, re-read after every action.** The same decision the git view
//! made and for the same reason: starting a container, storing a secret or
//! uploading a skill each change what the others say -- a secret is what a
//! container was waiting for -- and a client adjusting the list it already had
//! would be a client inventing an answer. So every action here returns this
//! whole view, freshly asked.

use serde::{Deserialize, Serialize};

use crate::mcp;
use crate::secrets;
use crate::skills;

/// What the integrations screen shows.
// `Integrations` on the wire, because `policy::View` is already `View` -- and
// this collision was made and not noticed once already: the generated `Reply`
// carried `{ "reply": "integrations" } & View` pointing at the *policy* view,
// which is a shape mismatch a webview would have discovered at runtime.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, rename = "Integrations"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    /// The catalog, with what Docker says about each managed one.
    pub mcp: Vec<mcp::Status>,
    /// Every secret name: the ones the store holds and the ones a catalog entry
    /// asks for and it does not. Never a value.
    pub secrets: Vec<secrets::Named>,
    /// The skills a client has uploaded into the server's library.
    pub skills: Vec<skills::Stored>,
    /// The skills the server's own config file names by path, which are not
    /// uploads and cannot be removed from here.
    pub configured_skills: Vec<String>,
}

/// Ask the server everything, once.
///
/// Costs one `docker inspect` per managed entry, a file read and a directory
/// listing. Cheap enough to be the answer to every action, which is what makes
/// the screen honest.
pub fn view(cfg: &crate::config::Config) -> View {
    let entries = cfg.mcp();
    View {
        mcp: mcp::statuses(entries),
        secrets: named_secrets(entries),
        skills: skills::library(),
        configured_skills: cfg.skills().iter().map(|s| s.name.clone()).collect(),
    }
}

/// The store's names and the catalog's, merged.
///
/// A name a catalog entry asks for and the store does not have is the failure
/// this screen exists to make visible -- it is the reason a container exits on
/// startup -- so it is listed as itself, unset, rather than being absent. And a
/// stored name that nothing references is listed too: it is either a leftover
/// from an entry that has gone or a typo in one that has not, and both are worth
/// seeing.
fn named_secrets(entries: &[mcp::Entry]) -> Vec<secrets::Named> {
    named_from(entries, &secrets::names())
}

/// [`named_secrets`] against a store given rather than read, so a test sees
/// the store it set up and not whatever this machine happens to keep.
fn named_from(entries: &[mcp::Entry], stored: &[String]) -> Vec<secrets::Named> {
    let mut out: Vec<secrets::Named> = Vec::new();

    let mut note = |name: &str, used_by: Option<&str>| {
        if let Some(existing) = out.iter_mut().find(|n| n.name == name) {
            if let Some(user) = used_by
                && !existing.used_by.iter().any(|u| u == user)
            {
                existing.used_by.push(user.to_string());
            }
            return;
        }
        out.push(secrets::Named {
            name: name.to_string(),
            set: stored.iter().any(|s| s == name),
            used_by: used_by.map(|u| vec![u.to_string()]).unwrap_or_default(),
        });
    };

    for entry in entries {
        for name in entry.managed.iter().flat_map(|m| m.secrets.iter()) {
            note(name, Some(entry.name()));
        }
    }
    // Only when stored: naming does without a key, so an absent one is not
    // the missing-secret failure this list exists to point at.
    if stored.iter().any(|s| s == crate::naming::KEY) {
        note(crate::naming::KEY, Some("session naming"));
    }
    for name in stored {
        note(name, None);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn entry(name: &str, secrets: &[&str]) -> mcp::Entry {
        mcp::Entry::managed(
            name,
            mcp::Transport::Http,
            mcp::Managed {
                image: "an/image:1".into(),
                port: 9000,
                args: Vec::new(),
                env: BTreeMap::new(),
                secrets: secrets.iter().map(|s| (*s).to_string()).collect(),
            },
        )
        .unwrap()
    }

    /// A name two entries share is one secret used by both, not two rows -- and
    /// one nothing asks for is still shown, because a leftover and a typo look
    /// identical until you can see them side by side.
    #[test]
    fn the_secret_list_is_the_catalog_and_the_store_together() {
        let entries = [
            entry("jira", &["ATLASSIAN_TOKEN"]),
            entry("wiki", &["ATLASSIAN_TOKEN"]),
        ];
        let stored = ["LEFTOVER".to_string()];
        let named = named_from(&entries, &stored);

        assert_eq!(named.len(), 2, "{named:?}");
        assert_eq!(named[0].name, "ATLASSIAN_TOKEN");
        assert_eq!(named[0].used_by, ["jira", "wiki"]);
        // Asked for and missing, which is the state that explains a container
        // exiting.
        assert!(!named[0].set);
        assert_eq!(named[1].name, "LEFTOVER");
        assert!(named[1].set && named[1].used_by.is_empty());
    }

    /// The naming key is used by something even with no integration asking
    /// for it, and is only listed when it is stored.
    #[test]
    fn a_stored_naming_key_is_used_by_session_naming() {
        let stored = [crate::naming::KEY.to_string()];
        let named = named_from(&[], &stored);
        assert_eq!(named.len(), 1, "{named:?}");
        assert_eq!(named[0].used_by, ["session naming"]);
        assert!(named_from(&[], &[]).is_empty());
    }

    /// An external entry has no secrets of its own here: whoever runs it gave it
    /// whatever it needs, and this screen would be claiming otherwise.
    #[test]
    fn an_external_entry_contributes_no_secret_names() {
        let external = mcp::Entry::external(
            mcp::Server::parse("theirs", "http://mcp-theirs:9000/mcp", mcp::Transport::Http)
                .unwrap(),
        );
        let named = named_from(std::slice::from_ref(&external), &[]);
        assert!(named.iter().all(|n| n.used_by.is_empty()), "{named:?}");
    }
}
