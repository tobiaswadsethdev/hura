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
use std::sync::Once;

use hura_core::tracker::{self, ureq::Ureq};

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

/// One request, by its `op`: [`tracker::request::handle`], over ureq.
pub fn handle(request: &str) -> Result<Value, String> {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| tracker::use_transport(Box::new(Ureq::new())));

    let request: Value =
        serde_json::from_str(request).map_err(|e| format!("not a request: {e}"))?;
    tracker::request::handle(&request)
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
    fn records_a_read_against_what_was_seen() {
        let task = json!({
            "tracker": "work", "kind": "jira", "id": "P-1", "key": "P-1", "title": "T",
            "url": "", "status": "To Do", "item_type": "Bug", "session_name": "",
            "branch": "", "repo": null, "relation": "assigned", "filters": [],
            "updated": null, "comments": 0, "last_commenter": null,
            "last_comment_mine": false
        });
        let inbox = |task: &Value| json!({ "tasks": [task], "warnings": [], "read": ["work"] });
        let ask = |inbox: Value, seen: &Value| {
            handle(&json!({ "op": "changes", "inbox": inbox, "seen": seen }).to_string()).unwrap()
        };

        let first = ask(inbox(&task), &Value::Null);
        assert_eq!(first["changes"], json!([]), "the first read is a baseline");

        let mut moved = task.clone();
        moved["status"] = json!("Done");
        let second = ask(inbox(&moved), &first["seen"]);
        assert_eq!(second["changes"][0]["what"], "To Do → Done", "{second}");

        let garbled = ask(inbox(&moved), &json!("not what was seen"));
        assert_eq!(garbled["changes"], json!([]), "a baseline again");
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
