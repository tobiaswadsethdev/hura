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
//!
//! Two kinds have no fixed hosts, because what they reach is the repository's
//! business rather than hura's, and the config file names it:
//!
//! * **`azure-cli`** mints Entra access tokens with the host's own `az`, one
//!   secret per resource (`https://vault.azure.net`, ...), each sent only to
//!   the hosts listed for it. The sandbox has no Azure CLI and no login, only
//!   the image's stand-in `az`, which answers `az account get-access-token`
//!   with the resource's placeholder (see `images/hura-base/az`). Azure.Identity's
//!   `AzureCliCredential` asks exactly that, so code written for a developer's
//!   laptop runs unchanged.
//! * **`nuget-feed`** is a private NuGet feed's password. NuGet builds its own
//!   `Authorization: Basic base64(user:password)`, so a plain placeholder would
//!   be encoded before the proxy could see it. Instead the sandbox's NuGet is
//!   given a user and a random password whose encoding *is* the placeholder,
//!   and the value stored behind it is the encoding of the real password.
//!
//! A token from `az` lives about an hour, so those secrets are re-resolved on
//! a timer (`refresh`), which the runtime does without the sandbox noticing.

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
    /// Entra access tokens from the host's Azure CLI, per resource.
    AzureCli,
    /// The password of a private NuGet feed.
    NugetFeed,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::ClaudeOauth,
        Kind::AzureDevOpsPat,
        Kind::GitHub,
        Kind::AzureCli,
        Kind::NugetFeed,
    ];

    /// The name the config file and the create form use. The same names the
    /// OpenShell provider profiles had, so a `providers` list and the form's
    /// preselection keep meaning the same thing.
    pub fn name(self) -> &'static str {
        match self {
            Kind::ClaudeOauth => "claude-code-oauth",
            Kind::AzureDevOpsPat => "azure-devops-pat",
            Kind::GitHub => "github",
            Kind::AzureCli => "azure-cli",
            Kind::NugetFeed => "nuget-feed",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// The hosts whose requests carry it. Empty for the kinds whose hosts the
    /// config file names.
    pub fn hosts(self) -> &'static [&'static str] {
        match self {
            Kind::ClaudeOauth => &["api.anthropic.com", "platform.claude.com"],
            Kind::AzureDevOpsPat => &["dev.azure.com"],
            Kind::GitHub => &["github.com", "api.github.com"],
            Kind::AzureCli | Kind::NugetFeed => &[],
        }
    }

    /// The variable the placeholder is put in: the one the tool reads. For
    /// the two kinds that set one per resource or feed, what each starts with.
    pub fn env(self) -> &'static str {
        match self {
            Kind::ClaudeOauth => "CLAUDE_CODE_OAUTH_TOKEN",
            Kind::AzureDevOpsPat => "AZURE_DEVOPS_PAT",
            Kind::GitHub => "GITHUB_TOKEN",
            Kind::AzureCli => AZURE_TOKEN_ENV,
            Kind::NugetFeed => NUGET_CREDENTIALS_ENV,
        }
    }

    /// How a placeholder for this kind starts. Shaped like the real thing for
    /// Claude, so a client that checks a token's prefix before sending it
    /// sends the placeholder rather than refusing it.
    fn prefix(self) -> &'static str {
        match self {
            Kind::ClaudeOauth => "sk-ant-oat01-hura",
            Kind::AzureDevOpsPat | Kind::GitHub | Kind::NugetFeed => "hura-cred-",
            Kind::AzureCli => "hura-az-",
        }
    }
}

/// What the image's `az` reads a resource's placeholder from, followed by
/// [`resource_slug`]. The two have to agree; a test runs the script to check.
const AZURE_TOKEN_ENV: &str = "HURA_AZURE_TOKEN_";

/// The variable NuGet reads a source's credentials from, followed by the
/// source's key in `NuGet.config`. NuGet's own convention, so nothing in the
/// repository has to change for a sandbox to restore from the feed.
const NUGET_CREDENTIALS_ENV: &str = "NuGetPackageSourceCredentials_";

/// The user the sandbox's NuGet is given for a feed. Azure DevOps ignores the
/// user of a PAT or a token; it only has to be the same on both sides.
const NUGET_USER: &str = "hura";

/// How long an `az` token secret is kept before `az` is asked again. Entra
/// access tokens live an hour or more, so half an hour never sends a stale one.
pub const AZURE_REFRESH_DEFAULT: &str = "30m";

/// Where a credential's value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A command run on the host, in a shell, whose stdout is the value.
    Command(String),
    /// A 1Password `op://` reference or an AWS Secrets Manager ARN.
    Ref(String),
}

/// One Entra resource an `azure-cli` credential mints tokens for, and the
/// hosts its token is sent to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    /// As `az account get-access-token --resource` takes it:
    /// `https://vault.azure.net`.
    pub resource: String,
    pub hosts: Vec<String>,
}

/// How a credential's value reaches a sandbox, which is what differs between
/// the kinds beyond their names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// One value, to the kind's own hosts: the agent's and the forges'.
    Value(Source),
    /// A NuGet feed's password, to the hosts the feed is served from.
    Feed {
        source: Source,
        /// The source's key in `NuGet.config`, such as `MyPackages`.
        feed: String,
        hosts: Vec<String>,
    },
    /// Tokens the host's `az` mints, one secret per resource.
    Azure {
        /// The Azure CLI on the host. `az` unless the config says where.
        az: String,
        resources: Vec<Resource>,
    },
}

/// One `[credentials.NAME]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub name: String,
    pub kind: Kind,
    pub delivery: Delivery,
    /// How long the runtime keeps the value before resolving it again, as it
    /// reads durations (`30m`). `None` leaves its default, which resolves it
    /// when a request needs it; `azure-cli` defaults to
    /// [`AZURE_REFRESH_DEFAULT`] instead, since every value it has expires.
    pub refresh: Option<String>,
}

/// What a sandbox is given for one credential: variables set when it is
/// made, and the secrets added to it right after, each standing behind a
/// placeholder in one of those variables.
///
/// Both halves are decided together because they have to agree, and the
/// variables cannot come later: the runtime only sets a sandbox's variables
/// when it creates it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Grant {
    pub env: Vec<(String, String)>,
    pub secrets: Vec<SecretSpec>,
}

impl Credential {
    /// A credential of one of the kinds that carry a single value to their
    /// own hosts.
    pub fn value(name: &str, kind: Kind, source: Source) -> Credential {
        Credential {
            name: name.to_string(),
            kind,
            delivery: Delivery::Value(source),
            refresh: None,
        }
    }

    /// What the sandbox `sandbox` gets for this credential, under fresh
    /// placeholders.
    pub fn grant(&self, sandbox: &str) -> Grant {
        self.grant_with(sandbox, &mut random_hex)
    }

    /// [`Self::grant`], with the random part of each placeholder drawn from
    /// `fresh`, so a test can see what goes where.
    fn grant_with(&self, sandbox: &str, fresh: &mut dyn FnMut() -> String) -> Grant {
        match &self.delivery {
            Delivery::Value(source) => {
                let placeholder = format!("{}{}", self.kind.prefix(), fresh());
                Grant {
                    env: vec![(self.kind.env().to_string(), placeholder.clone())],
                    secrets: vec![self.value_secret(sandbox, source, &placeholder)],
                }
            }
            Delivery::Feed {
                source,
                feed,
                hosts,
            } => {
                let password = fresh();
                let placeholder = base64(format!("{NUGET_USER}:{password}").as_bytes());
                let header_env = format!("HURA_NUGET_FEED_{}", slug(feed));
                Grant {
                    env: vec![
                        (
                            format!("{NUGET_CREDENTIALS_ENV}{feed}"),
                            format!("Username={NUGET_USER};Password={password}"),
                        ),
                        // The placeholder itself, which is the whole Basic
                        // credential: `Authorization: Basic $HURA_NUGET_FEED_X`
                        // works for curl or a `dotnet nuget push`.
                        (header_env.clone(), placeholder.clone()),
                    ],
                    secrets: vec![SecretSpec {
                        sandbox: sandbox.to_string(),
                        hosts: hosts.clone(),
                        env: header_env,
                        source: basic_source(source),
                        placeholder: Some(placeholder),
                        refresh: self.refresh.clone(),
                    }],
                }
            }
            Delivery::Azure { az, resources } => {
                let mut grant = Grant::default();
                for r in resources {
                    let env = azure_token_env(&r.resource);
                    let placeholder = format!("{}{}", self.kind.prefix(), fresh());
                    grant.env.push((env.clone(), placeholder.clone()));
                    grant.secrets.push(SecretSpec {
                        sandbox: sandbox.to_string(),
                        hosts: r.hosts.clone(),
                        env,
                        source: SecretSource::Command(format!(
                            "{az} account get-access-token --resource {} --query accessToken -o tsv",
                            sh_quote(&r.resource)
                        )),
                        placeholder: Some(placeholder),
                        refresh: Some(
                            self.refresh
                                .clone()
                                .unwrap_or_else(|| AZURE_REFRESH_DEFAULT.to_string()),
                        ),
                    });
                }
                grant
            }
        }
    }

    /// The secret for a single-value kind, under `placeholder`.
    ///
    /// The value is what the proxy writes over the placeholder, so it has to
    /// be what belongs in the header. Claude and GitHub tokens go in as they
    /// are. An Azure DevOps PAT is HTTP Basic with the token as the password
    /// and an empty user, `base64(":" + pat)`: from a command, the command is
    /// wrapped so the runtime stores that form, and the sandbox sends
    /// `Authorization: Basic <placeholder>`. A reference cannot be wrapped,
    /// so for Azure DevOps it has to name the encoded form itself.
    fn value_secret(&self, sandbox: &str, source: &Source, placeholder: &str) -> SecretSpec {
        let source = match self.kind {
            Kind::AzureDevOpsPat => basic_source(source),
            _ => match source {
                Source::Command(cmd) => SecretSource::Command(cmd.clone()),
                Source::Ref(r) => SecretSource::Ref(r.clone()),
            },
        };
        SecretSpec {
            sandbox: sandbox.to_string(),
            hosts: self.kind.hosts().iter().map(|h| (*h).to_string()).collect(),
            env: self.kind.env().to_string(),
            source,
            placeholder: Some(placeholder.to_string()),
            refresh: self.refresh.clone(),
        }
    }
}

/// A password's source, made to store `base64(":" + password)`: what an
/// `Authorization: Basic` header carries for a token with an empty user,
/// which is how Azure DevOps takes a PAT and an Entra token alike.
fn basic_source(source: &Source) -> SecretSource {
    match source {
        Source::Command(cmd) => SecretSource::Command(format!(
            "printf ':%s' \"$(sh -c {})\" | base64 -w0",
            sh_quote(cmd)
        )),
        Source::Ref(r) => SecretSource::Ref(r.clone()),
    }
}

/// The variable a resource's placeholder is in.
pub fn azure_token_env(resource: &str) -> String {
    format!("{AZURE_TOKEN_ENV}{}", resource_slug(resource))
}

/// A resource as the tail of a variable name: no scheme, no `/.default`
/// scope suffix, upper case, and `_` for anything that is not a letter or a
/// digit. `https://vault.azure.net` is `VAULT_AZURE_NET`. The image's `az`
/// derives the same, from the `--resource` or `--scope` it is asked for.
pub fn resource_slug(resource: &str) -> String {
    let r = resource.trim();
    let r = r.split_once("://").map_or(r, |(_, rest)| rest);
    let r = r.strip_suffix("/.default").unwrap_or(r);
    slug(r.trim_end_matches('/'))
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Standard base64, padded: the alphabet `base64 -w0` and HTTP Basic use.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A fresh placeholder for a credential of this kind.
///
/// Random rather than derived, so nothing outside the sandbox can work one
/// out, and long enough that it never turns up in a request by accident.
pub fn placeholder(kind: Kind) -> String {
    format!("{}{}", kind.prefix(), random_hex())
}

/// Twelve random bytes as hex, for a value nothing outside hura can guess.
pub(crate) fn random_hex() -> String {
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
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Placeholders numbered in the order they are drawn.
    fn counter() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("r{n}")
        }
    }

    fn sh(script: &str) -> String {
        let out = std::process::Command::new("sh")
            .args(["-c", script])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    }

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
        let c = Credential::value(
            "ado",
            Kind::AzureDevOpsPat,
            Source::Command("cat ~/.config/hura/ado-pat".into()),
        );
        let grant = c.grant_with("hura-a", &mut counter());
        assert_eq!(
            grant.env,
            [("AZURE_DEVOPS_PAT".to_string(), "hura-cred-r1".to_string())]
        );
        let [spec] = grant.secrets.as_slice() else {
            panic!("one secret: {grant:?}");
        };
        assert_eq!(spec.hosts, ["dev.azure.com"]);
        assert_eq!(spec.env, "AZURE_DEVOPS_PAT");
        assert_eq!(spec.placeholder.as_deref(), Some("hura-cred-r1"));
        assert_eq!(spec.refresh, None);
        assert_eq!(
            spec.source,
            SecretSource::Command(
                "printf ':%s' \"$(sh -c 'cat ~/.config/hura/ado-pat')\" | base64 -w0".into()
            )
        );

        let claude = Credential::value(
            "claude",
            Kind::ClaudeOauth,
            Source::Ref("op://Private/claude/token".into()),
        );
        assert_eq!(
            claude.grant("hura-a").secrets[0].source,
            SecretSource::Ref("op://Private/claude/token".into())
        );
    }

    /// The wrapper runs the user's command through `sh -c` exactly as written,
    /// whatever quotes it has, and encodes what it printed.
    #[test]
    fn the_wrapped_command_produces_the_basic_credential() {
        let c = Credential::value(
            "ado",
            Kind::AzureDevOpsPat,
            Source::Command("echo 'it'\"'\"'s-a-pat' | tr -d '\\n'".into()),
        );
        let SecretSource::Command(wrapped) = c.grant("s").secrets[0].source.clone() else {
            panic!("a command stays a command");
        };
        let encoded = sh(&wrapped);
        let decoded = sh(&format!("printf %s {} | base64 -d", sh_quote(&encoded)));
        assert_eq!(decoded, ":it's-a-pat");
    }

    #[test]
    fn base64_matches_the_standard_alphabet() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("hura:a1b2c3", "aHVyYTphMWIyYzM="),
        ] {
            assert_eq!(base64(plain.as_bytes()), encoded, "{plain}");
        }
        assert_eq!(
            base64(b"hura:0123456789abcdef01234567"),
            sh("printf %s hura:0123456789abcdef01234567 | base64 -w0")
        );
    }

    /// NuGet is handed a user and password whose Basic encoding is exactly
    /// the placeholder, so the header it builds itself is what the proxy swaps.
    #[test]
    fn a_nuget_feed_is_given_a_password_whose_encoding_is_the_placeholder() {
        let c = Credential {
            name: "feed".into(),
            kind: Kind::NugetFeed,
            delivery: Delivery::Feed {
                source: Source::Command("tr -d '\\n' < /tokens/feed".into()),
                feed: "MyPackages".into(),
                hosts: vec!["pkgs.dev.azure.com".into()],
            },
            refresh: Some("30m".into()),
        };
        let grant = c.grant_with("hura-a", &mut counter());
        assert_eq!(
            grant.env,
            [
                (
                    "NuGetPackageSourceCredentials_MyPackages".to_string(),
                    "Username=hura;Password=r1".to_string()
                ),
                ("HURA_NUGET_FEED_MYPACKAGES".to_string(), base64(b"hura:r1")),
            ]
        );
        let [spec] = grant.secrets.as_slice() else {
            panic!("one secret: {grant:?}");
        };
        assert_eq!(
            spec.placeholder.as_deref(),
            Some(base64(b"hura:r1").as_str())
        );
        assert_eq!(spec.hosts, ["pkgs.dev.azure.com"]);
        assert_eq!(spec.env, "HURA_NUGET_FEED_MYPACKAGES");
        assert_eq!(spec.refresh.as_deref(), Some("30m"));
        let SecretSource::Command(wrapped) = &spec.source else {
            panic!("a command stays a command");
        };
        assert!(wrapped.starts_with("printf ':%s'"), "{wrapped}");
    }

    /// One secret per resource, each to its own hosts, each minted by `az` on
    /// a timer, under a placeholder in the variable the image's `az` reads.
    #[test]
    fn an_azure_cli_credential_is_a_secret_per_resource() {
        let c = Credential {
            name: "azure".into(),
            kind: Kind::AzureCli,
            delivery: Delivery::Azure {
                az: "/usr/bin/az".into(),
                resources: vec![
                    Resource {
                        resource: "https://vault.azure.net".into(),
                        hosts: vec!["kv.vault.azure.net".into()],
                    },
                    Resource {
                        resource: "https://storage.azure.com/".into(),
                        hosts: vec!["acct.blob.core.windows.net".into()],
                    },
                ],
            },
            refresh: None,
        };
        let grant = c.grant_with("hura-a", &mut counter());
        assert_eq!(
            grant.env,
            [
                (
                    "HURA_AZURE_TOKEN_VAULT_AZURE_NET".to_string(),
                    "hura-az-r1".to_string()
                ),
                (
                    "HURA_AZURE_TOKEN_STORAGE_AZURE_COM".to_string(),
                    "hura-az-r2".to_string()
                ),
            ]
        );
        assert_eq!(grant.secrets.len(), 2);
        let vault = &grant.secrets[0];
        assert_eq!(vault.hosts, ["kv.vault.azure.net"]);
        assert_eq!(vault.placeholder.as_deref(), Some("hura-az-r1"));
        assert_eq!(vault.refresh.as_deref(), Some(AZURE_REFRESH_DEFAULT));
        assert_eq!(
            vault.source,
            SecretSource::Command(
                "/usr/bin/az account get-access-token --resource 'https://vault.azure.net' \
                 --query accessToken -o tsv"
                    .into()
            )
        );
        assert_eq!(grant.secrets[1].hosts, ["acct.blob.core.windows.net"]);
    }

    #[test]
    fn resources_become_variable_names() {
        for (resource, slug) in [
            ("https://vault.azure.net", "VAULT_AZURE_NET"),
            ("https://vault.azure.net/", "VAULT_AZURE_NET"),
            ("https://vault.azure.net/.default", "VAULT_AZURE_NET"),
            ("https://storage.azure.com", "STORAGE_AZURE_COM"),
            (
                "499b84ac-1321-427f-aa17-267ca6975798",
                "499B84AC_1321_427F_AA17_267CA6975798",
            ),
            ("api://my-app/.default", "MY_APP"),
        ] {
            assert_eq!(resource_slug(resource), slug, "{resource}");
        }
    }

    /// The image's `az` and [`resource_slug`] have to name the same variable,
    /// or every token request in a sandbox misses. Run the script itself, for
    /// both ways Azure.Identity asks (`--resource` and `--scope`).
    #[test]
    fn the_image_az_answers_with_the_resource_placeholder() {
        let stub = include_str!("../../../images/hura-base/az");
        let dir = std::env::temp_dir().join(format!("hura-az-test-{}", random_hex()));
        std::fs::create_dir_all(&dir).unwrap();
        let az = dir.join("az");
        std::fs::write(&az, stub).unwrap();
        let run = |args: &str| {
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("sh {} {args}", sh_quote(&az.display().to_string())))
                .env(azure_token_env("https://vault.azure.net"), "hura-az-vault")
                .env(azure_token_env("https://storage.azure.com"), "hura-az-st")
                .output()
                .unwrap();
            (
                out.status.success(),
                String::from_utf8_lossy(&out.stdout).to_string(),
                String::from_utf8_lossy(&out.stderr).to_string(),
            )
        };

        let (ok, out, _) =
            run("account get-access-token --output json --resource https://vault.azure.net");
        assert!(ok);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(json["accessToken"], "hura-az-vault");
        assert_eq!(json["tokenType"], "Bearer");
        assert!(json["expires_on"].as_i64().unwrap() > 0, "{out}");
        assert!(json["expiresOn"].as_str().unwrap().contains(':'), "{out}");

        let (ok, out, _) = run(
            "account get-access-token --output json --scope https://storage.azure.com/.default",
        );
        assert!(ok);
        assert!(out.contains("\"hura-az-st\""), "{out}");

        let (ok, _, err) =
            run("account get-access-token --output json --resource https://graph.microsoft.com");
        assert!(!ok, "a resource with no credential is refused");
        assert!(err.contains("https://graph.microsoft.com"), "{err}");

        let (ok, _, err) = run("login");
        assert!(!ok);
        assert!(err.contains("get-access-token"), "{err}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
