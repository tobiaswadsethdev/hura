//! The trackers this machine reads tickets from, and the tokens for them.
//!
//! Kept on the client, beside the paired servers, and never sent to one. A
//! server is where sandboxes run; a Jira or Azure DevOps token is a login to
//! somebody's tickets, and it has no business on the machine that hosts agents.
//! So the desktop application stores a tracker here, reads it from here, and
//! asks the tracker itself -- see [`hura_core::tracker`] for the reading.
//!
//! One file, `trackers.json` in [`state::dir`], written owner-only like
//! `remotes.json`: it holds credentials. A webview is never handed a
//! [`Stored`]; it gets [`Configured`], which says whether there is a token and
//! nothing else.

use std::io;
use std::path::PathBuf;

use hura_core::state;
use hura_core::tracker::{Configured, Source, Stored};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Trackers {
    #[serde(default)]
    trackers: Vec<Stored>,
}

impl Trackers {
    pub fn default_path() -> PathBuf {
        state::dir().join("trackers.json")
    }

    pub fn load() -> io::Result<Self> {
        Self::load_from(Self::default_path())
    }

    pub fn load_from(path: impl Into<PathBuf>) -> io::Result<Self> {
        match std::fs::read_to_string(path.into()) {
            Ok(text) => serde_json::from_str(&text).map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(Self::default_path())
    }

    pub fn save_to(&self, path: impl Into<PathBuf>) -> io::Result<()> {
        let path = path.into();
        if let Some(dir) = path.parent() {
            state::private_dir(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        state::write_private(&path, &text)
    }

    /// Every tracker, tokens included. For reading tickets, never for a
    /// webview.
    pub fn list(&self) -> &[Stored] {
        &self.trackers
    }

    /// What a webview is shown.
    pub fn views(&self) -> Vec<Configured> {
        self.trackers.iter().map(Configured::from).collect()
    }

    /// Add one. Refused when it could not work -- a Jira entry with no site --
    /// or when its name is taken, because the name is what a session records
    /// the ticket came from.
    pub fn add(&mut self, source: &Source, token: Option<&str>) -> Result<(), String> {
        let source = checked(source)?;
        if self.trackers.iter().any(|t| t.source.name == source.name) {
            return Err(format!(
                "there is already a tracker called `{}`",
                source.name
            ));
        }
        self.trackers.push(Stored {
            source,
            token: token
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(String::from),
        });
        Ok(())
    }

    /// Replace one's entry, found by the name it has now. The token stays: an
    /// edited filter is not a reason to paste it again.
    pub fn update(&mut self, name: &str, source: &Source) -> Result<(), String> {
        let source = checked(source)?;
        if source.name != name && self.trackers.iter().any(|t| t.source.name == source.name) {
            return Err(format!(
                "there is already a tracker called `{}`",
                source.name
            ));
        }
        let entry = self.find(name)?;
        entry.source = source;
        Ok(())
    }

    /// Store or replace one's token.
    pub fn set_token(&mut self, name: &str, token: &str) -> Result<(), String> {
        let token = token.trim();
        if token.is_empty() {
            return Err("that token is empty".into());
        }
        self.find(name)?.token = Some(token.to_string());
        Ok(())
    }

    /// Take one away, token and all.
    pub fn forget(&mut self, name: &str) -> Result<(), String> {
        let before = self.trackers.len();
        self.trackers.retain(|t| t.source.name != name);
        if self.trackers.len() == before {
            return Err(format!("no tracker called `{name}`"));
        }
        Ok(())
    }

    fn find(&mut self, name: &str) -> Result<&mut Stored, String> {
        self.trackers
            .iter_mut()
            .find(|t| t.source.name == name)
            .ok_or_else(|| format!("no tracker called `{name}`"))
    }
}

/// Trimmed, named, and able to work -- or why not.
fn checked(source: &Source) -> Result<Source, String> {
    let source = source.normalized();
    match source.problem() {
        Some(problem) => Err(problem),
        None => Ok(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hura_core::tracker::{Filter, Kind};

    fn jira(name: &str) -> Source {
        Source {
            kind: Kind::Jira,
            name: name.into(),
            site: Some("https://example.atlassian.net".into()),
            email: Some("you@example.com".into()),
            ..Default::default()
        }
    }

    fn path(tag: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("hura-trackers-{tag}-{}", std::process::id()))
            .join("trackers.json")
    }

    /// What goes in comes back, and the file is the owner's alone: it holds
    /// the tokens.
    #[test]
    fn a_tracker_and_its_token_survive_the_file() {
        let path = path("round-trip");
        let mut t = Trackers::default();
        t.add(&jira("work"), Some("  tok  ")).unwrap();
        t.save_to(&path).unwrap();

        let back = Trackers::load_from(&path).unwrap();
        assert_eq!(back.list().len(), 1);
        assert_eq!(back.list()[0].token.as_deref(), Some("tok"), "trimmed");
        assert!(back.views()[0].token_set);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "mode was {mode:o}");
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Editing the filters keeps the token, and a rename cannot take a name
    /// another tracker has.
    #[test]
    fn an_edit_keeps_the_token_and_the_names_stay_distinct() {
        let mut t = Trackers::default();
        t.add(&jira("work"), Some("tok")).unwrap();
        t.add(&jira("home"), None).unwrap();

        let mut edited = jira("work");
        edited.filters = vec![Filter {
            name: "ready to start".into(),
            query: "status = Ready".into(),
        }];
        t.update("work", &edited).unwrap();
        assert_eq!(t.list()[0].token.as_deref(), Some("tok"));
        assert_eq!(t.list()[0].source.filters, edited.filters);

        assert!(t.update("work", &jira("home")).is_err(), "name taken");
        assert!(t.add(&jira("work"), None).is_err(), "name taken");
    }

    /// An entry that could not work is refused before it is kept, in the
    /// words the reader would have used.
    #[test]
    fn a_tracker_that_could_not_work_is_refused() {
        let mut t = Trackers::default();
        let no_site = Source {
            site: None,
            ..jira("work")
        };
        let err = t.add(&no_site, Some("tok")).unwrap_err();
        assert!(err.contains("site"), "{err}");
        assert!(t.list().is_empty());
    }

    #[test]
    fn a_token_is_set_and_a_tracker_forgotten_by_name() {
        let mut t = Trackers::default();
        t.add(&jira("work"), None).unwrap();
        assert!(!t.views()[0].token_set);
        assert!(
            t.set_token("work", "   ").is_err(),
            "an empty token is no token"
        );
        t.set_token("work", "new").unwrap();
        assert!(t.views()[0].token_set);
        assert!(t.set_token("nope", "x").is_err());
        t.forget("work").unwrap();
        assert!(t.list().is_empty());
        assert!(t.forget("work").is_err());
    }
}
