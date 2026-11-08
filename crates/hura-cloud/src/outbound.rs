//! Requests to trackers, and only to trackers.
//!
//! **A Jira site is an address an account typed**, and this service is the
//! one that goes there. Unguarded, `https://10.0.0.5` or `http://localhost`
//! would be a way to make requests from inside the server's network, to
//! Postgres's neighbours and to Coolify. So every tracker request goes
//! through [`Public`], which sends only HTTPS, and only to a host whose
//! every address is a public one.
//!
//! **No redirect is followed**, or a public host could answer with one to an
//! address inside: a redirect is an answer, which the tracker code reads as a
//! failure.
//!
//! What this does not close: the host is resolved again by the client that
//! sends the request, so a name that answers differently the second time
//! (DNS rebinding) could still get past. Pinning the resolved address into
//! the request is phase 6's.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs as _};

use hura_core::tracker::ureq::Ureq;
use hura_core::tracker::{HttpRequest, Transport};

pub struct Public(pub Ureq);

impl Transport for Public {
    fn send(&self, request: &HttpRequest) -> Result<(u16, String), String> {
        allowed(&request.url)?;
        self.0.send(request)
    }
}

/// Whether a URL is one a tracker could be at, or why not.
pub fn allowed(url: &str) -> Result<(), String> {
    let refused = |why: &str| Err(format!("hura cloud does not send requests to {url}: {why}"));
    let Some(rest) = url.strip_prefix("https://") else {
        return refused("only to https addresses");
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return refused("an address with a user in it");
    }
    let (host, port) = match authority.strip_prefix('[') {
        // [v6]:port
        Some(v6) => {
            let (host, after) = v6.split_once(']').unwrap_or((v6, ""));
            (host, after.strip_prefix(':'))
        }
        None => match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        },
    };
    let port: u16 = match port {
        Some(p) => match p.parse() {
            Ok(p) => p,
            Err(_) => return refused("not a port"),
        },
        None => 443,
    };
    if host.is_empty() {
        return refused("no host");
    }
    let addresses: Vec<IpAddr> = match (host, port).to_socket_addrs() {
        Ok(found) => found.map(|a| a.ip()).collect(),
        Err(e) => return Err(format!("could not find {host}: {e}")),
    };
    if addresses.is_empty() {
        return Err(format!("could not find {host}"));
    }
    if addresses.iter().all(|ip| public(*ip)) {
        Ok(())
    } else {
        refused("not a public address")
    }
}

fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => public_v4(v4),
            None => public_v6(v6),
        },
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || a == 0
        // Shared address space (carrier-grade NAT), and benchmarking.
        || (a == 100 && (64..128).contains(&b))
        || (a == 198 && (b == 18 || b == 19))
        || a >= 240)
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // Unique local, link local, and documentation.
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || first == 0x2001 && ip.segments()[1] == 0x0db8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_what_is_not_a_public_https_address() {
        for url in [
            "http://example.com/rest",
            "https://127.0.0.1/rest",
            "https://localhost/rest",
            "https://10.0.0.5:8443/x",
            "https://192.168.1.1",
            "https://169.254.169.254/latest/meta-data",
            "https://100.64.0.1",
            "https://[::1]/x",
            "https://[fd00::1]:443/x",
            "https://[::ffff:10.0.0.1]/x",
            "https://user@example.com/x",
            "https:///x",
        ] {
            assert!(allowed(url).is_err(), "{url}");
        }
    }

    #[test]
    fn allows_a_public_address() {
        // Literal addresses, so this needs no DNS.
        assert!(allowed("https://1.1.1.1/rest/api/3/myself").is_ok());
        assert!(allowed("https://[2606:4700:4700::1111]:443/x").is_ok());
    }
}
