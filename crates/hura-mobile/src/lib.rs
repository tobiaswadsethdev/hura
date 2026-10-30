//! hura-core's tracker client, for the iOS app.
//!
//! **The same code the desktop runs**, so a ticket reads and saves the same
//! way on both, and a fix to how Jira is spoken to is made once. What differs
//! is the transport: the desktop sends tracker requests through `curl`, and
//! an iOS app cannot run a program, so this installs a client of its own
//! (ureq, with rustls and the public roots) through
//! [`hura_core::tracker::use_transport`].
//!
//! **One C function, JSON both ways.** The surface is the tracker module's
//! functions, whose arguments and answers are already serde types shared
//! with the desktop's TypeScript; a generated binding would be a second
//! description of them. The app keeps the trackers and their tokens in the
//! Keychain and passes the one a call needs, so nothing here stores anything.

use std::ffi::{CStr, CString, c_char};
use std::io::Read as _;
use std::sync::Once;
use std::time::Duration;

use hura_core::tracker::{
    self, FieldChange, HttpRequest, PullFile, PullVerdict, Source, Stored, Transport,
};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// Answers one request. See `include/hura_mobile.h`.
///
/// # Safety
///
/// `request` is a NUL-terminated string, and the answer is freed with
/// [`hura_free`] and nothing else.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hura_tracker(request: *const c_char) -> *mut c_char {
    let request = if request.is_null() {
        String::new()
    } else {
        // SAFETY: the caller passes a NUL-terminated string.
        unsafe { CStr::from_ptr(request) }
            .to_string_lossy()
            .into_owned()
    };
    // A panic must not cross into Swift: it is an answer like any other.
    let answer = std::panic::catch_unwind(|| handle(&request))
        .unwrap_or_else(|_| Err("the tracker code panicked".into()));
    let body = match answer {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "error": e }),
    };
    CString::new(body.to_string())
        .unwrap_or_default()
        .into_raw()
}

/// Frees an answer from [`hura_tracker`].
///
/// # Safety
///
/// `answer` came from [`hura_tracker`] and has not been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hura_free(answer: *mut c_char) {
    if !answer.is_null() {
        // SAFETY: it came from `CString::into_raw` above.
        drop(unsafe { CString::from_raw(answer) });
    }
}

/// One request, by its `op`.
pub fn handle(request: &str) -> Result<Value, String> {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| tracker::use_transport(Box::new(Ureq::new())));

    let request: Value =
        serde_json::from_str(request).map_err(|e| format!("not a request: {e}"))?;
    let op = request["op"].as_str().unwrap_or_default();
    let arg = |name: &str| -> Result<&str, String> {
        request[name]
            .as_str()
            .ok_or_else(|| format!("`{op}` needs `{name}`"))
    };
    let stored = || -> Result<Stored, String> { field(&request, "tracker") };

    match op {
        // A tracker about to be added: trimmed and named, or why it would not
        // work. The same check `hura_client::trackers` makes.
        "check" => {
            let source: Source = field(&request, "source")?;
            let source = source.normalized();
            match source.problem() {
                Some(problem) => Err(problem),
                None => to_value(source),
            }
        }
        // The branch a ticket's worktree gets on a server with this prefix.
        "branch" => to_value(tracker::branch(arg("prefix")?, arg("key")?, arg("title")?)),
        "inbox" => {
            let trackers: Vec<Stored> = field(&request, "trackers")?;
            to_value(tracker::inbox(&trackers, arg("prefix")?))
        }
        "issue" => to_value(tracker::issue(&stored()?, arg("key")?)?),
        "edit_form" => to_value(tracker::edit_form(&stored()?, arg("key")?)?),
        "save" => {
            let changes: Vec<FieldChange> = field(&request, "changes")?;
            done(tracker::save(&stored()?, arg("key")?, &changes))
        }
        "transitions" => to_value(tracker::transitions(&stored()?, arg("key")?)?),
        "transition" => done(tracker::transition(&stored()?, arg("key")?, arg("id")?)),
        "comment" => done(tracker::comment(&stored()?, arg("key")?, arg("markdown")?)),
        "edit_comment" => done(tracker::edit_comment(
            &stored()?,
            arg("key")?,
            arg("id")?,
            arg("markdown")?,
        )),
        "delete_comment" => done(tracker::delete_comment(&stored()?, arg("key")?, arg("id")?)),
        "users" => to_value(tracker::users(
            &stored()?,
            arg("key")?,
            arg("query")?,
            request["assignable"].as_bool().unwrap_or(false),
        )?),
        "pull" => to_value(tracker::pull(&stored()?, arg("repo")?, arg("id")?)?),
        "pull_files" => to_value(tracker::pull_files(&stored()?, arg("repo")?, arg("id")?)?),
        "pull_file" => {
            let file: PullFile = field(&request, "file")?;
            to_value(tracker::pull_file(
                &stored()?,
                arg("repo")?,
                arg("base")?,
                arg("head")?,
                &file,
            )?)
        }
        "pull_comment" => done(tracker::pull_comment(
            &stored()?,
            arg("repo")?,
            arg("id")?,
            arg("body")?,
            request["thread"].as_str(),
        )),
        "pull_vote" => {
            let verdict: PullVerdict = field(&request, "verdict")?;
            done(tracker::pull_vote(
                &stored()?,
                arg("repo")?,
                arg("id")?,
                verdict,
                arg("body")?,
            ))
        }
        other => Err(format!("no such request: `{other}`")),
    }
}

fn field<T: DeserializeOwned>(request: &Value, name: &str) -> Result<T, String> {
    T::deserialize(&request[name]).map_err(|e| format!("`{name}`: {e}"))
}

fn to_value<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn done(r: Result<(), String>) -> Result<Value, String> {
    r.map(|()| Value::Null)
}

/// Tracker requests over ureq, answering every status as an answer: a 401
/// is a request that worked, and the tracker code reads it.
struct Ureq(ureq::Agent);

impl Ureq {
    fn new() -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_a_tracker_before_it_is_kept() {
        let ok = handle(r#"{"op":"check","source":{"kind":"git-hub","name":"  "}}"#).unwrap();
        assert!(
            !ok["name"].as_str().unwrap().trim().is_empty(),
            "named after its kind: {ok}"
        );
        let bad = handle(r#"{"op":"check","source":{"kind":"jira","name":"work"}}"#);
        assert!(bad.is_err(), "a Jira tracker needs a site: {bad:?}");
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        assert!(handle(r#"{"op":"nope"}"#).unwrap_err().contains("nope"));
        assert!(handle("not json").is_err());
        assert!(
            handle(r#"{"op":"issue","key":"X-1"}"#)
                .unwrap_err()
                .contains("tracker")
        );
    }

    #[test]
    fn answers_through_the_c_function() {
        let request = CString::new(r#"{"op":"inbox","trackers":[],"prefix":"hura"}"#).unwrap();
        // SAFETY: a valid string in, and the answer freed with hura_free.
        let answer = unsafe { hura_tracker(request.as_ptr()) };
        let text = unsafe { CStr::from_ptr(answer) }
            .to_str()
            .unwrap()
            .to_string();
        unsafe { hura_free(answer) };
        let v: Value = serde_json::from_str(&text).unwrap();
        assert!(v["ok"]["tasks"].is_array(), "{text}");
    }
}
