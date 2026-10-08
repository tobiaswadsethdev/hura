//! The Docker Sandboxes contract, against a real `sbx`.
//!
//! Ignored by default: these need KVM, a signed-in `sbx` and its daemon, and
//! they make and remove real sandboxes named `hura-test-*`. One at a time:
//!
//! ```sh
//! cargo test -p sbx-client -- --ignored --test-threads=1
//! ```
//!
//! The image is the runtime's own `shell-docker` template, so nothing has to
//! be built first.

use sbx_client::{
    CliClient, CreateOpts, Decision, Error, RuleSpec, Sbx, SecretSource, SecretSpec, Status,
};

const TEMPLATE: &str = "docker/sandbox-templates:shell-docker";

fn client() -> CliClient {
    CliClient::new()
}

/// A sandbox that is removed when the test ends, however it ends.
struct Made<'a> {
    client: &'a CliClient,
    name: String,
}

impl<'a> Made<'a> {
    fn new(client: &'a CliClient, name: &str) -> Self {
        let _ = client.remove(name);
        client
            .create(&CreateOpts {
                name: name.into(),
                template: TEMPLATE.into(),
                cpus: Some(2),
                memory: Some("2g".into()),
                env: vec![],
            })
            .unwrap();
        Made {
            client,
            name: name.into(),
        }
    }
}

impl Drop for Made<'_> {
    fn drop(&mut self) {
        let _ = self.client.remove(&self.name);
    }
}

#[test]
#[ignore = "needs a signed-in sbx and KVM"]
fn a_missing_sandbox_is_not_found() {
    let c = client();
    assert!(matches!(
        c.exec("hura-test-nope", &["true"]),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        c.remove("hura-test-nope"),
        Err(Error::NotFound(_))
    ));
}

#[test]
#[ignore = "needs a signed-in sbx and KVM"]
fn create_exec_and_remove() {
    let c = client();
    let made = Made::new(&c, "hura-test-roundtrip");
    let listed = c.list().unwrap();
    let me = listed.iter().find(|s| s.name == made.name).unwrap();
    assert_eq!(me.status, Status::Running);

    let out = c.exec(&made.name, &["sh", "-c", "id -un; exit 3"]).unwrap();
    assert_eq!(out.stdout.trim(), "agent");
    assert_eq!(out.exit_code, 3);

    // Well past the 128 KiB a single argument may be.
    let big = vec![b'x'; 300 * 1024];
    let out = c.exec_stdin(&made.name, &["wc", "-c"], &big).unwrap();
    assert_eq!(out.stdout.trim(), (300 * 1024).to_string());
}

/// The longest name decides whether a session's sandbox can simply be
/// `hura-<session>`. Measured rather than read: the CLI's help names the
/// characters a name may have and not how many.
#[test]
#[ignore = "needs a signed-in sbx and KVM"]
fn a_long_name_is_accepted() {
    let c = client();
    let name = format!("hura-{}", "a".repeat(58));
    assert_eq!(name.len(), 63);
    let made = Made::new(&c, &name);
    assert!(c.list().unwrap().iter().any(|s| s.name == made.name));
}

#[test]
#[ignore = "needs a signed-in sbx and KVM"]
fn rules_are_added_listed_and_removed() {
    let c = client();
    let made = Made::new(&c, "hura-test-rules");
    let id = c
        .add_rule(
            &made.name,
            &RuleSpec {
                decision: Decision::Allow,
                resources: vec!["example.com:443".into()],
                methods: vec!["GET".into()],
                path: Some("/".into()),
            },
        )
        .unwrap();
    let rules = c.rules(&made.name).unwrap();
    let rule = rules.iter().find(|r| r.id == id).expect("the new rule");
    assert!(rule.is_scoped());
    assert_eq!(rule.methods, ["GET"]);

    c.remove_rule(&made.name, &id).unwrap();
    assert!(!c.rules(&made.name).unwrap().iter().any(|r| r.id == id));
}

/// Whether removing a sandbox takes its rules and secrets with it, which
/// decides whether a session made again under the same name could inherit an
/// allow its predecessor was given.
#[test]
#[ignore = "needs a signed-in sbx and KVM"]
fn removing_a_sandbox_takes_its_rules_and_secrets() {
    let c = client();
    let name = "hura-test-leftovers";
    let placeholder;
    {
        let made = Made::new(&c, name);
        c.add_rule(
            &made.name,
            &RuleSpec {
                decision: Decision::Allow,
                resources: vec!["example.org".into()],
                methods: vec![],
                path: None,
            },
        )
        .unwrap();
        placeholder = c
            .add_secret(&SecretSpec {
                sandbox: made.name.clone(),
                hosts: vec!["example.org".into()],
                env: "HURA_TEST_SECRET".into(),
                source: SecretSource::Command("echo not-a-secret".into()),
                placeholder: None,
            })
            .unwrap();
    }
    let made = Made::new(&c, name);
    let rules = c.rules(&made.name).unwrap();
    let leftover_rule = rules
        .iter()
        .any(|r| r.is_scoped() && r.resources.iter().any(|x| x == "example.org"));
    let leftover_secret = c
        .secrets()
        .unwrap()
        .iter()
        .any(|s| s.placeholder == placeholder);
    let _ = c.remove_secret(Some(name), &placeholder);
    println!("rule left behind: {leftover_rule}, secret left behind: {leftover_secret}");
    assert!(
        !leftover_rule,
        "a recreated sandbox inherited its predecessor's rule"
    );
}

#[test]
#[ignore = "needs a signed-in sbx and KVM"]
fn a_published_port_is_reported_and_withdrawn() {
    let c = client();
    let made = Made::new(&c, "hura-test-ports");
    let port = c.publish(&made.name, 8080).unwrap();
    assert_eq!(port.host_ip, "127.0.0.1");
    assert!(c.ports(&made.name).unwrap().contains(&port));
    c.unpublish(&made.name, &port).unwrap();
    assert!(!c.ports(&made.name).unwrap().contains(&port));
}

#[test]
#[ignore = "needs a signed-in sbx and KVM"]
fn a_detached_sandbox_outlives_its_sessions() {
    let c = client();
    let made = Made::new(&c, "hura-test-detached");
    c.detach(&made.name).unwrap();
    // Past the runtime's thirty seconds of grace after the last session.
    std::thread::sleep(std::time::Duration::from_secs(45));
    let me = c
        .list()
        .unwrap()
        .into_iter()
        .find(|s| s.name == made.name)
        .unwrap();
    assert_eq!(me.status, Status::Running);
}
