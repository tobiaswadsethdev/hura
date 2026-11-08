//! The tracker functions as JSON requests: `{"op": "issue", "tracker": ...,
//! "key": "PROJ-7"}` in, the function's answer out.
//!
//! **One surface for every client that cannot call these functions
//! directly**: the iOS app, through `hura-mobile`'s C function, and hura
//! cloud's `POST /v1/tracker`, which puts the account's own tracker where the
//! request names it. The arguments and answers are the tracker module's serde
//! types, already shared with the desktop's TypeScript; a request names its
//! function and its arguments, and nothing else is described twice.
//!
//! Where an op takes a tracker, `tracker` is a whole [`Stored`], token
//! included, and `inbox` takes `trackers`, a list of them.

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::changes::{self, Seen};
use super::{FieldChange, Inbox, PullFile, PullMerge, PullVerdict, Source, Stored};

/// One request, by its `op`.
pub fn handle(request: &Value) -> Result<Value, String> {
    let op = request["op"].as_str().unwrap_or_default();
    let arg = |name: &str| -> Result<&str, String> {
        request[name]
            .as_str()
            .ok_or_else(|| format!("`{op}` needs `{name}`"))
    };
    let stored = || -> Result<Stored, String> { field(request, "tracker") };

    match op {
        // A tracker about to be added: trimmed and named, or why it would not
        // work. The same check `hura_client::trackers` makes.
        "check" => {
            let source: Source = field(request, "source")?;
            let source = source.normalized();
            match source.problem() {
                Some(problem) => Err(problem),
                None => to_value(source),
            }
        }
        // The branch a ticket's worktree gets on a server with this prefix.
        "branch" => to_value(super::branch(arg("prefix")?, arg("key")?, arg("title")?)),
        "inbox" => {
            let trackers: Vec<Stored> = field(request, "trackers")?;
            to_value(super::inbox(&trackers, arg("prefix")?))
        }
        // A read recorded against what was seen at the last one: what is seen
        // now, for the app to keep and hand back next time, and what changed.
        // Nothing seen, or something that will not parse, makes this read the
        // baseline: one quiet read, rather than every ticket announced.
        "changes" => {
            let inbox: Inbox = field(request, "inbox")?;
            let seen: Option<Seen> = field(request, "seen").ok().flatten();
            let (seen, changes) = changes::record(seen.as_ref(), &inbox);
            Ok(json!({ "seen": seen, "changes": changes }))
        }
        "issue" => to_value(super::issue(&stored()?, arg("key")?)?),
        "edit_form" => to_value(super::edit_form(&stored()?, arg("key")?)?),
        "save" => {
            let changes: Vec<FieldChange> = field(request, "changes")?;
            done(super::save(&stored()?, arg("key")?, &changes))
        }
        "transitions" => to_value(super::transitions(&stored()?, arg("key")?)?),
        "transition" => done(super::transition(&stored()?, arg("key")?, arg("id")?)),
        "comment" => done(super::comment(&stored()?, arg("key")?, arg("markdown")?)),
        "edit_comment" => done(super::edit_comment(
            &stored()?,
            arg("key")?,
            arg("id")?,
            arg("markdown")?,
        )),
        "delete_comment" => done(super::delete_comment(&stored()?, arg("key")?, arg("id")?)),
        "users" => to_value(super::users(
            &stored()?,
            arg("key")?,
            arg("query")?,
            request["assignable"].as_bool().unwrap_or(false),
        )?),
        "pull" => to_value(super::pull(&stored()?, arg("repo")?, arg("id")?)?),
        "pull_files" => to_value(super::pull_files(&stored()?, arg("repo")?, arg("id")?)?),
        "pull_file" => {
            let file: PullFile = field(request, "file")?;
            to_value(super::pull_file(
                &stored()?,
                arg("repo")?,
                arg("base")?,
                arg("head")?,
                &file,
            )?)
        }
        "pull_comment" => done(super::pull_comment(
            &stored()?,
            arg("repo")?,
            arg("id")?,
            arg("body")?,
            request["thread"].as_str(),
        )),
        "pull_vote" => {
            let verdict: PullVerdict = field(request, "verdict")?;
            done(super::pull_vote(
                &stored()?,
                arg("repo")?,
                arg("id")?,
                verdict,
                arg("body")?,
            ))
        }
        "pull_merge" => {
            let how: PullMerge = field(request, "how")?;
            done(super::pull_merge(
                &stored()?,
                arg("repo")?,
                arg("id")?,
                how,
                request["head"].as_str().unwrap_or_default(),
                request["delete_branch"].as_bool().unwrap_or(false),
            ))
        }
        // `how` null takes it back.
        "pull_auto_merge" => {
            let how: Option<PullMerge> = field(request, "how")?;
            done(super::pull_auto_merge(
                &stored()?,
                arg("repo")?,
                arg("id")?,
                how,
                request["delete_branch"].as_bool().unwrap_or(false),
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
