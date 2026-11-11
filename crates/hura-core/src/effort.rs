//! The reasoning effort a new sandbox's agent starts at: the server user's own.
//!
//! Claude Code keeps a default effort in its user settings: `effortLevel` for
//! every model, and `modelSettings.<model>.effortLevel` for one model, which
//! wins for that model. `/effort` is what writes them. A sandbox has a `HOME`
//! of its own and the image's `settings.json`, which says nothing about
//! effort, so without this every session would start at the model's default,
//! whatever the server user has chosen for themselves.
//!
//! So the seeder copies those keys in, the way it copies skills: once, when
//! the sandbox is made, from whatever the host's file says at that moment. A
//! later `/effort` on the host is the next session's default, not this one's,
//! and a choice made in the session wins over it as it would on the host.
//!
//! Only those keys. The rest of the host's settings (its hooks, permissions,
//! plugins, status line) describe a different machine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::seed::sh_quote;

/// The levels Claude Code keeps in a settings file. `max` is not one: it lasts
/// a session, and Claude Code drops it from a settings file, so it is dropped
/// here too rather than carried in to be ignored.
const LEVELS: [&str; 4] = ["low", "medium", "high", "xhigh"];

/// The agent's user settings in a sandbox, as the image writes them.
pub const SANDBOX_SETTINGS: &str = "/home/agent/.claude/settings.json";

/// The effort keys of one settings file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Effort {
    /// `effortLevel`: every model's, unless it has one of its own.
    pub level: Option<String>,
    /// `modelSettings.<model>.effortLevel`, keyed as the file keys them.
    pub models: BTreeMap<String, String>,
}

impl Effort {
    pub fn is_empty(&self) -> bool {
        self.level.is_none() && self.models.is_empty()
    }

    /// The effort keys of a settings file's text. A level Claude Code would
    /// not take is left out, as Claude Code leaves it out; a file that is not
    /// a JSON object is an error, since that is a file Claude Code cannot read
    /// either.
    pub fn parse(text: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let Some(top) = value.as_object() else {
            return Err("not a JSON object".to_string());
        };
        let level = |v: Option<&serde_json::Value>| {
            v.and_then(|l| l.as_str())
                .filter(|l| LEVELS.contains(l))
                .map(str::to_string)
        };
        let models = top
            .get("modelSettings")
            .and_then(|m| m.as_object())
            .into_iter()
            .flatten()
            .filter_map(|(model, s)| Some((model.clone(), level(s.get("effortLevel"))?)))
            .collect();
        Ok(Effort {
            level: level(top.get("effortLevel")),
            models,
        })
    }

    /// The part of a settings file these keys are.
    fn fragment(&self) -> serde_json::Value {
        let mut out = serde_json::Map::new();
        if let Some(level) = &self.level {
            out.insert("effortLevel".into(), level.clone().into());
        }
        if !self.models.is_empty() {
            let models = self
                .models
                .iter()
                .map(|(m, l)| (m.clone(), serde_json::json!({ "effortLevel": l })))
                .collect();
            out.insert("modelSettings".into(), serde_json::Value::Object(models));
        }
        serde_json::Value::Object(out)
    }

    /// The seeder's effort step: these keys merged into the sandbox agent's
    /// settings. Empty when there is nothing to carry.
    pub fn script(&self) -> String {
        self.script_for(Path::new(SANDBOX_SETTINGS))
    }

    /// [`Effort::script`] against any file, which is what lets a test run it.
    ///
    /// A merge rather than a write, because the file is the image's and holds
    /// everything else the agent needs; `*` merges objects all the way down,
    /// so a model's other settings, if the image ever has any, stay. Through a
    /// temporary file, since `jq` reading and writing one file empties it.
    fn script_for(&self, settings: &Path) -> String {
        if self.is_empty() {
            return String::new();
        }
        let f = sh_quote(&settings.display().to_string());
        let add = sh_quote(&self.fragment().to_string());
        format!(
            "[ -f {f} ] || printf '{{}}\\n' > {f}\n\
             jq --argjson add {add} '. * $add' {f} > {f}.hura\n\
             mv {f}.hura {f}\n"
        )
    }
}

/// Where the server user's Claude Code keeps its user settings:
/// `$CLAUDE_CONFIG_DIR/settings.json`, else `~/.claude/settings.json`. The
/// same two places [`crate::skills::host_skills_dir`] looks.
pub fn host_settings() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(dir).join("settings.json");
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join(".claude").join("settings.json")
}

/// The server user's effort keys. No file is no keys, which is an ordinary
/// machine; a file that cannot be read is an error, for a warning.
pub fn host() -> Result<Effort, String> {
    let path = host_settings();
    match std::fs::read_to_string(&path) {
        Ok(text) => Effort::parse(&text).map_err(|e| {
            format!(
                "{}: {e}; the session starts at the model's default effort",
                path.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Effort::default()),
        Err(e) => Err(format!(
            "could not read {}: {e}; the session starts at the model's default effort",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_keys_are_read_and_nothing_else() {
        let e = Effort::parse(
            r#"{"model":"opus[1m]","effortLevel":"high","hooks":{},
                "modelSettings":{"claude-opus-5-5":{"effortLevel":"xhigh","autoCompactWindow":"auto"},
                                 "claude-sonnet-5-5":{"autoCompactWindow":"auto"}}}"#,
        )
        .unwrap();
        assert_eq!(e.level.as_deref(), Some("high"));
        assert_eq!(
            e.models,
            BTreeMap::from([("claude-opus-5-5".to_string(), "xhigh".to_string())])
        );
        assert_eq!(
            e.fragment(),
            serde_json::json!({
                "effortLevel": "high",
                "modelSettings": {"claude-opus-5-5": {"effortLevel": "xhigh"}}
            }),
            "a model's other settings stay on the host"
        );
    }

    #[test]
    fn a_level_claude_code_would_not_keep_is_left_out() {
        let e = Effort::parse(
            r#"{"effortLevel":"max","modelSettings":{"claude-opus-5-5":{"effortLevel":"extreme"}}}"#,
        )
        .unwrap();
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(e.script(), "", "nothing to carry is no step");
    }

    #[test]
    fn a_file_claude_code_cannot_read_is_an_error() {
        assert!(Effort::parse("{").is_err());
        assert!(Effort::parse("[]").is_err());
        assert_eq!(Effort::parse("{}").unwrap(), Effort::default());
    }

    /// The merge, run for real: the image's keys stay, the effort keys land,
    /// and a model's own settings beside them survive.
    #[test]
    fn the_script_merges_into_the_settings_it_finds() {
        if std::process::Command::new("jq")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipped: no jq here");
            return;
        }
        let dir = std::env::temp_dir().join(format!("hura-effort-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("it's settings.json");
        std::fs::write(
            &file,
            r#"{"model":"opus[1m]","modelSettings":{"claude-opus-5-5":{"autoCompactWindow":"auto"}}}"#,
        )
        .unwrap();
        let e = Effort {
            level: Some("high".into()),
            models: BTreeMap::from([("claude-opus-5-5".into(), "xhigh".into())]),
        };
        let run = |script: &str| {
            let out = std::process::Command::new("sh")
                .args(["-eu", "-c", script])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&e.script_for(&file));
        let merged: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(
            merged,
            serde_json::json!({
                "model": "opus[1m]",
                "effortLevel": "high",
                "modelSettings": {"claude-opus-5-5": {"autoCompactWindow": "auto", "effortLevel": "xhigh"}}
            })
        );

        // And a sandbox with no settings file yet gets one.
        std::fs::remove_file(&file).unwrap();
        run(&e.script_for(&file));
        assert_eq!(
            Effort::parse(&std::fs::read_to_string(&file).unwrap()).unwrap(),
            e
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
