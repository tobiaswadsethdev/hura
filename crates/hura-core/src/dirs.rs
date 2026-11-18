//! Where hura keeps things on this machine: one answer per kind of thing, for
//! every platform.
//!
//! Three directories, because they are three different promises:
//!
//! * **config**, what a person edits and may sync between machines:
//!   `config.toml`, the session cache and what is derived beside it.
//! * **state**, what this machine generated and must not follow anyone
//!   elsewhere: the server's key, its tokens, saved connections. See
//!   [`crate::state`] for why that matters.
//! * **data**, work that cannot be regenerated: worktree sessions' working
//!   copies and the skill library clients upload.
//!
//! `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_DATA_HOME` win wherever they
//! are set, Windows included. That is how a development server is kept apart
//! from the real one on the same account, and it would be a strange rule that
//! worked on one platform only. Unset, Unix follows the XDG defaults under
//! `$HOME`, and Windows the places its own applications use:
//!
//! | | Unix | Windows |
//! | --- | --- | --- |
//! | config | `~/.config/hura` | `%APPDATA%\hura` |
//! | state | `~/.local/state/hura` | `%LOCALAPPDATA%\hura` |
//! | data | `~/.local/share/hura` | `%LOCALAPPDATA%\hura\data` |
//!
//! `AppData\Roaming` for config because it is the half of a Windows profile
//! that follows a person to another machine, which is the property config has
//! on Unix. State is `AppData\Local` because a pairing token is a login to one
//! host, and it was already there before a Windows server existed: the desktop
//! application keeps its paired servers in it. Data sits under state's
//! directory in `data\` rather than beside it, because the two would otherwise
//! share a directory with a `worktrees` in each: records in state, working
//! copies in data.

use std::ffi::OsString;
use std::path::PathBuf;

/// The current user's home directory.
///
/// `$HOME` on Unix. `%USERPROFILE%` on Windows, where `HOME` is usually unset
/// (Git Bash sets it, to the same directory), falling back to `HOME` for the
/// shells that do.
pub fn home() -> Option<PathBuf> {
    home_in(Platform::current(), &env)
}

/// hura's config directory. See the module comment for which.
pub fn config() -> PathBuf {
    resolve(Kind::Config, Platform::current(), &env)
}

/// hura's state directory. See the module comment for which.
pub fn state() -> PathBuf {
    resolve(Kind::State, Platform::current(), &env)
}

/// hura's data directory. See the module comment for which.
pub fn data() -> PathBuf {
    resolve(Kind::Data, Platform::current(), &env)
}

/// `~` and `~/...` (and `~\...` on Windows) against [`home`], which is how
/// anyone writes a home-relative path in a config file, with no shell around
/// to expand it. `~other/...` is another user's home and is left alone.
pub fn expand_tilde(path: &std::path::Path) -> PathBuf {
    let Some(rest) = path.to_str().and_then(|s| s.strip_prefix('~')) else {
        return path.to_path_buf();
    };
    let Some(home) = home() else {
        return path.to_path_buf();
    };
    let tail = rest.strip_prefix('/').or_else(|| {
        if cfg!(windows) {
            rest.strip_prefix('\\')
        } else {
            None
        }
    });
    match tail {
        Some(tail) => home.join(tail),
        None if rest.is_empty() => home,
        None => path.to_path_buf(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Unix,
    Windows,
}

impl Platform {
    fn current() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Unix
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Config,
    State,
    Data,
}

fn env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// A variable that is set to something. An empty `HOME` is no home, not the
/// current directory.
fn var(get: &dyn Fn(&str) -> Option<OsString>, name: &str) -> Option<PathBuf> {
    get(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn home_in(platform: Platform, get: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    match platform {
        Platform::Unix => var(get, "HOME"),
        Platform::Windows => var(get, "USERPROFILE").or_else(|| var(get, "HOME")),
    }
}

/// Which directory `kind` is, with the platform and the environment handed in
/// so that both platforms' answers can be tested on either.
fn resolve(kind: Kind, platform: Platform, get: &dyn Fn(&str) -> Option<OsString>) -> PathBuf {
    let xdg = match kind {
        Kind::Config => "XDG_CONFIG_HOME",
        Kind::State => "XDG_STATE_HOME",
        Kind::Data => "XDG_DATA_HOME",
    };
    if let Some(dir) = var(get, xdg) {
        return dir.join("hura");
    }
    let home = || home_in(platform, get).unwrap_or_else(|| PathBuf::from("."));
    match platform {
        Platform::Unix => match kind {
            Kind::Config => home().join(".config").join("hura"),
            Kind::State => home().join(".local").join("state").join("hura"),
            Kind::Data => home().join(".local").join("share").join("hura"),
        },
        Platform::Windows => {
            let local =
                || var(get, "LOCALAPPDATA").unwrap_or_else(|| home().join("AppData").join("Local"));
            match kind {
                Kind::Config => var(get, "APPDATA")
                    .unwrap_or_else(|| home().join("AppData").join("Roaming"))
                    .join("hura"),
                Kind::State => local().join("hura"),
                Kind::Data => local().join("hura").join("data"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn unix_follows_the_xdg_defaults_under_home() {
        let get = with(&[("HOME", "/home/u")]);
        let at = |k| resolve(k, Platform::Unix, &get);
        assert_eq!(at(Kind::Config), PathBuf::from("/home/u/.config/hura"));
        assert_eq!(at(Kind::State), PathBuf::from("/home/u/.local/state/hura"));
        assert_eq!(at(Kind::Data), PathBuf::from("/home/u/.local/share/hura"));
    }

    #[test]
    fn windows_uses_the_profile_s_own_directories() {
        let get = with(&[
            ("USERPROFILE", r"C:\Users\u"),
            ("APPDATA", r"C:\Users\u\AppData\Roaming"),
            ("LOCALAPPDATA", r"C:\Users\u\AppData\Local"),
        ]);
        let at = |k| resolve(k, Platform::Windows, &get);
        assert_eq!(
            at(Kind::Config),
            PathBuf::from(r"C:\Users\u\AppData\Roaming").join("hura")
        );
        assert_eq!(
            at(Kind::State),
            PathBuf::from(r"C:\Users\u\AppData\Local").join("hura")
        );
        assert_eq!(
            at(Kind::Data),
            PathBuf::from(r"C:\Users\u\AppData\Local")
                .join("hura")
                .join("data")
        );
        assert_eq!(
            home_in(Platform::Windows, &get),
            Some(PathBuf::from(r"C:\Users\u"))
        );
    }

    /// A Windows worktree session's record lives in state's `worktrees` and
    /// its working copy in data's: the two must not be one directory.
    #[test]
    fn windows_state_and_data_do_not_overlap() {
        let get = with(&[("LOCALAPPDATA", r"C:\L")]);
        let state = resolve(Kind::State, Platform::Windows, &get);
        let data = resolve(Kind::Data, Platform::Windows, &get);
        assert_ne!(state.join("worktrees"), data.join("worktrees"));
    }

    #[test]
    fn xdg_wins_on_every_platform() {
        let get = with(&[
            ("XDG_CONFIG_HOME", "/x/config"),
            ("XDG_STATE_HOME", "/x/state"),
            ("XDG_DATA_HOME", "/x/data"),
            ("HOME", "/home/u"),
            ("APPDATA", r"C:\R"),
            ("LOCALAPPDATA", r"C:\L"),
        ]);
        for platform in [Platform::Unix, Platform::Windows] {
            assert_eq!(
                resolve(Kind::Config, platform, &get),
                PathBuf::from("/x/config/hura")
            );
            assert_eq!(
                resolve(Kind::State, platform, &get),
                PathBuf::from("/x/state/hura")
            );
            assert_eq!(
                resolve(Kind::Data, platform, &get),
                PathBuf::from("/x/data/hura")
            );
        }
    }

    #[test]
    fn an_empty_variable_is_not_a_directory() {
        let get = with(&[("XDG_CONFIG_HOME", ""), ("HOME", "/home/u")]);
        assert_eq!(
            resolve(Kind::Config, Platform::Unix, &get),
            PathBuf::from("/home/u/.config/hura")
        );
        assert_eq!(home_in(Platform::Unix, &with(&[("HOME", "")])), None);
    }

    #[test]
    fn windows_falls_back_to_home_when_the_profile_is_not_set() {
        let get = with(&[("HOME", "/c/Users/u")]);
        assert_eq!(
            home_in(Platform::Windows, &get),
            Some(PathBuf::from("/c/Users/u"))
        );
        assert_eq!(
            home_in(Platform::Unix, &with(&[("USERPROFILE", "x")])),
            None
        );
    }
}
