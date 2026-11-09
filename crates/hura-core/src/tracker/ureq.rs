//! A [`Transport`] over ureq, with rustls and the public roots, for a
//! process that cannot, or should not, run `curl` for every request: the iOS
//! app, and hura cloud.

use std::io::Read as _;
use std::time::Duration;

use super::{HttpRequest, Transport};

/// Tracker requests over ureq, answering every status as an answer: a 401
/// is a request that worked, and the tracker code reads it.
pub struct Ureq(ureq::Agent);

impl Ureq {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build();
        Ureq(config.into())
    }

    /// One that answers a redirect as it is rather than following it: for a
    /// caller that checks where each request goes (hura cloud, which must not
    /// be led from a public host to one inside its own network).
    pub fn without_redirects() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .build();
        Ureq(config.into())
    }
}

impl Transport for Ureq {
    fn send(&self, r: &HttpRequest) -> Result<(u16, Vec<u8>), String> {
        let timeout = Duration::from_secs(r.timeout);
        let mut builder = ureq::http::Request::builder()
            .method(r.method.as_str())
            .uri(&r.url);
        for line in &r.headers {
            if let Some((name, value)) = line.split_once(':') {
                builder = builder.header(name.trim(), value.trim());
            }
        }
        let request = builder
            .body(r.body.clone().unwrap_or_default())
            .map_err(|e| format!("could not make the request: {e}"))?;
        let request = self
            .0
            .configure_request(request)
            .timeout_global(Some(timeout))
            .build();
        let mut response = self
            .0
            .run(request)
            .map_err(|e| format!("the request failed: {e}"))?;
        let status = response.status().as_u16();
        // One byte past the limit, so that too large is told from just right.
        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(super::ATTACHMENT_LIMIT as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|e| format!("could not read the answer: {e}"))?;
        Ok((status, body))
    }
}

impl Default for Ureq {
    fn default() -> Self {
        Ureq::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// A server that answers every request with a redirect to `to`, once.
    fn redirecting(to: &str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let to = to.to_string();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = write!(
                    s,
                    "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            }
        });
        format!("http://{addr}/")
    }

    #[test]
    fn a_redirect_is_answered_as_it_is_and_not_followed() {
        let url = redirecting("http://169.254.169.254/latest/meta-data");
        let request = HttpRequest {
            method: "GET".into(),
            url,
            headers: Vec::new(),
            body: None,
            timeout: 5,
        };
        let (status, _) = Ureq::without_redirects().send(&request).unwrap();
        assert_eq!(status, 302);
    }
}
