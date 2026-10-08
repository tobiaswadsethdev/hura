//! The credentials a session can be given, and how each one reaches it.
//!
//! A credential is configured once, in `[credentials.NAME]`, as a *kind* and a
//! place the value comes from: a host command that prints it, or a 1Password
//! or AWS Secrets Manager reference. hura never reads the value. When a session
//! is created with a credential ticked, its sandbox gets a secret scoped to it
//! alone, made from that same reference, and the sandbox runtime resolves it
//! on the host.
//!
//! Inside the sandbox the credential is a placeholder: a random string in the
//! variable the tool reads (`CLAUDE_CODE_OAUTH_TOKEN`, `AZURE_DEVOPS_PAT`,
//! `GITHUB_TOKEN`). The runtime's proxy swaps it for the real value in the
//! headers of requests to that kind's hosts, and nowhere else. A session
//! created without the credential has neither the placeholder nor a secret
//! behind it, so ticking the box per session still decides something.

use std::io::Read as _;

use sbx_client::{SecretSource, SecretSpec};

use crate::seed::sh_quote;

/// What a credential is for, which decides where it is sent and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The long-lived token `claude setup-token` makes, backed by the Claude
    /// subscription rather than billed per token.
    ClaudeOauth,
    /// An Azure DevOps personal access token: git and the REST API.
    AzureDevOpsPat,
    /// A GitHub token: git over HTTPS and `gh`.
    GitHub,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::ClaudeOauth, Kind::AzureDevOpsPat, Kind::GitHub];

    /// The name the config file and the create form use. The same names the
    /// OpenShell provider profiles had, so a `providers` list and the form's
    /// preselection keep meaning the same thing.
    pub fn name(self) -> &'static str {
        match self {
            Kind::ClaudeOauth => "claude-code-oauth",
            Kind::AzureDevOpsPat => "azure-devops-pat",
            Kind::GitHub => "github",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// The hosts whose requests carry it.
    pub fn hosts(self) -> &'static [&'static str] {
        match self {
            Kind::ClaudeOauth => &["api.anthropic.com", "platform.claude.com"],
            Kind::AzureDevOpsPat => &["dev.azure.com"],
            Kind::GitHub => &["github.com", "api.github.com"],
        }
    }

    /// The variable the placeholder is put in: the one the tool reads.
    pub fn env(self) -> &'static str {
        match self {
            Kind::ClaudeOauth => "CLAUDE_CODE_OAUTH_TOKEN",
            Kind::AzureDevOpsPat => "AZURE_DEVOPS_PAT",
            Kind::GitHub => "GITHUB_TOKEN",
        }
    }

    /// How a placeholder for this kind starts. Shaped like the real thing for
    /// Claude, so a client that checks a token's prefix before sending it
    /// sends the placeholder rather than refusing it.
    fn prefix(self) -> &'static str {
        match self {
            Kind::ClaudeOauth => "sk-ant-oat01-hura",
            Kind::AzureDevOpsPat | Kind::GitHub => "hura-cred-",
        }
    }
}

/// Where a credential's value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A command run on the host, in a shell, whose stdout is the value.
    Command(String),
    /// A 1Password `op://` reference or an AWS Secrets Manager ARN.
    Ref(String),
}

/// One `[credentials.NAME]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub name: String,
    pub kind: Kind,
    pub source: Source,
}

impl Credential {
    /// The secret a sandbox is given for this credential, under `placeholder`.
    ///
    /// The value is what the proxy writes over the placeholder, so it has to
    /// be what belongs in the header. Claude and GitHub tokens go in as they
    /// are. An Azure DevOps PAT is HTTP Basic with the token as the password
    /// and an empty user, `base64(":" + pat)`: from a command, the command is
    /// wrapped so the runtime stores that form, and the sandbox sends
    /// `Authorization: Basic <placeholder>`. A reference cannot be wrapped,
    /// so for Azure DevOps it has to name the encoded form itself.
    pub fn secret(&self, sandbox: &str, placeholder: &str) -> SecretSpec {
        let source = match (&self.source, self.kind) {
            (Source::Command(cmd), Kind::AzureDevOpsPat) => SecretSource::Command(format!(
                "printf ':%s' \"$(sh -c {})\" | base64 -w0",
                sh_quote(cmd)
            )),
            (Source::Command(cmd), _) => SecretSource::Command(cmd.clone()),
            (Source::Ref(r), _) => SecretSource::Ref(r.clone()),
        };
        SecretSpec {
            sandbox: sandbox.to_string(),
            hosts: self.kind.hosts().iter().map(|h| (*h).to_string()).collect(),
            env: self.kind.env().to_string(),
            source,
            placeholder: Some(placeholder.to_string()),
        }
    }
}

/// A fresh placeholder for a credential of this kind.
///
/// Random rather than derived, so nothing outside the sandbox can work one
/// out, and long enough that it never turns up in a request by accident.
pub fn placeholder(kind: Kind) -> String {
    let mut bytes = [0u8; 12];
    // /dev/urandom does not fail on Linux once the system is up. If it ever
    // did, a placeholder made from the clock and the pid is still unique per
    // sandbox, which is what the runtime needs; the secret behind it is
    // scoped to the one sandbox either way.
    let random = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_ok();
    if !random {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let seed = now ^ u128::from(std::process::id());
        bytes.copy_from_slice(&seed.to_le_bytes()[..12]);
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}{hex}", kind.prefix())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_through_their_names() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.name()), Some(k));
        }
        assert_eq!(Kind::parse("nope"), None);
    }

    #[test]
    fn placeholders_are_fresh_and_shaped_by_kind() {
        let a = placeholder(Kind::ClaudeOauth);
        let b = placeholder(Kind::ClaudeOauth);
        assert_ne!(a, b);
        assert!(a.starts_with("sk-ant-oat01-hura"));
        assert_eq!(a.len(), "sk-ant-oat01-hura".len() + 24);
        assert!(placeholder(Kind::GitHub).starts_with("hura-cred-"));
    }

    /// An Azure DevOps PAT from a command is stored already encoded, so the
    /// header the sandbox sends is the placeholder and nothing around it.
    #[test]
    fn an_azure_devops_command_is_wrapped_to_store_the_basic_form() {
        let c = Credential {
            name: "ado".into(),
            kind: Kind::AzureDevOpsPat,
            source: Source::Command("cat ~/.config/hura/ado-pat".into()),
        };
        let spec = c.secret("hura-a", "hura-cred-x");
        assert_eq!(spec.hosts, ["dev.azure.com"]);
        assert_eq!(spec.env, "AZURE_DEVOPS_PAT");
        assert_eq!(spec.placeholder.as_deref(), Some("hura-cred-x"));
        assert_eq!(
            spec.source,
            SecretSource::Command(
                "printf ':%s' \"$(sh -c 'cat ~/.config/hura/ado-pat')\" | base64 -w0".into()
            )
        );

        let claude = Credential {
            name: "claude".into(),
            kind: Kind::ClaudeOauth,
            source: Source::Ref("op://Private/claude/token".into()),
        };
        assert_eq!(
            claude.secret("hura-a", "p").source,
            SecretSource::Ref("op://Private/claude/token".into())
        );
    }

    /// The wrapper runs the user's command through `sh -c` exactly as written,
    /// whatever quotes it has, and encodes what it printed.
    #[test]
    fn the_wrapped_command_produces_the_basic_credential() {
        let c = Credential {
            name: "ado".into(),
            kind: Kind::AzureDevOpsPat,
            source: Source::Command("echo 'it'\"'\"'s-a-pat' | tr -d '\\n'".into()),
        };
        let SecretSource::Command(wrapped) = c.secret("s", "p").source else {
            panic!("a command stays a command");
        };
        let out = std::process::Command::new("sh")
            .args(["-c", &wrapped])
            .output()
            .unwrap();
        let encoded = String::from_utf8(out.stdout).unwrap();
        let decoded = std::process::Command::new("sh")
            .args([
                "-c",
                &format!("printf %s {} | base64 -d", sh_quote(&encoded)),
            ])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(decoded.stdout).unwrap(), ":it's-a-pat");
    }
}
