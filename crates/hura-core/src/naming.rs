//! Naming a session from its task, by asking a model.
//!
//! A session has two names: the slug everything is keyed by -- sandbox,
//! worktree directory, branch -- and the title a person reads. The slug rules in
//! [`crate::session::slugify`] make a serviceable slug and a poor title: they
//! can drop filler words, but they cannot tell which of the words left are the
//! point. A model can, in a second or two, and the answer is both: "Add
//! MaxGaming customer id to importer" and `add-maxgaming-customer-id-importer`.
//!
//! **Two ways to reach one, and neither is required.** The Messages API when the
//! server has `ANTHROPIC_API_KEY`, which is fast and is what a key is for; else
//! the `claude` command, signed in as whoever runs the server, which is what
//! most machines running hura already have. The Claude Code credential hura
//! hands to sandboxes is not used for this: that token is for Claude Code.
//!
//! **Everything fails to the rules.** No key and no `claude`, a timeout, a
//! refusal, an answer that is not JSON -- each is the slug rules and a title
//! from the task's first sentence, exactly as if nothing had been asked. A
//! session that cannot be named by a model is still a session; one that cannot
//! be created because a model was slow would not be.
//!
//! **It runs before the create, on the request.** The slug is the sandbox's
//! name, so it has to be decided before anything is made; there is no renaming
//! a sandbox afterwards. That puts a model call in front of every unnamed
//! create, bounded by [`TIMEOUT`].

use std::io::{Read as _, Write as _};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::session;

/// How a session given no name is named. See the config file's `names`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Ask a model, and fall back to the rules.
    Ai,
    /// The rules alone: nothing leaves the machine.
    Rules,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "ai" => Some(Mode::Ai),
            "rules" => Some(Mode::Rules),
            _ => None,
        }
    }
}

/// A name and a title for a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub name: String,
    pub title: Option<String>,
}

/// How long a create waits for a model before naming by the rules instead.
///
/// The `claude` command takes about three seconds from a cold start, most of it
/// the command starting; the API, about one. Ten covers a slow day for either
/// without making the create form feel stuck on the worst one.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// The model asked. Haiku: naming is a small job, and a small fast model is
/// what keeps it from being the slow part of a create.
const MODEL: &str = "claude-haiku-5-5";

/// How much of the task is sent. A name comes from what the task is about,
/// which is said at the start; a task with a pasted log after it does not need
/// the log read to be named.
const TASK_CHARS: usize = 4000;

const SYSTEM: &str = "You name coding tasks for a list of work sessions. \
Given the task a developer gave a coding agent, answer with two names for the work:\n\
- title: what the work is, in 2 to 6 words, sentence case, no trailing punctuation, \
the way a person would label it in a list. Keep a ticket key such as PROJ-123 at the start.\n\
- slug: the same idea as 2 to 5 lowercase words joined by hyphens, letters and digits \
only, at most 40 characters, usable as a git branch name.\n\
Name the work, not the request: leave out words like \"I want to\", \"please\" or \"can you\".";

/// The JSON schema both ways of asking constrain the answer to.
fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "slug": {"type": "string"},
        },
        "required": ["title", "slug"],
        "additionalProperties": false,
    })
}

/// Name a session that was given no name, stepping around the names `taken`.
///
/// `None` only when there is nothing at all to name it from: no task, and a
/// repository whose last path segment makes no slug.
pub fn name(task: &str, repo: &str, mode: Mode, taken: &[String]) -> Option<Named> {
    let asked = match mode {
        Mode::Ai if !task.trim().is_empty() => ask(task),
        _ => None,
    };
    let named = asked.or_else(|| {
        Some(Named {
            name: session::derive_name(task, repo)?,
            title: session::title_from_task(task),
        })
    })?;
    Some(Named {
        name: session::unique_name(&named.name, taken),
        ..named
    })
}

/// Ask whichever model can be reached. `None` when none can, or none answered
/// with something usable.
fn ask(task: &str) -> Option<Named> {
    // The suite is hermetic: no test may reach a network or a signed-in CLI,
    // and every test that creates a draft with no name would otherwise do both.
    if cfg!(test) {
        return None;
    }
    let task: String = task.chars().take(TASK_CHARS).collect();
    let answer = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.trim().is_empty() => ask_api(&task, key.trim()),
        _ => ask_claude(&task),
    };
    match answer {
        Ok(named) => Some(named),
        Err(e) => {
            // Said on the server's own output and nowhere else: the session is
            // named either way, and the reason is for whoever set this up.
            eprintln!("hurad: naming by the rules: {e}");
            None
        }
    }
}

/// The Messages API, through curl, with the key on curl's standard input
/// rather than its command line. See `tracker::run_curl` for why curl.
fn ask_api(task: &str, key: &str) -> Result<Named, String> {
    let body = serde_json::json!({
        "model": MODEL,
        "max_tokens": 1024,
        "system": SYSTEM,
        // Low: this is a label, not a problem to think about.
        "output_config": {
            "effort": "low",
            "format": {"type": "json_schema", "schema": schema()},
        },
        "messages": [{"role": "user", "content": task}],
    });
    let quote = crate::tracker::quote;
    let mut config = String::new();
    config.push_str("url = \"https://api.anthropic.com/v1/messages\"\n");
    config.push_str(&format!(
        "header = {}\n",
        quote(&format!("x-api-key: {key}"))
    ));
    config.push_str("header = \"anthropic-version: 2026-10-21\"\n");
    config.push_str("header = \"Content-Type: application/json\"\n");
    config.push_str("user-agent = \"hura\"\n");
    config.push_str("silent\nshow-error\n");
    config.push_str(&format!("max-time = {}\n", TIMEOUT.as_secs()));
    config.push_str(&format!("data-raw = {}\n", quote(&body.to_string())));

    let out = run(Command::new("curl").args(["-K", "-"]), &config, "curl")?;
    parse_api(&out)
}

/// The `claude` command, as a one-shot with no tools, no project, no history.
///
/// Run from the temporary directory so no repository's `CLAUDE.md` is read
/// into it, with settings and MCP servers off for the same reason, and without
/// saving the session: a name being made is not a conversation anybody will
/// want to resume.
fn ask_claude(task: &str) -> Result<Named, String> {
    let schema = schema().to_string();
    let mut command = Command::new("claude");
    command
        .current_dir(std::env::temp_dir())
        .args(["-p", "--model", "haiku", "--output-format", "json"])
        .args(["--json-schema", &schema, "--system-prompt", SYSTEM])
        .args([
            "--tools",
            "",
            "--setting-sources",
            "",
            "--strict-mcp-config",
        ])
        .args(["--disable-slash-commands", "--no-session-persistence"]);
    let out = run(&mut command, task, "claude")?;
    parse_claude(&out)
}

/// Run a command with `input` on its standard input, for at most [`TIMEOUT`].
fn run(command: &mut Command, input: &str, what: &str) -> Result<String, String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run {what}: {e}"))?;
    // Dropped once written, so the command sees the end of its input.
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("could not write to {what}: {e}"))?;
    }
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = stdout.read_to_string(&mut out);
                }
                if !status.success() {
                    return Err(format!("{what} exited with {status}"));
                }
                return Ok(out);
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{what} took longer than {}s", TIMEOUT.as_secs()));
            }
            Err(e) => return Err(format!("{what} could not be waited for: {e}")),
        }
    }
}

/// A Messages API reply: the first text block, which structured output makes
/// the JSON asked for.
fn parse_api(body: &str) -> Result<Named, String> {
    let reply: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("the API answered something not JSON: {e}"))?;
    if let Some(message) = reply["error"]["message"].as_str() {
        return Err(format!("the API refused: {message}"));
    }
    if reply["stop_reason"] == "refusal" {
        return Err("the model declined".into());
    }
    let text = reply["content"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b["type"] == "text")
                .and_then(|b| b["text"].as_str())
        })
        .ok_or("the API answered with no text")?;
    let answer: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| format!("the model answered something not JSON: {e}"))?;
    named(&answer)
}

/// `claude -p --output-format json`: one result object, with the structured
/// answer under `structured_output`.
fn parse_claude(stdout: &str) -> Result<Named, String> {
    let reply: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("claude answered something not JSON: {e}"))?;
    if reply["is_error"] == true {
        let why = reply["result"].as_str().unwrap_or("no reason given");
        return Err(format!("claude failed: {why}"));
    }
    named(&reply["structured_output"])
}

/// The answer, made safe to use: the slug through the same rules as any other
/// name, the title through the same cleaning as any other title. A slug that
/// comes out empty is no answer; a title that does is the slug's to stand in for.
fn named(answer: &serde_json::Value) -> Result<Named, String> {
    let slug = answer["slug"].as_str().unwrap_or_default();
    let name = session::kebab(slug).ok_or("the model's slug had nothing usable in it")?;
    session::validate_name(&name).map_err(|e| format!("the model's slug: {e}"))?;
    Ok(Named {
        name,
        title: answer["title"].as_str().and_then(session::clean_title),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the Messages API answers, trimmed to what is read.
    #[test]
    fn an_api_reply_is_read_from_its_text_block() {
        let body = r#"{
            "type": "message",
            "stop_reason": "end_turn",
            "content": [
                {"type": "thinking", "thinking": ""},
                {"type": "text", "text": "{\"title\":\"Fix flaky login test\",\"slug\":\"fix-flaky-login-test\"}"}
            ]
        }"#;
        assert_eq!(
            parse_api(body),
            Ok(Named {
                name: "fix-flaky-login-test".into(),
                title: Some("Fix flaky login test".into()),
            })
        );
    }

    #[test]
    fn an_api_error_or_refusal_is_no_answer() {
        let error = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        assert!(parse_api(error).unwrap_err().contains("invalid x-api-key"));
        let refused = r#"{"type":"message","stop_reason":"refusal","content":[]}"#;
        assert!(parse_api(refused).is_err());
    }

    /// Captured from `claude -p --output-format json --json-schema ...`, with
    /// the fields nothing reads cut out.
    #[test]
    fn a_claude_reply_is_read_from_its_structured_output() {
        let stdout = r#"{"type":"result","subtype":"success","is_error":false,
            "result":"{\"title\":\"Add MaxGaming Scala customer ID to importer\",\"slug\":\"add-maxgaming-scala-customer-id-importer\"}",
            "structured_output":{"title":"Add MaxGaming Scala customer ID to importer","slug":"add-maxgaming-scala-customer-id-importer"}}"#;
        assert_eq!(
            parse_claude(stdout),
            Ok(Named {
                name: "add-maxgaming-scala-customer-id-importer".into(),
                title: Some("Add MaxGaming Scala customer ID to importer".into()),
            })
        );
        let failed = r#"{"type":"result","is_error":true,"result":"Not logged in"}"#;
        assert!(parse_claude(failed).unwrap_err().contains("Not logged in"));
    }

    /// A model's answer is text from outside: it is made into a name by the
    /// same rules as anything else, and a slug with nothing in it is refused
    /// rather than becoming an empty name.
    #[test]
    fn a_model_answer_is_held_to_the_name_rules() {
        let answer = serde_json::json!({
            "title": "Fix the  login\ntest",
            "slug": "Fix the Login Test!!",
        });
        assert_eq!(
            named(&answer),
            Ok(Named {
                name: "fix-the-login-test".into(),
                title: Some("Fix the login test".into()),
            })
        );
        assert!(named(&serde_json::json!({"title": "x", "slug": "!!!"})).is_err());
        assert_eq!(
            named(&serde_json::json!({"title": "  ", "slug": "ok"}))
                .unwrap()
                .title,
            None
        );
    }

    /// The rules never ask anything, and are what every failure lands on.
    #[test]
    fn by_the_rules_a_title_comes_from_the_first_sentence() {
        let task = "fix the flaky login test. It fails on CI about once a day.";
        let named = name(task, "", Mode::Rules, &[]).unwrap();
        assert_eq!(named.name, session::derive_name(task, "").unwrap());
        assert_eq!(named.title.as_deref(), Some("Fix the flaky login test"));

        let taken = vec!["fix-flaky-login-test".to_string()];
        let again = name("fix the flaky login test", "", Mode::Rules, &taken).unwrap();
        assert_eq!(
            again.name, "fix-flaky-login-test-2",
            "a taken name steps aside"
        );
    }

    /// No task is nothing to ask a model about, so not even `ai` asks: the
    /// repository's name it is, with no title, which shows the name.
    #[test]
    fn with_no_task_the_repository_names_it_and_nothing_is_asked() {
        let named = name("  ", "https://github.com/you/api-server.git", Mode::Ai, &[]).unwrap();
        assert_eq!(named.name, "api-server");
        assert_eq!(named.title, None);
    }

    /// The real `claude` command, signed in. Ignored, like every test that
    /// needs something outside the suite: `cargo test -p hura-core -- --ignored
    /// naming`.
    #[test]
    #[ignore]
    fn the_claude_command_names_a_task() {
        let named = ask_claude("I want to add the MaxGaming Scala customer id to the importer")
            .expect("claude should have answered");
        assert!(session::validate_name(&named.name).is_ok(), "{named:?}");
        assert!(named.title.is_some(), "{named:?}");
        eprintln!("{named:?}");
    }

    #[test]
    fn the_mode_parses_its_two_words() {
        assert_eq!(Mode::parse("ai"), Some(Mode::Ai));
        assert_eq!(Mode::parse(" rules "), Some(Mode::Rules));
        assert_eq!(Mode::parse("llm"), None);
    }
}
