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
}

impl Transport for Ureq {
    fn send(&self, r: &HttpRequest) -> Result<(u16, String), String> {
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
        let mut body = String::new();
        response
            .body_mut()
            .as_reader()
            .read_to_string(&mut body)
            .map_err(|e| format!("could not read the answer: {e}"))?;
        Ok((status, body))
    }
}

impl Default for Ureq {
    fn default() -> Self {
        Ureq::new()
    }
}
