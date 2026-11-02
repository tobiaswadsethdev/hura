//! The .NET SDKs a checkout pins, and whether an image already has them.
//!
//! A `global.json` names the SDK a repository builds with, and its
//! `rollForward` says how far from that version the `dotnet` host may go to
//! find one. `disable` goes nowhere: the exact version is the only one that
//! will do, and a repository pinned that way fails every command, `dotnet
//! --version` included, on an image whose SDK is one patch newer. So the dotnet
//! variant carries the newest SDK, and a session whose checkout pins one it does
//! not satisfy is given that exact SDK as well, in a layer on top of the
//! variant (see [`crate::image::ensure`]).
//!
//! Read from the local checkouts of the session's remote, the way
//! [`crate::toolchain::detect`] reads its markers, and with the same caveat:
//! the sandbox clones `origin`, and a pin changed but not pushed is read here
//! all the same. In that direction the cost is an SDK nothing uses.

use std::path::{Path, PathBuf};

use crate::repos;
use crate::toolchain;

/// One `global.json`'s SDK requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub version: String,
    pub roll_forward: RollForward,
}

/// How far from its version a pin lets the host go, as `rollForward` says.
///
/// The `latest*` policies and their plain namesakes pick differently among
/// the SDKs installed, but accept the same ones, and which ones are acceptable
/// is the only question asked here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollForward {
    /// A newer patch in the same feature band: `patch`, `latestPatch`, and
    /// what a pin with no `rollForward` gets.
    Patch,
    /// A newer feature band of the same major and minor.
    Feature,
    /// A newer minor of the same major.
    Minor,
    /// Anything newer.
    Major,
    /// Exactly the version named. Also what a value this does not know is
    /// taken to mean, since the exact version satisfies every policy there is.
    Disable,
}

impl RollForward {
    fn parse(value: Option<&str>) -> Self {
        match value.map(str::to_ascii_lowercase).as_deref() {
            None | Some("patch" | "latestpatch") => Self::Patch,
            Some("feature" | "latestfeature") => Self::Feature,
            Some("minor" | "latestminor") => Self::Minor,
            Some("major" | "latestmajor") => Self::Major,
            Some(_) => Self::Disable,
        }
    }
}

/// An SDK version as `(major, minor, feature band and patch)`, the last as the
/// one number it is written as: `10.0.401` is band 4, patch 1. `None` for a
/// prerelease, which only an exact pin can ask for.
fn numbers(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.split('.');
    let parsed = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(parsed)
}

/// Whether a string is an SDK version and nothing else.
///
/// It reaches a `RUN` line, from a file in a repository, so this is the check
/// that keeps a `global.json` from being a way to run a command at build time.
fn valid(version: &str) -> bool {
    let (release, pre) = version.split_once('-').unwrap_or((version, ""));
    numbers(release).is_some()
        && pre
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

impl Pin {
    /// Whether any of these installed SDKs is one the pin accepts.
    pub fn satisfied_by(&self, installed: &[String]) -> bool {
        if installed.contains(&self.version) {
            return true;
        }
        let Some(want) = numbers(&self.version) else {
            return false;
        };
        // The feature band is the hundreds of the third number.
        let same_band = |a: (u32, u32, u32), b: (u32, u32, u32)| {
            a.0 == b.0 && a.1 == b.1 && a.2 / 100 == b.2 / 100
        };
        installed.iter().filter_map(|v| numbers(v)).any(|have| {
            have > want
                && match self.roll_forward {
                    RollForward::Patch => same_band(have, want),
                    RollForward::Feature => have.0 == want.0 && have.1 == want.1,
                    RollForward::Minor => have.0 == want.0,
                    RollForward::Major => true,
                    RollForward::Disable => false,
                }
        })
    }
}

/// Read one `global.json`. `None` when it pins nothing, which is when any SDK
/// will do, and when what it pins is not a version.
pub fn parse(text: &str) -> Option<Pin> {
    let json: serde_json::Value = serde_json::from_str(&strip_comments(text)).ok()?;
    let sdk = json.get("sdk")?;
    let version = sdk.get("version")?.as_str()?.trim().to_string();
    if !valid(&version) {
        return None;
    }
    Some(Pin {
        version,
        roll_forward: RollForward::parse(sdk.get("rollForward").and_then(|v| v.as_str())),
    })
}

/// `//` and `/* */` comments removed, since the `dotnet` host accepts them in
/// a `global.json` and a JSON parser does not. Strings are left alone, so a URL
/// in one survives.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// Every pin in a checkout: its root and one level under it, where
/// [`toolchain::detect`] looks for the solution a `global.json` sits beside.
pub fn pins(root: &Path) -> Vec<Pin> {
    let mut found: Vec<Pin> = Vec::new();
    toolchain::walk(root, |path, name| {
        if name != "global.json" {
            return;
        }
        let pin = std::fs::read_to_string(path)
            .ok()
            .as_deref()
            .and_then(parse);
        if let Some(pin) = pin.filter(|p| !found.contains(p)) {
            found.push(pin);
        }
    });
    found
}

/// The pins of every checkout on this machine whose `origin` is `remote`.
///
/// Every one rather than the first, because a repository checked out twice is
/// usually two branches of it, and either may be what the session is for.
pub fn pins_for(remote: &str, roots: &[repos::Root]) -> Vec<Pin> {
    let mut found: Vec<Pin> = Vec::new();
    let checkouts: Vec<PathBuf> = repos::discover_in(roots)
        .into_iter()
        .filter(|r| r.origin.as_deref().is_some_and(|o| same_remote(o, remote)))
        .map(|r| r.path)
        .collect();
    for checkout in checkouts {
        for pin in pins(&checkout) {
            if !found.contains(&pin) {
                found.push(pin);
            }
        }
    }
    found
}

/// Whether two remote URLs name one repository: alike once the scheme, any
/// user in front of the host, a trailing `.git` and a trailing slash are gone.
/// Azure DevOps puts the organisation in front of the host in the URL it hands
/// out to clone with, and a URL typed by hand usually leaves it off.
fn same_remote(a: &str, b: &str) -> bool {
    fn bare(url: &str) -> String {
        let url = url.trim();
        let rest = url.split_once("://").map_or(url, |(_, r)| r);
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let path = path.trim_end_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        format!("{}/{path}", host.to_ascii_lowercase())
    }
    bare(a) == bare(b)
}

/// The exact SDKs to install so every pin is met, given what is installed.
///
/// Exact pins first, because an SDK installed for one of those may meet a
/// looser pin as well, and installing the looser pin's own version beside it
/// would be an SDK nothing needs.
pub fn missing(pins: &[Pin], installed: &[String]) -> Vec<String> {
    let mut have: Vec<String> = installed.to_vec();
    let mut add: Vec<String> = Vec::new();
    let exact_first = pins
        .iter()
        .filter(|p| p.roll_forward == RollForward::Disable)
        .chain(
            pins.iter()
                .filter(|p| p.roll_forward != RollForward::Disable),
        );
    for pin in exact_first {
        if !pin.satisfied_by(&have) {
            have.push(pin.version.clone());
            add.push(pin.version.clone());
        }
    }
    add.sort();
    add
}

/// The Dockerfile that adds `versions` to the image `tag`.
///
/// `FROM` the tag it then replaces, so the SDKs arrive as one more layer on an
/// image that is otherwise unchanged, and the next pin this does not meet is
/// one more again. The installer comes from the context beside it, not from the
/// image, so an image built before the installer existed can be added to too.
pub fn layer(tag: &str, versions: &[String]) -> String {
    debug_assert!(versions.iter().all(|v| valid(v)));
    format!(
        "# syntax=docker/dockerfile:1\n\
         #\n\
         # Generated by hura: the .NET SDKs a checkout's global.json pins, onto {tag}.\n\
         FROM {tag}\n\
         \n\
         USER root\n\
         \n\
         COPY {script} /usr/local/lib/hura/{script}\n\
         RUN sh /usr/local/lib/hura/{script} {versions}\n\
         \n\
         USER agent\n",
        script = toolchain::DOTNET_SDK_SCRIPT,
        versions = versions.join(" "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(version: &str, roll_forward: RollForward) -> Pin {
        Pin {
            version: version.into(),
            roll_forward,
        }
    }

    fn sdks(versions: &[&str]) -> Vec<String> {
        versions.iter().map(|v| v.to_string()).collect()
    }

    /// The shape every .NET repository on the machine this was written on has:
    /// an exact SDK, and no rolling forward from it.
    #[test]
    fn a_global_json_is_read_for_its_pin() {
        let text = r#"{
          // the version CI builds with
          "sdk": { "version": "10.0.401", "rollForward": "disable" } /* exact */
        }"#;
        assert_eq!(parse(text), Some(pin("10.0.401", RollForward::Disable)));

        // No `rollForward` is `latestPatch`, which accepts the same SDKs `patch`
        // does.
        assert_eq!(
            parse(r#"{"sdk": {"version": "10.0.300"}}"#),
            Some(pin("10.0.300", RollForward::Patch))
        );
        assert_eq!(
            parse(r#"{"sdk": {"version": "8.0.100", "rollForward": "latestMajor"}}"#),
            Some(pin("8.0.100", RollForward::Major))
        );
        // A comment marker inside a string is part of the string.
        assert_eq!(
            parse(
                r#"{"msbuild-sdks": {"x": "http://example.com"}, "sdk": {"version": "9.0.100"}}"#
            ),
            Some(pin("9.0.100", RollForward::Patch))
        );

        // Any SDK will do, so there is nothing to install.
        assert_eq!(parse(r#"{"sdk": {"rollForward": "latestMajor"}}"#), None);
        assert_eq!(parse(r#"{"msbuild-sdks": {}}"#), None);
        assert_eq!(parse("not json"), None);
    }

    /// The version reaches a `RUN` line, so anything but a version is dropped.
    #[test]
    fn a_pin_that_is_not_a_version_is_not_one() {
        assert_eq!(parse(r#"{"sdk": {"version": "10.0.401; rm -rf /"}}"#), None);
        assert_eq!(parse(r#"{"sdk": {"version": "$(id)"}}"#), None);
        assert_eq!(parse(r#"{"sdk": {"version": "10.0"}}"#), None);
        assert_eq!(
            parse(r#"{"sdk": {"version": "11.0.100-rc.2.25502.107", "rollForward": "disable"}}"#),
            Some(pin("11.0.100-rc.2.25502.107", RollForward::Disable))
        );
    }

    #[test]
    fn roll_forward_decides_which_installed_sdks_will_do() {
        let newest = sdks(&["10.0.403"]);

        assert!(!pin("10.0.401", RollForward::Disable).satisfied_by(&newest));
        assert!(pin("10.0.403", RollForward::Disable).satisfied_by(&newest));

        // A newer patch in the same band, and not a newer band.
        assert!(pin("10.0.400", RollForward::Patch).satisfied_by(&newest));
        assert!(!pin("10.0.300", RollForward::Patch).satisfied_by(&newest));
        // An older patch is not rolling forward.
        assert!(!pin("10.0.405", RollForward::Patch).satisfied_by(&newest));

        assert!(pin("10.0.300", RollForward::Feature).satisfied_by(&newest));
        assert!(!pin("9.0.300", RollForward::Feature).satisfied_by(&newest));
        assert!(!pin("9.0.300", RollForward::Minor).satisfied_by(&newest));
        assert!(pin("9.0.300", RollForward::Major).satisfied_by(&newest));

        // A prerelease is only ever met exactly.
        assert!(!pin("11.0.100-rc.2", RollForward::Major).satisfied_by(&newest));
    }

    #[test]
    fn only_the_sdks_no_installed_one_meets_are_added() {
        let installed = sdks(&["10.0.403"]);
        let pins = [
            pin("10.0.400", RollForward::Patch),
            pin("10.0.401", RollForward::Disable),
            pin("10.0.301", RollForward::Disable),
            pin("10.0.403", RollForward::Disable),
        ];
        // 10.0.401 is added for its exact pin, and then meets the `latestPatch`
        // one too, so 10.0.400 is not added beside it.
        assert_eq!(missing(&pins, &installed), ["10.0.301", "10.0.401"]);
        assert!(missing(&pins, &sdks(&["10.0.403", "10.0.401", "10.0.301"])).is_empty());
        assert!(missing(&[], &installed).is_empty());
    }

    #[test]
    fn a_remote_is_recognised_however_it_is_written() {
        let azure = "https://org@dev.azure.com/org/Project/_git/Service";
        assert!(same_remote(
            azure,
            "https://dev.azure.com/org/Project/_git/Service"
        ));
        assert!(same_remote(
            azure,
            "https://dev.azure.com/org/Project/_git/Service/"
        ));
        assert!(same_remote(
            "https://github.com/o/r.git",
            "https://GitHub.com/o/r"
        ));
        assert!(!same_remote(
            azure,
            "https://dev.azure.com/org/Project/_git/Admin"
        ));
    }

    #[test]
    fn a_checkout_is_read_where_its_solutions_are() {
        let root = std::env::temp_dir().join(format!("hura-pins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, version: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                path,
                format!(r#"{{"sdk": {{"version": "{version}", "rollForward": "disable"}}}}"#),
            )
            .unwrap();
        };
        write("global.json", "10.0.401");
        write("Charts/global.json", "10.0.202");
        // Twice is once.
        write("Other/global.json", "10.0.202");
        // Dependencies and output are not the repository.
        write("node_modules/thing/global.json", "6.0.100");
        write("a/b/global.json", "7.0.100");

        let mut got: Vec<String> = pins(&root).into_iter().map(|p| p.version).collect();
        got.sort();
        assert_eq!(got, ["10.0.202", "10.0.401"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_layer_adds_onto_the_tag_and_drops_privileges() {
        let df = layer("hura-sandbox:dotnet-node", &sdks(&["10.0.301", "10.0.401"]));
        assert!(df.contains("FROM hura-sandbox:dotnet-node\n"), "{df}");
        assert!(df.contains("COPY dotnet-sdk "), "{df}");
        assert!(df.contains("dotnet-sdk 10.0.301 10.0.401\n"), "{df}");
        assert!(df.trim_end().ends_with("USER agent"), "{df}");
    }
}
