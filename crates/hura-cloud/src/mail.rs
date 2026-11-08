//! Sending the sign-in code.

use std::time::Duration;

use serde_json::json;

/// Where a code goes.
pub enum Mailer {
    /// Through Resend, from a domain verified there.
    Resend {
        key: String,
        from: String,
        agent: ureq::Agent,
    },
    /// Printed, for running the service on a laptop. Never chosen unless
    /// asked for by name: see [`crate::config`].
    Log,
    /// Kept, for the tests to read.
    #[cfg(test)]
    Kept(std::sync::Mutex<Vec<(String, String)>>),
}

impl Mailer {
    pub fn resend(key: String, from: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .into();
        Mailer::Resend { key, from, agent }
    }

    pub async fn send_code(&self, to: &str, code: &str) -> Result<(), String> {
        match self {
            Mailer::Resend { key, from, agent } => {
                let body = json!({
                    "from": from,
                    "to": [to],
                    "subject": format!("{code} is your hura sign-in code"),
                    "text": format!(
                        "Your code for signing in to hura is {code}.\n\n\
                         It works for ten minutes, once. If you did not ask to sign in, \
                         nobody can without this code, and you can ignore this email.\n"
                    ),
                })
                .to_string();
                let (agent, key) = (agent.clone(), key.clone());
                tokio::task::spawn_blocking(move || {
                    agent
                        .post("https://api.resend.com/emails")
                        .header("Authorization", format!("Bearer {key}"))
                        .content_type("application/json")
                        .send(&body)
                        .map(|_| ())
                        .map_err(|e| format!("Resend: {e}"))
                })
                .await
                .map_err(|e| e.to_string())?
            }
            Mailer::Log => {
                eprintln!("sign-in code for {to}: {code}");
                Ok(())
            }
            #[cfg(test)]
            Mailer::Kept(kept) => {
                kept.lock()
                    .unwrap()
                    .push((to.to_string(), code.to_string()));
                Ok(())
            }
        }
    }
}
