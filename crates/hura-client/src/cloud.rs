//! This machine and hura cloud: signing in with an emailed code, the account
//! and its devices, and the trackers kept there. The Rust half of the iOS
//! app's `Cloud`, for the desktop application.
//!
//! **Trackers need it.** They are kept and read on the service, the same on
//! every device signed in to the account; without a session there are none.
//!
//! **The session is a file**, `cloud.json` in [`state::dir`], written
//! owner-only like `remotes.json`: it holds the refresh token, which this
//! machine keeps until it signs out. Access tokens last an hour and are only
//! ever held in memory.

use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hura_core::cloud::{Account, Device};
use hura_core::state;
use hura_core::tracker::changes::Change;
use hura_core::tracker::{Configured, Source};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DEFAULT_BASE: &str = "https://api.gethura.app";

/// This machine signed in to an account.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub base: String,
    pub email: String,
    pub account: String,
    pub device: String,
    pub refresh_token: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("base", &self.base)
            .field("email", &self.email)
            .field("refresh_token", &"<redacted>")
            .finish()
    }
}

/// One request on the wire: the status and the body, whatever the status.
pub trait Http: Send + Sync {
    fn send(
        &self,
        method: &str,
        url: &str,
        bearer: Option<&str>,
        body: Option<&Value>,
    ) -> Result<(u16, String), String>;

    /// A `GET` whose answer is read a line at a time as it arrives, for as
    /// long as `line` answers true and the connection stays open. The status,
    /// once it is over.
    fn lines(
        &self,
        url: &str,
        bearer: &str,
        line: &mut dyn FnMut(&str) -> bool,
    ) -> Result<u16, String>;
}

/// [`Http`] over ureq, with the public roots: the service has an ordinary
/// certificate, unlike a paired server.
pub struct Ureq {
    agent: ureq::Agent,
    /// For a stream, which is meant to stay open: no limit on the whole
    /// request, only on getting it started and on silence (the service
    /// writes a comment every fifteen seconds).
    streams: ureq::Agent,
}

impl Ureq {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        let streams = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(60)))
            .build();
        Ureq {
            agent: agent.into(),
            streams: streams.into(),
        }
    }
}

impl Default for Ureq {
    fn default() -> Self {
        Ureq::new()
    }
}

impl Http for Ureq {
    fn send(
        &self,
        method: &str,
        url: &str,
        bearer: Option<&str>,
        body: Option<&Value>,
    ) -> Result<(u16, String), String> {
        let mut request = ureq::http::Request::builder().method(method).uri(url);
        if let Some(token) = bearer {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if body.is_some() {
            request = request.header("Content-Type", "application/json");
        }
        let request = request
            .body(body.map(Value::to_string).unwrap_or_default())
            .map_err(|e| e.to_string())?;
        let mut response = self
            .agent
            .run(request)
            .map_err(|e| format!("could not reach hura cloud: {e}"))?;
        let status = response.status().as_u16();
        // Past ureq's ten megabytes: a ticket's image comes back in base64,
        // a third larger than the most an attachment may be.
        let text = response
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| format!("could not read hura cloud's answer: {e}"))?;
        Ok((status, text))
    }

    fn lines(
        &self,
        url: &str,
        bearer: &str,
        line: &mut dyn FnMut(&str) -> bool,
    ) -> Result<u16, String> {
        use std::io::BufRead as _;
        let mut response = self
            .streams
            .get(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Accept", "text/event-stream")
            .call()
            .map_err(|e| format!("could not reach hura cloud: {e}"))?;
        let status = response.status().as_u16();
        if status != 200 {
            return Ok(status);
        }
        let reader = std::io::BufReader::new(response.body_mut().as_reader());
        for read in reader.lines() {
            let text = read.map_err(|e| format!("the stream from hura cloud broke: {e}"))?;
            if !line(&text) {
                break;
            }
        }
        Ok(status)
    }
}

/// A failure, in words to show, and whether it means signed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub status: u16,
    pub message: String,
}

impl Failure {
    fn signed_out() -> Self {
        Failure {
            status: 401,
            message: "this computer was signed out of hura cloud".into(),
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub struct Cloud {
    http: Box<dyn Http>,
    path: PathBuf,
    access: Mutex<Option<(String, Instant)>>,
    /// Session file changes, one at a time.
    writing: Mutex<()>,
}

impl Cloud {
    pub fn new() -> Self {
        Cloud::with(Box::new(Ureq::new()), state::dir().join("cloud.json"))
    }

    pub fn with(http: Box<dyn Http>, path: PathBuf) -> Self {
        Cloud {
            http,
            path,
            access: Mutex::new(None),
            writing: Mutex::new(()),
        }
    }

    /// The session, read from its file each time, so a sign-out elsewhere in
    /// the process is seen at once.
    pub fn session(&self) -> Option<Session> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn signed_in(&self) -> bool {
        self.session().is_some()
    }

    // ------------------------------------------------------------ signing in

    /// Emails a six-digit code to `email`.
    pub fn send_code(&self, email: &str) -> Result<(), Failure> {
        self.request(
            DEFAULT_BASE,
            "POST",
            "/v1/sign-in/email",
            None,
            Some(&json!({ "email": email })),
        )
        .map(drop)
    }

    /// Signs this machine in with the code that was emailed, as `device`
    /// (its host name) on `platform`.
    pub fn verify(
        &self,
        email: &str,
        code: &str,
        device: &str,
        platform: &str,
    ) -> Result<Session, Failure> {
        #[derive(Deserialize)]
        struct Signed {
            account: String,
            device: String,
            refresh_token: String,
            access_token: String,
            expires_in: u64,
        }
        let body = json!({
            "email": email,
            "code": code,
            "device": { "name": device, "platform": platform },
        });
        let signed: Signed = parse(&self.request(
            DEFAULT_BASE,
            "POST",
            "/v1/sign-in/email/verify",
            None,
            Some(&body),
        )?)?;
        let session = Session {
            base: DEFAULT_BASE.into(),
            email: email.trim().to_lowercase(),
            account: signed.account,
            device: signed.device,
            refresh_token: signed.refresh_token,
        };
        if let Err(e) = self.keep(Some(&session)) {
            // Signed in on the service with nowhere to keep it: sign that
            // device straight back out rather than leave one nothing can use.
            let _ = self.request(
                DEFAULT_BASE,
                "POST",
                "/v1/sign-out",
                Some(&signed.access_token),
                None,
            );
            return Err(Failure {
                status: 0,
                message: format!("could not keep the session: {e}"),
            });
        }
        *self.access.lock().unwrap_or_else(|e| e.into_inner()) = Some((
            signed.access_token,
            Instant::now() + Duration::from_secs(signed.expires_in),
        ));
        Ok(session)
    }

    /// Signs this machine out: on the service if it answers, here regardless.
    pub fn sign_out(&self) {
        if self.signed_in() {
            let _ = self.authorized("POST", "/v1/sign-out", None);
        }
        self.forget();
    }

    // ------------------------------------------------------------ the account

    pub fn account(&self) -> Result<Account, Failure> {
        parse(&self.authorized("GET", "/v1/me", None)?)
    }

    pub fn devices(&self) -> Result<Vec<Device>, Failure> {
        parse(&self.authorized("GET", "/v1/devices", None)?)
    }

    /// Signs another of the account's devices out.
    pub fn sign_out_device(&self, id: &str) -> Result<(), Failure> {
        self.authorized("DELETE", &format!("/v1/devices/{}", segment(id)), None)
            .map(drop)
    }

    // ------------------------------------------------------------ trackers

    pub fn trackers(&self) -> Result<Vec<Configured>, Failure> {
        parse(&self.authorized("GET", "/v1/trackers", None)?)
    }

    pub fn add_tracker(
        &self,
        source: &Source,
        token: Option<&str>,
    ) -> Result<Vec<Configured>, Failure> {
        let body = json!({ "source": source, "token": token });
        parse(&self.authorized("POST", "/v1/trackers", Some(&body))?)
    }

    pub fn edit_tracker(&self, name: &str, source: &Source) -> Result<Vec<Configured>, Failure> {
        let path = format!("/v1/trackers/{}", segment(name));
        parse(&self.authorized("PUT", &path, Some(&json!({ "source": source })))?)
    }

    pub fn set_token(&self, name: &str, token: &str) -> Result<Vec<Configured>, Failure> {
        let path = format!("/v1/trackers/{}/token", segment(name));
        parse(&self.authorized("PUT", &path, Some(&json!({ "token": token })))?)
    }

    pub fn forget_tracker(&self, name: &str) -> Result<Vec<Configured>, Failure> {
        parse(&self.authorized("DELETE", &format!("/v1/trackers/{}", segment(name)), None)?)
    }

    /// The inbox was just looked at here, with these waiting (`tracker/id`
    /// keys): the account's other phones clear their badge and notifications.
    pub fn set_seen(&self, items: &[String]) -> Result<(), Failure> {
        self.authorized("PUT", "/v1/seen", Some(&json!({ "items": items })))
            .map(drop)
    }

    /// The providers the service can connect a tracker through (`github`,
    /// `jira`).
    pub fn connectable(&self) -> Result<Vec<String>, Failure> {
        let v: std::collections::BTreeMap<String, bool> =
            parse(&self.authorized("GET", "/v1/connect", None)?)?;
        Ok(v.into_iter()
            .filter(|(_, on)| *on)
            .map(|(k, _)| k)
            .collect())
    }

    /// Where to send a browser to connect `provider`. It ends on a page of
    /// the service's saying to come back.
    pub fn connect_url(&self, provider: &str) -> Result<String, Failure> {
        let path = format!("/v1/connect/{}", segment(provider));
        let v: Value =
            parse(&self.authorized("POST", &path, Some(&json!({ "return": "browser" })))?)?;
        v["url"].as_str().map(String::from).ok_or_else(|| Failure {
            status: 0,
            message: "hura cloud gave no address to connect with".into(),
        })
    }

    /// One of hura-core's tracker requests (`hura_core::tracker::request`),
    /// on the account's trackers: where it takes a `tracker`, a name. The
    /// answer, or the tracker's failure in words.
    pub fn tracker(&self, request: &Value) -> Result<Value, Failure> {
        let answer: Value = parse(&self.authorized("POST", "/v1/tracker", Some(request))?)?;
        match answer.get("error").and_then(Value::as_str) {
            Some(e) => Err(Failure {
                status: 200,
                message: e.to_string(),
            }),
            None => Ok(answer.get("ok").cloned().unwrap_or(Value::Null)),
        }
    }

    // ------------------------------------------------------------ live

    /// Blocks while hura cloud's stream of the account's changes is open
    /// (`GET /v1/events`), calling `on` with each batch as the service finds
    /// it, and `open` once the first line has arrived. Returns when the
    /// connection closes or this machine signs out; the caller reconnects.
    pub fn events(
        &self,
        open: &mut dyn FnMut(),
        on: &mut dyn FnMut(Vec<Change>),
    ) -> Result<(), Failure> {
        let session = self.session().ok_or_else(Failure::signed_out)?;
        let token = self.access_token(&session)?;
        let mut data = String::new();
        let mut opened = false;
        let status = self
            .http
            .lines(
                &format!("{}/v1/events", session.base),
                &token,
                &mut |line| {
                    if !opened {
                        opened = true;
                        open();
                    }
                    if let Some(d) = line.strip_prefix("data:") {
                        data.push_str(d.strip_prefix(' ').unwrap_or(d));
                    } else if line.is_empty() && !data.is_empty() {
                        #[derive(Deserialize)]
                        struct Told {
                            changes: Vec<Change>,
                        }
                        if let Ok(told) = serde_json::from_str::<Told>(&data) {
                            on(told.changes);
                        }
                        data.clear();
                    }
                    // Signed out meanwhile: done.
                    self.path.exists()
                },
            )
            .map_err(|message| Failure { status: 0, message })?;
        match status {
            200 => Ok(()),
            401 => {
                *self.access.lock().unwrap_or_else(|e| e.into_inner()) = None;
                Err(Failure {
                    status,
                    message: "hura cloud refused the access token".into(),
                })
            }
            other => Err(Failure {
                status: other,
                message: format!("hura cloud answered {other}"),
            }),
        }
    }

    // ------------------------------------------------------------ requests

    /// A request as the signed-in device. An access token that is refused is
    /// renewed once; a refresh token that is refused means this machine was
    /// signed out, and it is forgotten here too.
    fn authorized(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<String, Failure> {
        let session = self.session().ok_or_else(Failure::signed_out)?;
        let token = self.access_token(&session)?;
        match self.request(&session.base, method, path, Some(&token), body) {
            Err(e) if e.status == 401 => {
                *self.access.lock().unwrap_or_else(|e| e.into_inner()) = None;
                let token = self.access_token(&session)?;
                self.request(&session.base, method, path, Some(&token), body)
            }
            other => other,
        }
    }

    fn access_token(&self, session: &Session) -> Result<String, Failure> {
        {
            let access = self.access.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((token, until)) = access.as_ref()
                && *until > Instant::now() + Duration::from_secs(60)
            {
                return Ok(token.clone());
            }
        }
        #[derive(Deserialize)]
        struct Access {
            access_token: String,
            expires_in: u64,
        }
        let body = json!({ "refresh_token": session.refresh_token });
        let access: Access =
            match self.request(&session.base, "POST", "/v1/token", None, Some(&body)) {
                Ok(text) => parse(&text)?,
                Err(e) if e.status == 401 => {
                    self.forget();
                    return Err(Failure::signed_out());
                }
                Err(e) => return Err(e),
            };
        *self.access.lock().unwrap_or_else(|e| e.into_inner()) = Some((
            access.access_token.clone(),
            Instant::now() + Duration::from_secs(access.expires_in),
        ));
        Ok(access.access_token)
    }

    fn keep(&self, session: Option<&Session>) -> io::Result<()> {
        let _held = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        match session {
            Some(s) => {
                if let Some(dir) = self.path.parent() {
                    state::private_dir(dir)?;
                }
                let text = serde_json::to_string_pretty(s).map_err(io::Error::other)?;
                state::write_private(&self.path, &text)
            }
            None => match std::fs::remove_file(&self.path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        }
    }

    fn forget(&self) {
        let _ = self.keep(None);
        *self.access.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn request(
        &self,
        base: &str,
        method: &str,
        path: &str,
        bearer: Option<&str>,
        body: Option<&Value>,
    ) -> Result<String, Failure> {
        let (status, text) = self
            .http
            .send(method, &format!("{base}{path}"), bearer, body)
            .map_err(|message| Failure { status: 0, message })?;
        if (200..300).contains(&status) {
            return Ok(text);
        }
        let said = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from));
        Err(Failure {
            status,
            message: said.unwrap_or_else(|| format!("hura cloud answered {status}")),
        })
    }
}

impl Default for Cloud {
    fn default() -> Self {
        Cloud::new()
    }
}

fn parse<T: DeserializeOwned>(text: &str) -> Result<T, Failure> {
    serde_json::from_str(text).map_err(|e| Failure {
        status: 0,
        message: format!("hura cloud's answer was not understood: {e}"),
    })
}

/// A tracker's name or a device's id as one path segment.
fn segment(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// A stand-in for the service: answers by method and path, in order, the
    /// last repeating; and keeps what it was sent.
    #[derive(Default, Clone)]
    struct Service(Arc<Mutex<Book>>);

    #[derive(Default)]
    struct Book {
        answers: HashMap<String, Vec<(u16, String)>>,
        sent: Vec<Sent>,
    }

    #[derive(Debug, Clone)]
    struct Sent {
        route: String,
        bearer: Option<String>,
        body: Option<Value>,
    }

    impl Service {
        fn on(&self, route: &str, status: u16, body: &str) {
            let mut s = self.0.lock().unwrap();
            s.answers
                .entry(route.into())
                .or_default()
                .push((status, body.into()));
        }
        fn sent(&self, route: &str) -> Vec<Sent> {
            let s = self.0.lock().unwrap();
            s.sent
                .iter()
                .filter(|r| r.route == route)
                .cloned()
                .collect()
        }
    }

    impl Http for Service {
        fn send(
            &self,
            method: &str,
            url: &str,
            bearer: Option<&str>,
            body: Option<&Value>,
        ) -> Result<(u16, String), String> {
            let route = format!("{method} {}", url.trim_start_matches(DEFAULT_BASE));
            let mut s = self.0.lock().unwrap();
            s.sent.push(Sent {
                route: route.clone(),
                bearer: bearer.map(String::from),
                body: body.cloned(),
            });
            let queue = s.answers.entry(route).or_default();
            Ok(match queue.len() {
                0 => (404, r#"{"error":"no route"}"#.into()),
                1 => queue[0].clone(),
                _ => queue.remove(0),
            })
        }

        fn lines(
            &self,
            url: &str,
            bearer: &str,
            line: &mut dyn FnMut(&str) -> bool,
        ) -> Result<u16, String> {
            let (status, body) = self.send("GET", url, Some(bearer), None)?;
            for text in body.lines() {
                if !line(text) {
                    break;
                }
            }
            Ok(status)
        }
    }

    const SIGNED: &str = r#"{"account":"acc","device":"dev","refresh_token":"hura_rt_r","access_token":"hura_at_1","expires_in":3600}"#;

    fn signed_in() -> (Cloud, Service, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "hura-cloud-test-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        let path = dir.join("cloud.json");
        let service = Service::default();
        service.on("POST /v1/sign-in/email/verify", 200, SIGNED);
        let cloud = Cloud::with(Box::new(service.clone()), path.clone());
        cloud
            .verify(" Me@Example.com ", "123456", "laptop", "macos")
            .unwrap();
        (cloud, service, path)
    }

    /// A directory name of each test's own: the clock alone is the same for
    /// two tests started together, and two writing one `cloud.json` race.
    fn rand_suffix() -> String {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{nanos}-{n}")
    }

    #[test]
    fn signing_in_keeps_the_session_owner_only_and_uses_its_token() {
        let (cloud, service, path) = signed_in();
        let kept = cloud.session().unwrap();
        assert_eq!(kept.email, "me@example.com");
        assert!(!format!("{kept:?}").contains("hura_rt_r"), "redacted");
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "mode was {mode:o}");
        }
        let verify = &service.sent("POST /v1/sign-in/email/verify")[0];
        assert_eq!(verify.body.as_ref().unwrap()["device"]["platform"], "macos");

        service.on(
            "GET /v1/me",
            200,
            r#"{"account":"acc","plan":"free","sign_ins":[]}"#,
        );
        assert_eq!(cloud.account().unwrap().plan, "free");
        assert_eq!(
            service.sent("GET /v1/me")[0].bearer.as_deref(),
            Some("hura_at_1")
        );
    }

    #[test]
    fn a_refused_access_token_is_renewed_once() {
        let (cloud, service, _) = signed_in();
        service.on("GET /v1/trackers", 401, r#"{"error":"not signed in"}"#);
        service.on(
            "GET /v1/trackers",
            200,
            r#"[{"source":{"kind":"git-hub","name":"gh"},"token_set":true}]"#,
        );
        service.on(
            "POST /v1/token",
            200,
            r#"{"access_token":"hura_at_2","expires_in":3600}"#,
        );
        let list = cloud.trackers().unwrap();
        assert!(list[0].token_set);
        let sent = service.sent("GET /v1/trackers");
        assert_eq!(sent.last().unwrap().bearer.as_deref(), Some("hura_at_2"));
    }

    #[test]
    fn a_refused_refresh_token_signs_this_machine_out() {
        let (cloud, service, path) = signed_in();
        service.on("GET /v1/me", 401, r#"{"error":"not signed in"}"#);
        service.on("POST /v1/token", 401, r#"{"error":"not signed in"}"#);
        assert_eq!(cloud.account().unwrap_err().status, 401);
        assert!(!cloud.signed_in());
        assert!(!path.exists());
    }

    #[test]
    fn a_tracker_request_answers_its_value_or_its_failure() {
        let (cloud, service, _) = signed_in();
        service.on("POST /v1/tracker", 200, r#"{"ok":{"tasks":[]}}"#);
        service.on(
            "POST /v1/tracker",
            200,
            r#"{"error":"no tracker called `x`"}"#,
        );
        let ok = cloud
            .tracker(&json!({ "op": "inbox", "prefix": "hura" }))
            .unwrap();
        assert_eq!(ok["tasks"], json!([]));
        let err = cloud.tracker(&json!({ "op": "issue", "tracker": "x", "key": "1" }));
        assert_eq!(err.unwrap_err().message, "no tracker called `x`");
    }

    #[test]
    fn a_session_that_cannot_be_kept_is_signed_straight_back_out() {
        let service = Service::default();
        service.on("POST /v1/sign-in/email/verify", 200, SIGNED);
        service.on("POST /v1/sign-out", 204, "");
        // A directory where the file should be: it cannot be written.
        let dir = std::env::temp_dir().join(format!("hura-cloud-test-dir-{}", rand_suffix()));
        std::fs::create_dir_all(dir.join("cloud.json")).unwrap();
        let cloud = Cloud::with(Box::new(service.clone()), dir.join("cloud.json"));
        assert!(
            cloud
                .verify("me@example.com", "123456", "laptop", "linux")
                .is_err()
        );
        let out = &service.sent("POST /v1/sign-out")[0];
        assert_eq!(out.bearer.as_deref(), Some("hura_at_1"));
    }

    #[test]
    fn the_stream_hands_over_each_batch_of_changes() {
        let (cloud, service, _) = signed_in();
        let change = r#"{"tracker":"work","id":"1","key":"P-1","title":"T","what":"To Do → Done"}"#;
        service.on(
            "GET /v1/events",
            200,
            &format!(": keep-alive\n\nevent: tracker_change\ndata: {{\"changes\":[{change}]}}\n\n"),
        );
        let (mut opened, mut told) = (0, Vec::new());
        cloud
            .events(&mut || opened += 1, &mut |changes| told.extend(changes))
            .unwrap();
        assert_eq!(opened, 1);
        assert_eq!(told.len(), 1);
        assert_eq!(told[0].what, "To Do → Done");
        assert_eq!(
            service.sent("GET /v1/events")[0].bearer.as_deref(),
            Some("hura_at_1")
        );
    }

    #[test]
    fn names_are_one_path_segment() {
        assert_eq!(segment("my jira/prod"), "my%20jira%2Fprod");
        assert_eq!(segment("gh-1.x"), "gh-1.x");
    }
}
