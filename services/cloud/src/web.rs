//! Requests to GitHub and Atlassian on the service's own behalf: exchanging
//! and refreshing OAuth tokens, and asking who a token belongs to. Always to
//! the same few hosts, written here, never to an address anybody sent.

use std::time::Duration;

pub enum Web {
    Https(ureq::Agent),
    /// Answers from a list, for the tests: by `METHOD url` without its query,
    /// in order, the last repeating. Keeps what it was sent.
    #[cfg(test)]
    Kept {
        answers: std::sync::Mutex<std::collections::HashMap<String, Vec<(u16, String)>>>,
        sent: std::sync::Mutex<Vec<(String, Option<String>)>>,
    },
}

impl Web {
    pub fn https() -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .build();
        Web::Https(agent.into())
    }

    /// One request: the status and the body, whatever the status.
    pub async fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<String>,
    ) -> Result<(u16, String), String> {
        match self {
            Web::Https(agent) => {
                let mut request = ureq::http::Request::builder()
                    .method(method)
                    .uri(url)
                    // GitHub refuses a request with no user agent.
                    .header("User-Agent", "hura-cloud");
                for (name, value) in headers {
                    request = request.header(*name, value);
                }
                let request = request
                    .body(body.unwrap_or_default())
                    .map_err(|e| e.to_string())?;
                let agent = agent.clone();
                tokio::task::spawn_blocking(move || {
                    let mut response = agent.run(request).map_err(|e| e.to_string())?;
                    let status = response.status().as_u16();
                    let text = response
                        .body_mut()
                        .read_to_string()
                        .map_err(|e| e.to_string())?;
                    Ok((status, text))
                })
                .await
                .map_err(|e| e.to_string())?
            }
            #[cfg(test)]
            Web::Kept { answers, sent } => {
                let route = format!("{method} {}", url.split('?').next().unwrap_or(url));
                sent.lock().unwrap().push((route.clone(), body));
                let mut answers = answers.lock().unwrap();
                let queue = answers.entry(route).or_default();
                Ok(match queue.len() {
                    0 => (404, String::new()),
                    1 => queue[0].clone(),
                    _ => queue.remove(0),
                })
            }
        }
    }
}

#[cfg(test)]
impl Web {
    pub fn kept() -> Self {
        Web::Kept {
            answers: Default::default(),
            sent: Default::default(),
        }
    }

    pub fn on(&self, route: &str, status: u16, body: &str) {
        let Web::Kept { answers, .. } = self else {
            unreachable!()
        };
        answers
            .lock()
            .unwrap()
            .entry(route.into())
            .or_default()
            .push((status, body.into()));
    }

    pub fn sent(&self, route: &str) -> Vec<Option<String>> {
        let Web::Kept { sent, .. } = self else {
            unreachable!()
        };
        sent.lock()
            .unwrap()
            .iter()
            .filter(|(r, _)| r == route)
            .map(|(_, b)| b.clone())
            .collect()
    }
}
