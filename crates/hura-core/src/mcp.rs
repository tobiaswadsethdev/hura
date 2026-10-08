//! MCP servers the agent can reach, running *outside* the sandbox.
//!
//! An MCP server is a tool the agent can call, and the interesting ones need
//! credentials -- a Jira token, an Azure DevOps PAT. Running them inside the
//! sandbox would mean putting those credentials inside the sandbox, which is
//! the thing this tool exists not to do. So they run on the host, in their own
//! containers, holding their own secrets, and the sandbox is granted one
//! endpoint each.
//!
//! A sandbox reaches this machine as `host.docker.internal`, which the sandbox
//! runtime's proxy takes to the host's own loopback, and nothing else on it: a
//! container on the host cannot be reached by its name. So a server for
//! sandboxes is a port published on `127.0.0.1`, which is how a managed one is
//! run, and its url names `host.docker.internal` and that port. A server
//! somewhere else entirely is reached by its own host name.
//!
//! What does *not* work, and is rejected here rather than three steps later:
//! `localhost` and `127.0.0.1`. Inside a sandbox those mean the sandbox itself,
//! so a URL that is correct on the host is silently wrong once it gets there.
//!
//! **The grant is to the whole sandbox.** Its rules are not about one program,
//! so anything in the session can reach a server it is given, not only the
//! agent.
//!
//! **What this costs, said plainly.** The agent gains everything the MCP server
//! can do, using the host's credentials, and the proxy can only see it as
//! `POST /mcp`: every MCP call is the same request shape, so the method/path
//! rules that make the git endpoints sharp buy nothing here. A server that can
//! transition Jira issues means a sandboxed agent can transition Jira issues.
//! That is a fine trade for Jira and Azure DevOps, and a terrible one for a
//! filesystem or Docker MCP server on the host, which would be a straight
//! sandbox escape.

use serde::{Deserialize, Serialize};

use sbx_client::{Decision, RuleSpec};

mod managed;

pub use managed::{
    CONTAINER_PREFIX, Entry, Managed, State, Status, container_name, ensure, start, statuses, stop,
};

/// The name a sandbox reaches this machine by.
///
/// The sandbox runtime's proxy takes it to the host's own loopback, which is
/// why a rule for it names `localhost` and the port rather than this name.
pub const HOST_ALIAS: &str = "host.docker.internal";

/// What a URL named before the move to Docker Sandboxes said for the same
/// thing. Still recognised, so such an entry is known for what it means.
const OLD_HOST_ALIAS: &str = "host.openshell.internal";

/// How the agent talks to the server.
///
/// Not `stdio`: a stdio server runs *inside* the sandbox, which is a different
/// feature with a different security story -- it would need the package
/// registries open and it would hold its credentials in the sandbox. Wrap a
/// stdio server in an HTTP shim on the host instead; the README shows how.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// Streamable HTTP, the current transport.
    #[default]
    Http,
    /// Server-sent events, for servers that only speak the older transport.
    Sse,
}

impl Transport {
    /// What `claude mcp add --transport` is given.
    pub fn flag(self) -> &'static str {
        match self {
            Transport::Http => "http",
            Transport::Sse => "sse",
        }
    }

    pub fn parse(s: &str) -> Result<Self, Error> {
        match s.trim().to_ascii_lowercase().as_str() {
            "http" | "streamable-http" | "streamablehttp" => Ok(Transport::Http),
            "sse" => Ok(Transport::Sse),
            "stdio" => Err(Error::Stdio),
            other => Err(Error::Transport(other.to_string())),
        }
    }
}

/// One MCP server, as the config file names it and as the sandbox records it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    /// What the agent calls it. Also the tool prefix the agent shows, so it is
    /// worth keeping short.
    pub name: String,
    /// Exactly as configured, and exactly what `claude mcp add` is given.
    pub url: String,
    #[serde(default)]
    pub transport: Transport,
    /// `host:port`, which is what `policy update` addresses. Derived at parse
    /// time and kept, so nothing downstream re-derives it and so the sandbox's
    /// own record says what it was granted.
    pub endpoint: String,
}

impl Server {
    /// Validate a configured server, or say what is wrong with it.
    pub fn parse(name: &str, url: &str, transport: Transport) -> Result<Self, Error> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::NoName);
        }
        if let Some(c) = name
            .chars()
            .find(|c| !c.is_ascii_alphanumeric() && !matches!(c, '-' | '_'))
        {
            return Err(Error::BadName(c));
        }

        let url = url.trim();
        let endpoint = endpoint_of(url)?;
        Ok(Server {
            name: name.to_string(),
            url: url.to_string(),
            transport,
            endpoint,
        })
    }

    /// The host half of the endpoint.
    pub fn host(&self) -> &str {
        self.endpoint
            .rsplit_once(':')
            .map_or(self.endpoint.as_str(), |(h, _)| h)
    }

    /// Whether this one is reached through the host's published ports rather
    /// than by container name. The two need different checks in `doctor`.
    pub fn via_host(&self) -> bool {
        self.host() == HOST_ALIAS || self.host() == OLD_HOST_ALIAS
    }
}

/// The rule that opens every configured server, or none for no servers.
///
/// One rule, de-duplicated: two servers on one container, a common shape, are
/// one endpoint. A server reached through the host is named as the runtime
/// sees it, `localhost` and its port, since that is where its proxy takes
/// `host.docker.internal`.
pub fn rules(servers: &[Server]) -> Vec<RuleSpec> {
    let mut hosts: Vec<String> = Vec::new();
    for s in servers {
        let endpoint = if s.via_host() {
            let port = s.endpoint.rsplit_once(':').map_or("80", |(_, p)| p);
            format!("localhost:{port}")
        } else {
            s.endpoint.clone()
        };
        if !hosts.contains(&endpoint) {
            hosts.push(endpoint);
        }
    }
    if hosts.is_empty() {
        return Vec::new();
    }
    vec![RuleSpec {
        decision: Decision::Allow,
        resources: hosts,
        methods: Vec::new(),
        path: None,
    }]
}

/// The seeder's MCP step: register every server with the agent.
///
/// `claude mcp add` rather than writing `mcpServers` into `/home/agent/.claude.json`
/// by hand, because the CLI owns that file's shape and the image already
/// pre-populates it with the onboarding keys -- a hand-written merge would be a
/// second thing that has to know the format. `--scope user` puts them in that
/// same file, so they apply wherever in the sandbox the agent is started.
///
/// The remove before each add is what makes re-seeding an existing sandbox
/// idempotent; it fails when the server is not registered, which is the normal
/// case, so its failure is discarded.
///
/// An `add` that fails is *not* discarded. It runs under the seeder's `set -eu`,
/// so a URL the agent rejects fails the seed loudly, with the reason carried out
/// in the state file by the trap -- the alternative is a session that comes up
/// looking healthy and quietly has no tools.
pub fn register_script(servers: &[Server]) -> String {
    let mut out = String::new();
    for s in servers {
        out.push_str(&format!(
            "claude mcp remove --scope user {name} >/dev/null 2>&1 || true\n\
             claude mcp add --transport {transport} --scope user {name} {url}\n",
            name = crate::seed::sh_quote(&s.name),
            transport = s.transport.flag(),
            url = crate::seed::sh_quote(&s.url),
        ));
    }
    out
}

/// `host:port` for a URL, with the scheme's default port when it carries none.
///
/// A deliberately small parser rather than a URL crate: the shapes that reach it
/// are `http://name:9000/mcp` and `https://host.openshell.internal/mcp`, and the
/// only questions asked of them are the host and the port.
fn endpoint_of(url: &str) -> Result<String, Error> {
    let (scheme, rest) = url.split_once("://").ok_or(Error::Scheme)?;
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "http" => 80,
        "https" => 443,
        other => return Err(Error::BadScheme(other.to_string())),
    };

    // Path, query and fragment are none of an endpoint's business.
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|a| !a.is_empty())
        .ok_or(Error::NoHost)?;
    // Userinfo in an MCP URL would be a credential in a config file, which is
    // the thing this whole arrangement exists to avoid.
    if authority.contains('@') {
        return Err(Error::UserInfo);
    }

    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => {
            let port: u16 = p.parse().map_err(|_| Error::BadPort(p.to_string()))?;
            if port == 0 {
                return Err(Error::BadPort(p.to_string()));
            }
            (h, port)
        }
        None => (authority, default_port),
    };

    if host.is_empty() {
        return Err(Error::NoHost);
    }
    // The mistake worth catching at parse time. A server the *host* reaches on
    // localhost is not the same server the *sandbox* reaches there: inside, that
    // address is the sandbox itself, so the agent would report a dead MCP server
    // and nothing would ever say why.
    if is_loopback(host) {
        return Err(Error::Loopback(host.to_string()));
    }
    Ok(format!("{host}:{port}"))
}

fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host == "::1"
        || host
            .split_once('.')
            .is_some_and(|(first, _)| first == "127" && host.split('.').count() == 4)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("has no name")]
    NoName,
    #[error("is managed but names no image to run")]
    NoImage,
    #[error("name may only contain letters, digits, dashes and underscores; `{0}` is not allowed")]
    BadName(char),
    #[error("`{0}` is already the name of another mcp server")]
    DuplicateName(String),
    #[error("url has no scheme; write it as `http://host:port/mcp`")]
    Scheme,
    #[error("`{0}://` is not a transport the agent can be pointed at; use http or https")]
    BadScheme(String),
    #[error("url has no host")]
    NoHost,
    #[error("`{0}` is not a port number")]
    BadPort(String),
    #[error("url carries credentials; an mcp server should hold its own, not take them from here")]
    UserInfo,
    #[error(
        "`{0}` is the sandbox itself, not the host: publish the server on \
         127.0.0.1 here and reach it as `{HOST_ALIAS}`"
    )]
    Loopback(String),
    #[error(
        "`stdio` would run the server inside the sandbox, where its credentials \
         would have to live too; run it on the host behind an http shim instead"
    )]
    Stdio,
    #[error("`{0}` is not a transport; use http or sse")]
    Transport(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(url: &str) -> Result<Server, Error> {
        Server::parse("jira", url, Transport::Http)
    }

    #[test]
    fn an_endpoint_is_the_host_and_the_port() {
        assert_eq!(
            server("http://mcp-jira:9000/mcp").unwrap().endpoint,
            "mcp-jira:9000"
        );
        assert_eq!(
            server("https://host.openshell.internal/mcp")
                .unwrap()
                .endpoint,
            "host.openshell.internal:443"
        );
        assert_eq!(
            server("http://host.openshell.internal/mcp?x=1#f")
                .unwrap()
                .endpoint,
            "host.openshell.internal:80"
        );
    }

    /// The whole reason this is validated at parse time rather than left to the
    /// gateway: a loopback URL is correct on the host and wrong in the sandbox,
    /// and nothing downstream can tell the difference.
    #[test]
    fn loopback_is_refused_with_the_address_that_works() {
        for url in [
            "http://localhost:9000/mcp",
            "http://127.0.0.1:9000/mcp",
            "http://127.1.2.3:9000/mcp",
            "http://[::1]:9000/mcp",
        ] {
            let e = server(url).unwrap_err();
            assert!(matches!(e, Error::Loopback(_)), "{url}: {e:?}");
            let msg = e.to_string();
            assert!(
                msg.contains(HOST_ALIAS),
                "the message says what to use: {msg}"
            );
        }
    }

    /// `127.0.0.1` is loopback; a hostname that merely starts with the digits
    /// is not.
    #[test]
    fn only_real_loopback_addresses_are_loopback() {
        assert!(server("http://127-mcp:9000/mcp").is_ok());
        assert!(server("http://localhost.corp.example:9000/mcp").is_ok());
    }

    #[test]
    fn a_url_has_to_be_one() {
        assert_eq!(server("mcp-jira:9000").unwrap_err(), Error::Scheme);
        assert_eq!(
            server("ws://mcp-jira:9000").unwrap_err(),
            Error::BadScheme("ws".into())
        );
        assert_eq!(server("http:///mcp").unwrap_err(), Error::NoHost);
        assert_eq!(
            server("http://mcp:90000/mcp").unwrap_err(),
            Error::BadPort("90000".into())
        );
        assert_eq!(
            server("http://user:pass@mcp:9000/mcp").unwrap_err(),
            Error::UserInfo
        );
    }

    #[test]
    fn a_name_is_a_name() {
        assert!(Server::parse("", "http://m:1/mcp", Transport::Http).is_err());
        assert!(Server::parse("azure devops", "http://m:1/mcp", Transport::Http).is_err());
        assert!(Server::parse("azure-devops_2", "http://m:1/mcp", Transport::Http).is_ok());
    }

    /// A stdio server is a different feature, and saying so is more useful than
    /// "unknown transport".
    #[test]
    fn stdio_says_why_not() {
        let e = Transport::parse("stdio").unwrap_err();
        assert!(e.to_string().contains("inside the sandbox"), "{e}");
        assert_eq!(Transport::parse("http").unwrap(), Transport::Http);
        assert_eq!(Transport::parse("SSE").unwrap(), Transport::Sse);
        assert!(Transport::parse("carrier-pigeon").is_err());
    }

    #[test]
    fn nothing_configured_is_no_rule() {
        assert!(rules(&[]).is_empty());
    }

    /// One rule for every server, and a server on this machine as the
    /// runtime's proxy sees it.
    #[test]
    fn one_rule_opens_every_server() {
        let servers = vec![
            server("http://mcp-jira:9000/mcp").unwrap(),
            Server::parse(
                "local",
                "http://host.docker.internal:9001/mcp",
                Transport::Http,
            )
            .unwrap(),
        ];
        let r = rules(&servers);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].resources, ["mcp-jira:9000", "localhost:9001"]);
        assert!(
            r[0].methods.is_empty(),
            "MCP is one URL; a path rule buys nothing"
        );
    }

    /// Two tool sets served by one container are one endpoint.
    #[test]
    fn endpoints_are_not_added_twice() {
        let servers = vec![
            Server::parse("a", "http://mcp:9000/a", Transport::Http).unwrap(),
            Server::parse("b", "http://mcp:9000/b", Transport::Sse).unwrap(),
        ];
        assert_eq!(rules(&servers)[0].resources.len(), 1);
    }

    #[test]
    fn the_register_script_is_idempotent_and_quoted() {
        let servers = vec![
            server("http://mcp-jira:9000/mcp").unwrap(),
            Server::parse("azure", "http://mcp-azure:9001/mcp", Transport::Sse).unwrap(),
        ];
        let script = register_script(&servers);
        assert!(script.contains("claude mcp remove --scope user 'jira' >/dev/null 2>&1 || true"));
        assert!(script.contains(
            "claude mcp add --transport http --scope user 'jira' 'http://mcp-jira:9000/mcp'"
        ));
        assert!(script.contains("--transport sse --scope user 'azure'"));
    }

    #[test]
    fn a_hostile_name_cannot_escape_the_script() {
        // Refused outright, so the script never has to survive it -- but the
        // quoting is there anyway, because a Server can also come back from a
        // sandbox's own metadata record.
        assert!(Server::parse("a'; rm -rf /; '", "http://m:1/mcp", Transport::Http).is_err());
        let hostile = Server {
            name: "a'; rm -rf /; '".into(),
            url: "http://m:1/mcp".into(),
            transport: Transport::Http,
            endpoint: "m:1".into(),
        };
        let script = register_script(&[hostile]);
        assert!(!script.contains("; rm -rf /; \n"), "{script}");
        assert!(script.contains(r"'a'\''; rm -rf /; '\'''"), "{script}");
    }

    #[test]
    fn the_host_alias_is_told_apart_from_a_container() {
        assert!(
            server("http://host.openshell.internal:9000/mcp")
                .unwrap()
                .via_host()
        );
        assert!(
            server("http://host.docker.internal:9000/mcp")
                .unwrap()
                .via_host()
        );
        assert!(!server("http://mcp-jira:9000/mcp").unwrap().via_host());
        assert_eq!(
            server("http://mcp-jira:9000/mcp").unwrap().host(),
            "mcp-jira"
        );
    }
}
