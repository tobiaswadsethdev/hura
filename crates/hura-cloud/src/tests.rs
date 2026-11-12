//! The API end to end, against a real Postgres.
//!
//! **Needs `HURA_CLOUD_TEST_DATABASE_URL`**, a server these may make
//! databases on; each test makes its own, so they run in parallel. Without
//! it they pass having checked nothing and say so, which keeps `cargo test`
//! needing nothing it did not need before. CI's `cloud` job runs them with a
//! Postgres beside it.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use sqlx::Connection as _;
use sqlx::postgres::{PgConnection, PgPool};
use tower::ServiceExt as _;

use crate::{Mailer, Shared, State};

struct Test {
    app: Router,
    state: Shared,
}

/// A service on a database of its own, or `None` with nothing to run on.
async fn service() -> Option<Test> {
    let Ok(url) = std::env::var("HURA_CLOUD_TEST_DATABASE_URL") else {
        eprintln!("skipped: HURA_CLOUD_TEST_DATABASE_URL is not set");
        return None;
    };
    let name = format!(
        "hura_cloud_test_{}",
        crate::token::number(u32::MAX).unwrap()
    );
    let mut admin = PgConnection::connect(&url)
        .await
        .expect("the test database");
    sqlx::query(&format!("create database {name}"))
        .execute(&mut admin)
        .await
        .unwrap();
    let (base, _) = url
        .rsplit_once('/')
        .expect("a URL ending in a database name");
    let db = PgPool::connect(&format!("{base}/{name}")).await.unwrap();
    crate::migrate(&db).await.unwrap();
    let state = Arc::new(State {
        db,
        mail: Mailer::Kept(Mutex::new(Vec::new())),
        keys: crate::crypto::Keys::from_base64("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
            .unwrap(),
        relay: crate::push::Relay::Kept {
            sent: Mutex::new(Vec::new()),
            statuses: Mutex::new(Vec::new()),
        },
        live: Default::default(),
        apple: crate::apple::Apple::fixed(
            "dev.example.app",
            "test-key",
            include_str!("testdata/apple-test-key.n.b64url").trim(),
        ),
        web: crate::web::Web::kept(),
        url: "https://cloud.example".into(),
        setup: "setup-token".into(),
    });
    crate::install_transport();
    Some(Test {
        app: crate::app(state.clone()),
        state,
    })
}

impl Test {
    async fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let request = match body {
            Some(body) => request
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    /// The last code sent to `email`.
    fn code(&self, email: &str) -> String {
        let Mailer::Kept(kept) = &self.state.mail else {
            unreachable!()
        };
        kept.lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(to, _)| to == email)
            .map(|(_, code)| code.clone())
            .expect("a code was sent")
    }

    /// Lets an address be sent another code at once.
    async fn forget_codes(&self) {
        sqlx::query("update email_codes set created_at = created_at - interval '2 hours'")
            .execute(&self.state.db)
            .await
            .unwrap();
    }

    async fn ask(&self, email: &str) -> StatusCode {
        self.call(
            Method::POST,
            "/v1/sign-in/email",
            None,
            Some(json!({ "email": email })),
        )
        .await
        .0
    }

    async fn verify(&self, email: &str, code: &str, device: &str) -> (StatusCode, Value) {
        let body = json!({ "email": email, "code": code, "device": { "name": device, "platform": "ios" } });
        self.call(Method::POST, "/v1/sign-in/email/verify", None, Some(body))
            .await
    }

    /// A device signed in to `email`'s account.
    async fn sign_in(&self, email: &str, device: &str) -> Value {
        self.forget_codes().await;
        assert_eq!(self.ask(email).await, StatusCode::NO_CONTENT);
        let (status, session) = self.verify(email, &self.code(email), device).await;
        assert_eq!(status, StatusCode::OK, "{session}");
        session
    }
}

fn token<'a>(session: &'a Value, which: &str) -> &'a str {
    session[which].as_str().unwrap()
}

#[tokio::test]
async fn signs_in_with_an_emailed_code() {
    let Some(t) = service().await else { return };
    assert_eq!(t.ask("  Alex@Example.com ").await, StatusCode::NO_CONTENT);
    let code = t.code("alex@example.com");
    assert_eq!(code.len(), 6);

    let (status, _) = t.verify("alex@example.com", "000000x", "iPhone").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "the wrong code");

    let (status, session) = t.verify("ALEX@example.com", &code, "iPhone").await;
    assert_eq!(status, StatusCode::OK, "{session}");
    assert!(token(&session, "refresh_token").starts_with("hura_rt_"));

    let (status, me) = t
        .call(
            Method::GET,
            "/v1/me",
            Some(token(&session, "access_token")),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["account"], session["account"]);
    assert_eq!(me["plan"], "free");
    assert_eq!(
        me["sign_ins"],
        json!([{ "provider": "email", "email": "alex@example.com" }])
    );

    // A refresh token buys a working access token.
    let refresh = json!({ "refresh_token": token(&session, "refresh_token") });
    let (status, access) = t.call(Method::POST, "/v1/token", None, Some(refresh)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = t
        .call(
            Method::GET,
            "/v1/me",
            Some(token(&access, "access_token")),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_code_is_used_once_and_tried_five_times() {
    let Some(t) = service().await else { return };
    t.ask("a@example.com").await;
    let code = t.code("a@example.com");
    assert_eq!(
        t.verify("a@example.com", &code, "Mac").await.0,
        StatusCode::OK
    );
    assert_eq!(
        t.verify("a@example.com", &code, "Mac").await.0,
        StatusCode::BAD_REQUEST
    );

    t.forget_codes().await;
    t.ask("a@example.com").await;
    let code = t.code("a@example.com");
    for _ in 0..5 {
        t.verify("a@example.com", "999999x", "Mac").await;
    }
    assert_eq!(
        t.verify("a@example.com", &code, "Mac").await.0,
        StatusCode::BAD_REQUEST,
        "spent by the wrong guesses"
    );
}

#[tokio::test]
async fn an_address_is_sent_one_code_a_minute() {
    let Some(t) = service().await else { return };
    assert_eq!(t.ask("a@example.com").await, StatusCode::NO_CONTENT);
    assert_eq!(t.ask("a@example.com").await, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        t.ask("b@example.com").await,
        StatusCode::NO_CONTENT,
        "per address"
    );
    assert_eq!(t.ask("not an address").await, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn devices_are_listed_and_signed_out_from_each_other() {
    let Some(t) = service().await else { return };
    let phone = t.sign_in("a@example.com", "iPhone").await;
    let mac = t.sign_in("a@example.com", "Mac").await;
    assert_eq!(phone["account"], mac["account"], "one address, one account");
    let (phone_at, mac_at) = (token(&phone, "access_token"), token(&mac, "access_token"));

    let (_, devices) = t.call(Method::GET, "/v1/devices", Some(mac_at), None).await;
    let names: Vec<(&str, bool)> = devices
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["name"].as_str().unwrap(), d["current"].as_bool().unwrap()))
        .collect();
    assert_eq!(names, [("Mac", true), ("iPhone", false)]);

    // Somebody else cannot sign the phone out.
    let other = t.sign_in("b@example.com", "Laptop").await;
    let path = format!("/v1/devices/{}", token(&phone, "device"));
    let (status, _) = t
        .call(
            Method::DELETE,
            &path,
            Some(token(&other, "access_token")),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The Mac can, and the phone's tokens stop working at once.
    let (status, _) = t.call(Method::DELETE, &path, Some(mac_at), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = t.call(Method::GET, "/v1/me", Some(phone_at), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let refresh = json!({ "refresh_token": token(&phone, "refresh_token") });
    let (status, _) = t.call(Method::POST, "/v1/token", None, Some(refresh)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // And signs itself out.
    let (status, _) = t
        .call(Method::POST, "/v1/sign-out", Some(mac_at), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = t.call(Method::GET, "/v1/devices", Some(mac_at), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn nothing_is_answered_without_a_token() {
    let Some(t) = service().await else { return };
    for token in [None, Some("hura_at_nonsense")] {
        let (status, body) = t.call(Method::GET, "/v1/me", token, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "not signed in");
    }
    let (status, _) = t.call(Method::GET, "/healthz", None, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn trackers_are_kept_and_their_tokens_never_shown() {
    let Some(t) = service().await else { return };
    let me = t.sign_in("a@example.com", "Mac").await;
    let at = token(&me, "access_token");
    let gh = json!({ "kind": "git-hub", "name": " gh " });

    let (status, list) = t
        .call(
            Method::POST,
            "/v1/trackers",
            Some(at),
            Some(json!({ "source": gh, "token": "ghp_secret" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list[0]["source"]["name"], "gh", "trimmed");
    assert_eq!(list[0]["token_set"], true);
    assert!(!list.to_string().contains("ghp_secret"));

    // Encrypted where it is kept.
    let kept: Vec<u8> = sqlx::query_scalar("select token from trackers")
        .fetch_one(&t.state.db)
        .await
        .unwrap();
    assert!(!kept.windows(10).any(|w| w == b"ghp_secret"));

    let (status, _) = t
        .call(
            Method::POST,
            "/v1/trackers",
            Some(at),
            Some(json!({ "source": gh })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "the name is taken");
    let jira = json!({ "kind": "jira", "name": "work" });
    let (status, body) = t
        .call(
            Method::POST,
            "/v1/trackers",
            Some(at),
            Some(json!({ "source": jira })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a Jira tracker needs a site: {body}"
    );
    let plain = json!({ "kind": "fizzy", "name": "cards", "site": "http://fizzy.example.com" });
    let (status, body) = t
        .call(
            Method::POST,
            "/v1/trackers",
            Some(at),
            Some(json!({ "source": plain })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a site that is not https is never read: {body}"
    );

    let renamed = json!({ "source": { "kind": "git-hub", "name": "other" } });
    let (status, _) = t
        .call(Method::PUT, "/v1/trackers/gh", Some(at), Some(renamed))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "not renamed");
    let edited = json!({ "source": { "kind": "git-hub", "name": "gh", "hide_created": true } });
    let (status, list) = t
        .call(Method::PUT, "/v1/trackers/gh", Some(at), Some(edited))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["source"]["hide_created"], true);
    assert_eq!(list[0]["token_set"], true, "the token stays");

    let (status, _) = t
        .call(
            Method::PUT,
            "/v1/trackers/nope/token",
            Some(at),
            Some(json!({ "token": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Nobody else's.
    let other = t.sign_in("b@example.com", "Laptop").await;
    let (_, theirs) = t
        .call(
            Method::GET,
            "/v1/trackers",
            Some(token(&other, "access_token")),
            None,
        )
        .await;
    assert_eq!(theirs, json!([]));

    let (status, list) = t
        .call(Method::DELETE, "/v1/trackers/gh", Some(at), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list, json!([]));
}

#[tokio::test]
async fn tracker_requests_read_the_accounts_own_trackers() {
    let Some(t) = service().await else { return };
    let me = t.sign_in("a@example.com", "iPhone").await;
    let at = token(&me, "access_token");
    let ask = |request: Value| t.call(Method::POST, "/v1/tracker", Some(at), Some(request));

    // No token, so nothing is sent anywhere: the inbox says so.
    t.call(
        Method::POST,
        "/v1/trackers",
        Some(at),
        Some(json!({ "source": { "kind": "git-hub", "name": "gh" } })),
    )
    .await;
    let (status, inbox) = ask(json!({ "op": "inbox", "prefix": "hura" })).await;
    assert_eq!(status, StatusCode::OK);
    let warnings = inbox["ok"]["warnings"].to_string();
    assert!(warnings.contains("gh"), "{inbox}");

    let (_, branch) =
        ask(json!({ "op": "branch", "prefix": "hura", "key": "P-1", "title": "Add it" })).await;
    assert!(
        branch["ok"].as_str().unwrap().starts_with("hura/P-1"),
        "{branch}"
    );

    let (_, unknown) = ask(json!({ "op": "issue", "tracker": "nope", "key": "1" })).await;
    assert_eq!(unknown["error"], "no tracker called `nope`");

    // A tracker brought along, token and all, is refused: the service is not
    // a way to send any token anywhere.
    let brought = json!({ "source": { "kind": "git-hub", "name": "x" }, "token": "t" });
    let (_, refused) = ask(json!({ "op": "issue", "tracker": brought, "key": "1" })).await;
    assert!(
        refused["error"].as_str().unwrap().contains("name"),
        "{refused}"
    );

    // A site inside the server's network is never asked.
    let inside =
        json!({ "kind": "jira", "name": "work", "site": "https://127.0.0.1", "email": "a@b.c" });
    t.call(
        Method::POST,
        "/v1/trackers",
        Some(at),
        Some(json!({ "source": inside, "token": "t" })),
    )
    .await;
    let (_, refused) = ask(json!({ "op": "issue", "tracker": "work", "key": "P-1" })).await;
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("does not send requests"),
        "{refused}"
    );
}

const PUSH_KEY: [u8; 32] = [9; 32];
const SERVER_ID: &str = "h_cloudcloudcloud2";

impl Test {
    fn relay_sent(&self) -> Vec<(String, Value)> {
        let crate::push::Relay::Kept { sent, .. } = &self.state.relay else {
            unreachable!()
        };
        sent.lock().unwrap().clone()
    }

    fn relay_answers(&self, list: &[u16]) {
        let crate::push::Relay::Kept { statuses, .. } = &self.state.relay else {
            unreachable!()
        };
        statuses.lock().unwrap().extend_from_slice(list);
    }

    /// A device told through the relay, and its account.
    async fn routed(&self, email: &str) -> (Value, uuid::Uuid) {
        use base64::Engine as _;
        let session = self.sign_in(email, "iPhone").await;
        let body = json!({
            "server_id": SERVER_ID,
            "push_token": "hup_token",
            "push_key": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(PUSH_KEY),
        });
        let (status, _) = self
            .call(
                Method::PUT,
                "/v1/push",
                Some(token(&session, "access_token")),
                Some(body),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let account = session["account"].as_str().unwrap().parse().unwrap();
        (session, account)
    }
}

fn ticket(id: &str, status: &str) -> hura_core::tracker::Task {
    hura_core::tracker::Task {
        tracker: "work".into(),
        kind: hura_core::tracker::Kind::Jira,
        id: format!("1000{id}"),
        key: format!("P-{id}"),
        title: format!("Ticket {id}"),
        url: String::new(),
        status: status.into(),
        item_type: "Bug".into(),
        session_name: String::new(),
        branch: String::new(),
        repo: None,
        relation: hura_core::tracker::Relation::Assigned,
        filters: Vec::new(),
        updated: None,
        comments: Some(0),
        last_commenter: None,
        last_comment_mine: false,
    }
}

fn read(tasks: Vec<hura_core::tracker::Task>) -> hura_core::tracker::Inbox {
    hura_core::tracker::Inbox {
        tasks,
        read: vec!["work".into()],
        ..Default::default()
    }
}

#[tokio::test]
async fn a_push_route_is_checked_and_kept_for_the_device() {
    let Some(t) = service().await else { return };
    let (session, _) = t.routed("a@example.com").await;
    let at = token(&session, "access_token");
    let (_, view) = t.call(Method::GET, "/v1/push", Some(at), None).await;
    assert_eq!(view, json!({ "configured": true, "broken": null }));

    // Kept encrypted.
    let kept: Vec<u8> = sqlx::query_scalar("select push_token from push_routes")
        .fetch_one(&t.state.db)
        .await
        .unwrap();
    assert!(!kept.windows(9).any(|w| w == b"hup_token"));

    for bad in [
        json!({ "server_id": "nope", "push_token": "hup_x", "push_key": "AAAA" }),
        json!({ "server_id": SERVER_ID, "push_token": "hup_x", "push_key": "AAAA" }),
        json!({ "server_id": SERVER_ID, "push_token": "nope", "push_key": "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk" }),
    ] {
        let (status, body) = t.call(Method::PUT, "/v1/push", Some(at), Some(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    // A token the relay does not know.
    t.relay_answers(&[401]);
    let refused = json!({ "server_id": SERVER_ID, "push_token": "hup_y", "push_key": "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk" });
    let (status, _) = t
        .call(Method::PUT, "/v1/push", Some(at), Some(refused))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = t.call(Method::DELETE, "/v1/push", Some(at), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, view) = t.call(Method::GET, "/v1/push", Some(at), None).await;
    assert_eq!(view["configured"], false);
}

#[tokio::test]
async fn what_changed_is_told_once_encrypted_to_every_route() {
    let Some(t) = service().await else { return };
    let (_, account) = t.routed("a@example.com").await;

    // The first read is the baseline.
    assert!(
        crate::watch::announce(&t.state, account, &read(vec![ticket("1", "To Do")]))
            .await
            .unwrap()
    );
    assert!(t.relay_sent().is_empty());

    assert!(
        crate::watch::announce(&t.state, account, &read(vec![ticket("1", "Done")]))
            .await
            .unwrap()
    );
    let sent = t.relay_sent();
    assert_eq!(sent.len(), 1);
    let (push_token, event) = &sent[0];
    assert_eq!(push_token, "hup_token");
    assert_eq!(event["kind"], "tracker_change");
    assert!(event.get("title").is_none(), "only ciphertext: {event}");
    let text = crate::push::decrypt(&PUSH_KEY, SERVER_ID, event["encrypted"].as_str().unwrap());
    assert_eq!(text.title, "P-1 · To Do → Done");
    assert_eq!(
        (text.tracker.as_deref(), text.id.as_deref()),
        (Some("work"), Some("10001"))
    );

    // The same read again says nothing.
    assert!(
        crate::watch::announce(&t.state, account, &read(vec![ticket("1", "Done")]))
            .await
            .unwrap()
    );
    assert_eq!(t.relay_sent().len(), 1);
}

#[tokio::test]
async fn a_relay_that_is_away_is_tried_again_with_the_same_event() {
    let Some(t) = service().await else { return };
    let (_, account) = t.routed("a@example.com").await;
    crate::watch::announce(&t.state, account, &read(vec![ticket("1", "To Do")]))
        .await
        .unwrap();

    t.relay_answers(&[503]);
    let moved = read(vec![ticket("1", "Done")]);
    assert!(
        !crate::watch::announce(&t.state, account, &moved)
            .await
            .unwrap(),
        "not told"
    );
    assert!(
        crate::watch::announce(&t.state, account, &moved)
            .await
            .unwrap(),
        "told now"
    );
    let sent = t.relay_sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[0].1["event_id"], sent[1].1["event_id"],
        "a duplicate to the relay"
    );
}

#[tokio::test]
async fn a_refused_route_is_marked_broken_and_left_out() {
    let Some(t) = service().await else { return };
    let (session, account) = t.routed("a@example.com").await;
    crate::watch::announce(&t.state, account, &read(vec![ticket("1", "To Do")]))
        .await
        .unwrap();

    t.relay_answers(&[401]);
    assert!(
        crate::watch::announce(&t.state, account, &read(vec![ticket("1", "Done")]))
            .await
            .unwrap()
    );
    let (_, view) = t
        .call(
            Method::GET,
            "/v1/push",
            Some(token(&session, "access_token")),
            None,
        )
        .await;
    assert!(
        view["broken"].as_str().unwrap().contains("refused"),
        "{view}"
    );

    // Nobody left to tell, so the account is not read.
    sqlx::query("update watches set next_read_at = now() - interval '1 minute'")
        .execute(&t.state.db)
        .await
        .unwrap();
    assert_eq!(crate::watch::tick(&t.state).await.unwrap(), 0);
}

#[tokio::test]
async fn many_changes_at_once_are_one_notification() {
    let Some(t) = service().await else { return };
    let (_, account) = t.routed("a@example.com").await;
    let tickets = |status: &str| read((1..=5).map(|i| ticket(&i.to_string(), status)).collect());
    crate::watch::announce(&t.state, account, &tickets("To Do"))
        .await
        .unwrap();
    crate::watch::announce(&t.state, account, &tickets("Done"))
        .await
        .unwrap();
    let sent = t.relay_sent();
    assert_eq!(sent.len(), 1);
    let text = crate::push::decrypt(
        &PUSH_KEY,
        SERVER_ID,
        sent[0].1["encrypted"].as_str().unwrap(),
    );
    assert!(text.title.starts_with("5 changes"), "{}", text.title);
    assert_eq!(text.tracker, None);
}

#[tokio::test]
async fn a_due_account_with_a_route_is_read_and_put_back_in_line() {
    let Some(t) = service().await else { return };
    t.routed("a@example.com").await;
    assert_eq!(
        crate::watch::tick(&t.state).await.unwrap(),
        1,
        "due at once"
    );
    assert_eq!(
        crate::watch::tick(&t.state).await.unwrap(),
        0,
        "not again so soon"
    );
    let wait: f64 =
        sqlx::query_scalar("select extract(epoch from next_read_at - now())::float8 from watches")
            .fetch_one(&t.state.db)
            .await
            .unwrap();
    assert!((50.0..=95.0).contains(&wait), "next read in {wait}s");
    let seen: Option<Vec<u8>> = sqlx::query_scalar("select seen from watches")
        .fetch_one(&t.state.db)
        .await
        .unwrap();
    assert!(seen.is_some(), "the baseline is kept");
}

#[tokio::test]
async fn an_open_stream_is_told_once_and_gets_its_account_read() {
    let Some(t) = service().await else { return };
    let session = t.sign_in("a@example.com", "Mac").await;
    let account: uuid::Uuid = session["account"].as_str().unwrap().parse().unwrap();

    // Asked over HTTP: a stream of events, for a signed-in device only.
    let (status, _) = t.call(Method::GET, "/v1/events", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let request = Request::builder()
        .uri("/v1/events")
        .header(
            "Authorization",
            format!("Bearer {}", token(&session, "access_token")),
        )
        .body(Body::empty())
        .unwrap();
    let response = t.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");

    // With a stream open and no phone, the account is read.
    let mut stream = t.state.live.subscribe(account);
    assert_eq!(crate::watch::tick(&t.state).await.unwrap(), 1);

    // The first read with the tracker in it is its baseline.
    crate::watch::announce(&t.state, account, &read(vec![ticket("1", "To Do")]))
        .await
        .unwrap();
    crate::watch::announce(&t.state, account, &read(vec![ticket("1", "Done")]))
        .await
        .unwrap();
    let told = stream.try_recv().unwrap();
    assert_eq!(told["changes"][0]["what"], "To Do → Done");
    assert!(stream.try_recv().is_err(), "once");
    drop(response);
}

#[tokio::test]
async fn a_stream_is_not_told_twice_when_the_relay_puts_the_telling_off() {
    let Some(t) = service().await else { return };
    let (_, account) = t.routed("a@example.com").await;
    let mut stream = t.state.live.subscribe(account);
    crate::watch::announce(&t.state, account, &read(vec![ticket("1", "To Do")]))
        .await
        .unwrap();

    t.relay_answers(&[503]);
    let moved = read(vec![ticket("1", "Done")]);
    assert!(
        !crate::watch::announce(&t.state, account, &moved)
            .await
            .unwrap()
    );
    assert!(
        crate::watch::announce(&t.state, account, &moved)
            .await
            .unwrap()
    );
    assert!(stream.try_recv().is_ok());
    assert!(
        stream.try_recv().is_err(),
        "the retry told the phone, not the stream again"
    );
}

/// An identity token as Apple would sign it, with the test key in
/// `testdata/` (made for these tests and nothing else).
fn apple_token(claims: Value) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    let der = STANDARD
        .decode(include_str!("testdata/apple-test-key.pkcs8.b64").trim())
        .unwrap();
    let key = ring::signature::RsaKeyPair::from_pkcs8(&der).unwrap();
    let head = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","kid":"test-key"}"#);
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let signed = format!("{head}.{body}");
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        signed.as_bytes(),
        &mut signature,
    )
    .unwrap();
    format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature))
}

fn apple_claims(sub: &str, email: Option<&str>, nonce: &str) -> Value {
    use sha2::Digest as _;
    let hashed: String = sha2::Sha256::digest(nonce.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    let mut c = json!({
        "iss": "https://appleid.apple.com", "aud": "dev.example.app", "exp": exp,
        "sub": sub, "nonce": hashed,
    });
    if let Some(e) = email {
        c["email"] = json!(e);
        c["email_verified"] = json!("true");
    }
    c
}

impl Test {
    async fn apple(&self, token: String, nonce: &str) -> (StatusCode, Value) {
        let body = json!({
            "identity_token": token, "nonce": nonce,
            "device": { "name": "iPhone", "platform": "ios" },
        });
        self.call(Method::POST, "/v1/sign-in/apple", None, Some(body))
            .await
    }
}

#[tokio::test]
async fn signs_in_with_apple_and_joins_the_account_with_the_same_address() {
    let Some(t) = service().await else { return };
    let by_email = t.sign_in("me@example.com", "Mac").await;

    let (status, apple) = t
        .apple(
            apple_token(apple_claims("001.apple", Some("Me@Example.com"), "n1")),
            "n1",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{apple}");
    assert_eq!(
        apple["account"], by_email["account"],
        "joined by the verified address"
    );

    // The same Apple ID again, whatever its address now: the same account.
    let (_, again) = t
        .apple(apple_token(apple_claims("001.apple", None, "n2")), "n2")
        .await;
    assert_eq!(again["account"], by_email["account"]);

    let (_, me) = t
        .call(
            Method::GET,
            "/v1/me",
            Some(token(&apple, "access_token")),
            None,
        )
        .await;
    let providers: Vec<&str> = me["sign_ins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["provider"].as_str().unwrap())
        .collect();
    assert_eq!(providers, ["email", "apple"]);
}

#[tokio::test]
async fn a_hidden_address_makes_an_account_of_its_own() {
    let Some(t) = service().await else { return };
    let relay = "abc@privaterelay.appleid.com";
    t.sign_in(relay, "Mac").await;
    let (_, apple) = t
        .apple(
            apple_token(apple_claims("002.apple", Some(relay), "n")),
            "n",
        )
        .await;
    let (_, mine) = t
        .call(
            Method::GET,
            "/v1/me",
            Some(token(&apple, "access_token")),
            None,
        )
        .await;
    assert_eq!(
        mine["sign_ins"].as_array().unwrap().len(),
        1,
        "not joined to the other"
    );
}

#[tokio::test]
async fn an_apple_token_that_does_not_check_out_is_refused() {
    let Some(t) = service().await else { return };
    let good = apple_claims("003.apple", None, "nonce");
    let cases = [
        // Someone else's app.
        (json!({ "aud": "dev.other.app" }), "nonce"),
        // Not from Apple.
        (json!({ "iss": "https://example.com" }), "nonce"),
        // Expired.
        (json!({ "exp": 1 }), "nonce"),
    ];
    for (patch, nonce) in cases {
        let mut claims = good.clone();
        for (k, v) in patch.as_object().unwrap() {
            claims[k] = v.clone();
        }
        let (status, _) = t.apple(apple_token(claims), nonce).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{patch}");
    }
    // Replayed with another nonce.
    let (status, _) = t.apple(apple_token(good.clone()), "another").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Tampered with after signing.
    let token = apple_token(good);
    let mut parts: Vec<String> = token.split('.').map(String::from).collect();
    use base64::Engine as _;
    parts[1] = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(apple_claims("someone.else", None, "nonce").to_string());
    let (status, _) = t.apple(parts.join("."), "nonce").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

const MANIFEST_APP: &str = r#"{"id":1,"slug":"hura-cloud","client_id":"Iv1.client","client_secret":"shh","webhook_secret":"hook-secret","pem":"-----","html_url":"https://github.com/apps/hura-cloud"}"#;

impl Test {
    /// Raw, for answers that are a redirect or a page.
    async fn get_raw(&self, path: &str) -> (StatusCode, String, String) {
        let request = Request::builder().uri(path).body(Body::empty()).unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get("location")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            location,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    async fn github_set_up(&self) {
        self.state.web.on(
            "POST https://api.github.com/app-manifests/abc123/conversions",
            201,
            MANIFEST_APP,
        );
        let (status, _, page) = self
            .get_raw("/setup/github/done?code=abc123&state=setup-token")
            .await;
        assert_eq!(status, StatusCode::OK, "{page}");
    }

    /// Back from a provider: the confirmation page, then its Connect (or
    /// Cancel). The status and where it sends the browser.
    async fn finish(&self, callback: &str, cancel: bool) -> (StatusCode, String, String) {
        let (status, location, page) = self.get_raw(callback).await;
        if status != StatusCode::OK {
            return (status, location, page);
        }
        let made = page
            .split("name=\"state\" value=\"")
            .nth(1)
            .and_then(|r| r.split('"').next())
            .unwrap_or_else(|| panic!("no confirmation in {page}"))
            .to_string();
        let form = if cancel {
            format!("state={made}&cancel=1")
        } else {
            format!("state={made}")
        };
        let request = Request::builder()
            .method(Method::POST)
            .uri("/v1/oauth/confirm")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(Body::from(form))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get("location")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            location,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    /// The state in a URL the service handed out.
    async fn connect(&self, at: &str, provider: &str) -> String {
        let (status, body) = self
            .call(
                Method::POST,
                &format!("/v1/connect/{provider}"),
                Some(at),
                Some(json!({ "return": "app" })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let url = body["url"].as_str().unwrap().to_string();
        url.split("state=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string()
    }
}

#[tokio::test]
async fn the_setup_page_is_only_for_whoever_has_the_token() {
    let Some(t) = service().await else { return };
    let (status, _, _) = t.get_raw("/setup?token=wrong").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, page) = t.get_raw("/setup?token=setup-token").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains("github.com/settings/apps/new"),
        "the manifest form"
    );
    assert!(
        page.contains("https://cloud.example/v1/webhooks/github"),
        "its webhook"
    );

    let (status, _, _) = t
        .get_raw("/setup/github/done?code=abc123&state=wrong")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    t.github_set_up().await;
    let kept: Vec<u8> = sqlx::query_scalar("select config from integrations where kind = 'github'")
        .fetch_one(&t.state.db)
        .await
        .unwrap();
    assert!(
        !kept.windows(3).any(|w| w == b"shh"),
        "the secret is encrypted"
    );
}

#[tokio::test]
async fn github_connects_as_you_and_installs_where_it_is_needed() {
    let Some(t) = service().await else { return };
    t.github_set_up().await;
    let me = t.sign_in("a@example.com", "iPhone").await;
    let at = token(&me, "access_token");
    let (_, can) = t.call(Method::GET, "/v1/connect", Some(at), None).await;
    assert_eq!(can, json!({ "github": true, "jira": false }));

    let web = &t.state.web;
    web.on(
        "POST https://github.com/login/oauth/access_token",
        200,
        r#"{"access_token":"ghu_access","refresh_token":"ghr_refresh","expires_in":28800}"#,
    );
    web.on(
        "GET https://api.github.com/user",
        200,
        r#"{"login":"OctoCat"}"#,
    );
    web.on(
        "GET https://api.github.com/user/installations",
        200,
        r#"{"total_count":0}"#,
    );

    let state = t.connect(at, "github").await;
    let (status, to, page) = t
        .finish(
            &format!("/v1/oauth/github/callback?code=c&state={state}"),
            false,
        )
        .await;
    assert_eq!(status, StatusCode::FOUND, "{page}");
    assert!(
        to.starts_with("https://github.com/apps/hura-cloud/installations/new?state="),
        "{to}"
    );

    // The same state does not work twice.
    let (status, _, _) = t
        .get_raw(&format!("/v1/oauth/github/callback?code=c&state={state}"))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Back from installing: into the app.
    let install = to.split("state=").nth(1).unwrap();
    let (_, to, _) = t
        .get_raw(&format!(
            "/v1/oauth/github/callback?installation_id=1&setup_action=install&state={install}"
        ))
        .await;
    assert_eq!(to, "hura://cloud/connected?provider=github");

    let (_, list) = t.call(Method::GET, "/v1/trackers", Some(at), None).await;
    assert_eq!(list[0]["source"]["name"], "github");
    assert_eq!(list[0]["connected"], "@OctoCat");
    assert_eq!(list[0]["token_set"], true);
    let (status, _) = t
        .call(
            Method::PUT,
            "/v1/trackers/github/token",
            Some(at),
            Some(json!({ "token": "x" })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "nothing to paste on a connected tracker"
    );
}

#[tokio::test]
async fn jira_connects_the_tracker_already_there_for_its_site() {
    let Some(t) = service().await else { return };
    let me = t.sign_in("a@example.com", "Mac").await;
    let at = token(&me, "access_token");
    let request = Request::builder()
        .method(Method::POST)
        .uri("/setup/jira?token=setup-token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(
            "client_id=jira-client&client_secret=jira-secret",
        ))
        .unwrap();
    assert_eq!(
        t.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );

    // A tracker with a pasted token and a filter, before connecting.
    let work = json!({ "source": {
        "kind": "jira", "name": "work", "site": "https://acme.atlassian.net/", "email": "a@example.com",
        "jql_filters": [{ "name": "ready", "jql": "status = Ready" }] }, "token": "pasted" });
    t.call(Method::POST, "/v1/trackers", Some(at), Some(work))
        .await;

    let web = &t.state.web;
    web.on(
        "POST https://auth.atlassian.com/oauth/token",
        200,
        r#"{"access_token":"jira-access","refresh_token":"jira-refresh-1","expires_in":3600}"#,
    );
    web.on(
        "GET https://api.atlassian.com/oauth/token/accessible-resources",
        200,
        r#"[{"id":"cloud-1","url":"https://acme.atlassian.net","name":"acme"}]"#,
    );

    let state = t.connect(at, "jira").await;
    let (_, to, _) = t
        .finish(
            &format!("/v1/oauth/jira/callback?code=c&state={state}"),
            false,
        )
        .await;
    assert_eq!(to, "hura://cloud/connected?provider=jira");

    let (_, list) = t.call(Method::GET, "/v1/trackers", Some(at), None).await;
    assert_eq!(
        list.as_array().unwrap().len(),
        1,
        "connected in place, not added"
    );
    assert_eq!(list[0]["source"]["name"], "work");
    assert_eq!(
        list[0]["source"]["jql_filters"][0]["name"], "ready",
        "its filters stay"
    );
    assert_eq!(
        list[0]["connected"], "acme.atlassian.net",
        "the site, not the person"
    );
    assert!(list[0]["source"].get("cloud_id").is_none(), "never shown");

    // Read with Atlassian's gateway.
    let account = me["account"].as_str().unwrap().parse().unwrap();
    let stored = crate::trackers::stored(&t.state, account).await.unwrap();
    assert_eq!(stored[0].source.cloud_id.as_deref(), Some("cloud-1"));
    assert_eq!(stored[0].token.as_deref(), Some("jira-access"));

    // A filter changed from the phone, as it sends it: untrimmed, with no
    // email and no cloud ID, both of which only the service knows about.
    let from_phone = json!({ "source": { "kind": "jira", "name": "work", "site": "https://acme.atlassian.net",
        "email": null, "jql_filters": [{ "name": " ready ", "jql": "status = “Ready”" }] } });
    let (status, body) = t
        .call(Method::PUT, "/v1/trackers/work", Some(at), Some(from_phone))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body[0]["source"]["jql_filters"][0],
        json!({ "name": "ready", "jql": "status = \"Ready\"" })
    );

    // Changed without an email, which the connection makes right.
    let edit = json!({ "source": { "kind": "jira", "name": "work", "site": "https://acme.atlassian.net",
        "hide_created": true } });
    let (status, body) = t
        .call(Method::PUT, "/v1/trackers/work", Some(at), Some(edit))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_token_about_to_run_out_is_renewed_before_it_is_read() {
    let Some(t) = service().await else { return };
    t.github_set_up().await;
    let me = t.sign_in("a@example.com", "iPhone").await;
    let at = token(&me, "access_token");
    let web = &t.state.web;
    web.on(
        "POST https://github.com/login/oauth/access_token",
        200,
        r#"{"access_token":"ghu_1","refresh_token":"ghr_1","expires_in":28800}"#,
    );
    web.on(
        "POST https://github.com/login/oauth/access_token",
        200,
        r#"{"access_token":"ghu_2","refresh_token":"ghr_2","expires_in":28800}"#,
    );
    web.on(
        "POST https://github.com/login/oauth/access_token",
        400,
        r#"{"error":"bad_refresh_token"}"#,
    );
    web.on(
        "GET https://api.github.com/user",
        200,
        r#"{"login":"octocat"}"#,
    );
    web.on(
        "GET https://api.github.com/user/installations",
        200,
        r#"{"total_count":1}"#,
    );
    let state = t.connect(at, "github").await;
    t.finish(
        &format!("/v1/oauth/github/callback?code=c&state={state}"),
        false,
    )
    .await;

    let account = me["account"].as_str().unwrap().parse().unwrap();
    let expire = || async {
        sqlx::query("update trackers set connection = jsonb_set(connection, '{expires_at}', '1')")
            .execute(&t.state.db)
            .await
            .unwrap();
    };
    expire().await;
    let stored = crate::trackers::stored(&t.state, account).await.unwrap();
    assert_eq!(stored[0].token.as_deref(), Some("ghu_2"), "renewed");
    let sent = web.sent("POST https://github.com/login/oauth/access_token");
    assert!(
        sent.last()
            .unwrap()
            .as_deref()
            .unwrap()
            .contains("refresh_token=ghr_1")
    );

    // Refused: the token goes, and connecting again is the way back.
    expire().await;
    let stored = crate::trackers::stored(&t.state, account).await.unwrap();
    assert_eq!(stored[0].token, None);
    let (_, list) = t.call(Method::GET, "/v1/trackers", Some(at), None).await;
    assert_eq!(list[0]["token_set"], false);
}

#[tokio::test]
async fn a_signed_github_delivery_makes_the_people_in_it_due() {
    let Some(t) = service().await else { return };
    t.github_set_up().await;
    let me = t.sign_in("a@example.com", "iPhone").await;
    let at = token(&me, "access_token");
    let web = &t.state.web;
    web.on(
        "POST https://github.com/login/oauth/access_token",
        200,
        r#"{"access_token":"ghu","expires_in":28800}"#,
    );
    web.on(
        "GET https://api.github.com/user",
        200,
        r#"{"login":"octocat"}"#,
    );
    web.on(
        "GET https://api.github.com/user/installations",
        200,
        r#"{"total_count":1}"#,
    );
    let state = t.connect(at, "github").await;
    t.finish(
        &format!("/v1/oauth/github/callback?code=c&state={state}"),
        false,
    )
    .await;
    sqlx::query("insert into watches (account_id, next_read_at) values ($1::uuid, now() + interval '1 hour')")
        .bind(me["account"].as_str().unwrap())
        .execute(&t.state.db)
        .await
        .unwrap();

    let body = r#"{"action":"review_requested","requested_reviewer":{"login":"OctoCat"},"sender":{"login":"someone"}}"#;
    let sign = |secret: &[u8]| {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret);
        let tag = ring::hmac::sign(&key, body.as_bytes());
        format!(
            "sha256={}",
            tag.as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    };
    let deliver = |signature: String| {
        Request::builder()
            .method(Method::POST)
            .uri("/v1/webhooks/github")
            .header("X-Hub-Signature-256", signature)
            .body(Body::from(body))
            .unwrap()
    };
    let refused = t
        .app
        .clone()
        .oneshot(deliver(sign(b"wrong")))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let taken = t
        .app
        .clone()
        .oneshot(deliver(sign(b"hook-secret")))
        .await
        .unwrap();
    assert_eq!(taken.status(), StatusCode::ACCEPTED);
    let due: bool = sqlx::query_scalar("select next_read_at <= now() from watches")
        .fetch_one(&t.state.db)
        .await
        .unwrap();
    assert!(due, "read at the watcher's next pass");
}

#[tokio::test]
async fn looking_at_the_inbox_on_one_device_quietly_tells_the_other_phones() {
    let Some(t) = service().await else { return };
    let (phone, _) = t.routed("a@example.com").await;
    let mac = t.sign_in("a@example.com", "Mac").await;
    let mac_at = token(&mac, "access_token");

    let (_, none) = t.call(Method::GET, "/v1/seen", Some(mac_at), None).await;
    assert_eq!(none, json!({ "items": [], "at": null }));

    let looked = json!({ "items": ["work/10002", " work/10001 ", "work/10001", "gh/7"] });
    let (status, seen) = t
        .call(Method::PUT, "/v1/seen", Some(mac_at), Some(looked.clone()))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        seen["items"],
        json!(["gh/7", "work/10001", "work/10002"]),
        "trimmed, sorted, once each"
    );
    assert!(seen["at"].as_i64().is_some());

    // The phone is told, with nothing to show.
    let sent = t.relay_sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1["kind"], "inbox_seen");
    assert!(sent[0].1.get("encrypted").is_none() && sent[0].1.get("body").is_none());

    // The same again says nothing; the phone reads it.
    t.call(Method::PUT, "/v1/seen", Some(mac_at), Some(looked))
        .await;
    assert_eq!(t.relay_sent().len(), 1);
    let (_, read) = t
        .call(
            Method::GET,
            "/v1/seen",
            Some(token(&phone, "access_token")),
            None,
        )
        .await;
    assert_eq!(read["items"], seen["items"]);

    // Looked at on the phone itself: nobody else to tell.
    let looked = json!({ "items": ["gh/7"] });
    t.call(
        Method::PUT,
        "/v1/seen",
        Some(token(&phone, "access_token")),
        Some(looked),
    )
    .await;
    assert_eq!(t.relay_sent().len(), 1);
}

#[tokio::test]
async fn nothing_is_connected_until_the_account_is_confirmed() {
    let Some(t) = service().await else { return };
    t.github_set_up().await;
    let me = t.sign_in("a@example.com", "iPhone").await;
    let at = token(&me, "access_token");
    let web = &t.state.web;
    web.on(
        "POST https://github.com/login/oauth/access_token",
        200,
        r#"{"access_token":"ghu","expires_in":28800}"#,
    );
    web.on(
        "GET https://api.github.com/user",
        200,
        r#"{"login":"victim"}"#,
    );
    web.on(
        "GET https://api.github.com/user/installations",
        200,
        r#"{"total_count":1}"#,
    );

    let state = t.connect(at, "github").await;
    let (status, _, page) = t
        .get_raw(&format!("/v1/oauth/github/callback?code=c&state={state}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, list) = t.call(Method::GET, "/v1/trackers", Some(at), None).await;
    assert_eq!(list, json!([]), "not before Connect");
    assert!(
        page.contains("a@example.com"),
        "says whose account it would join"
    );

    // Cancelled: nothing saved, and back to the app saying so.
    let state = t.connect(at, "github").await;
    let (_, to, _) = t
        .finish(
            &format!("/v1/oauth/github/callback?code=c&state={state}"),
            true,
        )
        .await;
    assert_eq!(to, "hura://cloud/connected?error=cancelled");
    let (_, list) = t.call(Method::GET, "/v1/trackers", Some(at), None).await;
    assert_eq!(list, json!([]));

    // A made-up confirmation does nothing.
    let forged = Request::builder()
        .method(Method::POST)
        .uri("/v1/oauth/confirm")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from("state=made-up"))
        .unwrap();
    assert_eq!(
        t.app.clone().oneshot(forged).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn jira_sites_from_one_consent_are_renewed_together() {
    let Some(t) = service().await else { return };
    let me = t.sign_in("a@example.com", "Mac").await;
    let at = token(&me, "access_token");
    let request = Request::builder()
        .method(Method::POST)
        .uri("/setup/jira?token=setup-token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from("client_id=c&client_secret=s"))
        .unwrap();
    t.app.clone().oneshot(request).await.unwrap();
    let web = &t.state.web;
    web.on(
        "POST https://auth.atlassian.com/oauth/token",
        200,
        r#"{"access_token":"a1","refresh_token":"r1","expires_in":3600}"#,
    );
    web.on(
        "POST https://auth.atlassian.com/oauth/token",
        200,
        r#"{"access_token":"a2","refresh_token":"r2","expires_in":3600}"#,
    );
    web.on("GET https://api.atlassian.com/oauth/token/accessible-resources", 200,
        r#"[{"id":"one","url":"https://one.atlassian.net"},{"id":"two","url":"https://two.atlassian.net"}]"#);
    let state = t.connect(at, "jira").await;
    t.finish(
        &format!("/v1/oauth/jira/callback?code=c&state={state}"),
        false,
    )
    .await;

    sqlx::query("update trackers set connection = jsonb_set(connection, '{expires_at}', '1')")
        .execute(&t.state.db)
        .await
        .unwrap();
    let account = me["account"].as_str().unwrap().parse().unwrap();
    let stored = crate::trackers::stored(&t.state, account).await.unwrap();
    assert_eq!(stored.len(), 2);
    assert!(
        stored.iter().all(|s| s.token.as_deref() == Some("a2")),
        "both renewed: {stored:?}"
    );
    // One renewal for the two, with the one refresh token.
    let refreshes: Vec<_> = web
        .sent("POST https://auth.atlassian.com/oauth/token")
        .into_iter()
        .flatten()
        .filter(|b| b.contains("refresh_token"))
        .collect();
    assert_eq!(refreshes.len(), 1, "{refreshes:?}");
    assert!(refreshes[0].contains("\"r1\""));
}

#[tokio::test]
async fn a_provider_that_is_away_does_not_cost_the_connection() {
    let Some(t) = service().await else { return };
    t.github_set_up().await;
    let me = t.sign_in("a@example.com", "iPhone").await;
    let at = token(&me, "access_token");
    let web = &t.state.web;
    web.on(
        "POST https://github.com/login/oauth/access_token",
        200,
        r#"{"access_token":"ghu_1","refresh_token":"ghr_1","expires_in":28800}"#,
    );
    web.on(
        "POST https://github.com/login/oauth/access_token",
        503,
        "busy",
    );
    web.on(
        "GET https://api.github.com/user",
        200,
        r#"{"login":"octocat"}"#,
    );
    web.on(
        "GET https://api.github.com/user/installations",
        200,
        r#"{"total_count":1}"#,
    );
    let state = t.connect(at, "github").await;
    t.finish(
        &format!("/v1/oauth/github/callback?code=c&state={state}"),
        false,
    )
    .await;
    sqlx::query("update trackers set connection = jsonb_set(connection, '{expires_at}', '1')")
        .execute(&t.state.db)
        .await
        .unwrap();
    let account = me["account"].as_str().unwrap().parse().unwrap();
    let stored = crate::trackers::stored(&t.state, account).await.unwrap();
    assert_eq!(
        stored[0].token.as_deref(),
        Some("ghu_1"),
        "kept, to be renewed at the next read"
    );
}

#[tokio::test]
async fn a_read_asked_for_while_one_runs_is_not_put_off() {
    let Some(t) = service().await else { return };
    t.routed("a@example.com").await;
    // As a webhook does while the account is leased for a read.
    sqlx::query("update watches set next_read_at = now() - interval '1 minute'")
        .execute(&t.state.db)
        .await
        .unwrap();
    assert_eq!(crate::watch::tick(&t.state).await.unwrap(), 1);
    sqlx::query("update watches set next_read_at = now()")
        .execute(&t.state.db)
        .await
        .unwrap();
    // The read that was running puts itself back in line: the earlier time stays.
    let wait: f64 =
        sqlx::query_scalar("select extract(epoch from next_read_at - now())::float8 from watches")
            .fetch_one(&t.state.db)
            .await
            .unwrap();
    assert!(wait <= 1.0, "{wait}");
}

#[tokio::test]
async fn a_signed_fizzy_delivery_makes_its_webhooks_account_due() {
    let Some(t) = service().await else { return };
    let me = t.sign_in("a@example.com", "iPhone").await;
    let at = token(&me, "access_token");
    let account: uuid::Uuid = me["account"].as_str().unwrap().parse().unwrap();
    // No token, so nothing is sent to Fizzy: the webhook is put in by hand.
    let (status, list) = t
        .call(
            Method::POST,
            "/v1/trackers",
            Some(at),
            Some(json!({ "source": { "kind": "fizzy", "name": "cards" } })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert!(
        list[0].get("webhooks").is_none(),
        "not looked at without a token"
    );
    let id = crate::token::id().unwrap();
    let key = t.state.keys.account(&t.state.db, account).await.unwrap();
    sqlx::query(
        "insert into fizzy_hooks (id, account_id, tracker, fizzy_account, board_id, webhook_id, secret)
         values ($1, $2, 'cards', '897362094', 'b1', 'w1', $3)",
    )
    .bind(id)
    .bind(account)
    .bind(key.seal(&crate::fizzy_hooks::label(id), b"s3cret").unwrap())
    .execute(&t.state.db)
    .await
    .unwrap();
    sqlx::query(
        "insert into watches (account_id, next_read_at) values ($1, now() + interval '1 hour')
         on conflict (account_id) do update set next_read_at = now() + interval '1 hour'",
    )
    .bind(account)
    .execute(&t.state.db)
    .await
    .unwrap();

    let body = r#"{"id":"e1","action":"comment_created","eventable":{"id":"c1"}}"#;
    let sign = |secret: &[u8]| {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret);
        ring::hmac::sign(&key, body.as_bytes())
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let deliver = |to: uuid::Uuid, signature: String| {
        Request::builder()
            .method(Method::POST)
            .uri(format!("/v1/webhooks/fizzy/{to}"))
            .header("X-Webhook-Signature", signature)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    let send = |r| t.app.clone().oneshot(r);
    assert_eq!(
        send(deliver(id, sign(b"wrong"))).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(deliver(crate::token::id().unwrap(), sign(b"s3cret")))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND,
        "a webhook that is not one of ours"
    );
    let due = || async {
        sqlx::query_scalar::<_, bool>(
            "select next_read_at <= now() from watches where account_id = $1",
        )
        .bind(account)
        .fetch_one(&t.state.db)
        .await
        .unwrap()
    };
    assert!(!due().await);
    assert_eq!(
        send(deliver(id, sign(b"s3cret"))).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    assert!(due().await, "read at the watcher's next pass");

    // Gone with its tracker.
    let (status, _) = t
        .call(Method::DELETE, "/v1/trackers/cards", Some(at), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let left: i64 = sqlx::query_scalar("select count(*) from fizzy_hooks")
        .fetch_one(&t.state.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn a_connected_jira_site_gets_a_webhook_and_its_deliveries_make_yours_due() {
    let Some(t) = service().await else { return };
    let me = t.sign_in("a@example.com", "iPhone").await;
    let at = token(&me, "access_token");
    let account: uuid::Uuid = me["account"].as_str().unwrap().parse().unwrap();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/setup/jira?token=setup-token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(
            "client_id=jira-client&client_secret=jira-secret",
        ))
        .unwrap();
    t.app.clone().oneshot(request).await.unwrap();

    let web = &t.state.web;
    let api = "https://api.atlassian.com/ex/jira/cloud-1/rest/api/3";
    web.on(
        "POST https://auth.atlassian.com/oauth/token",
        200,
        r#"{"access_token":"jira-access","refresh_token":"jira-refresh-1","expires_in":3600}"#,
    );
    web.on(
        "GET https://api.atlassian.com/oauth/token/accessible-resources",
        200,
        r#"[{"id":"cloud-1","url":"https://acme.atlassian.net","name":"acme"}]"#,
    );
    web.on(&format!("GET {api}/myself"), 200, r#"{"accountId":"me-1"}"#);
    // A leftover of ours from before, and somebody else's app's address.
    web.on(
        &format!("GET {api}/webhook"),
        200,
        r#"{"isLast":true,"values":[{"id":7,"url":"https://cloud.example/v1/webhooks/jira"},{"id":8,"url":"https://other.example/hook"}]}"#,
    );
    // The next look's.
    web.on(
        &format!("GET {api}/webhook"),
        200,
        r#"{"isLast":true,"values":[{"id":1000,"url":"https://cloud.example/v1/webhooks/jira"}]}"#,
    );
    web.on(
        &format!("POST {api}/webhook"),
        200,
        r#"{"webhookRegistrationResult":[{"createdWebhookId":1000}]}"#,
    );
    web.on(&format!("DELETE {api}/webhook"), 202, "");

    let state = t.connect(at, "jira").await;
    t.finish(
        &format!("/v1/oauth/jira/callback?code=c&state={state}"),
        false,
    )
    .await;
    let made = web.sent(&format!("POST {api}/webhook"));
    assert_eq!(made.len(), 1);
    let made: Value = serde_json::from_str(made[0].as_deref().unwrap()).unwrap();
    assert_eq!(made["url"], "https://cloud.example/v1/webhooks/jira");
    assert_eq!(
        made["webhooks"][0]["jqlFilter"], "",
        "every issue: matched here"
    );
    let removed = web.sent(&format!("DELETE {api}/webhook"));
    assert_eq!(
        removed,
        vec![Some(r#"{"webhookIds":[7]}"#.to_string())],
        "the leftover, not the other app's"
    );
    let (_, list) = t.call(Method::GET, "/v1/trackers", Some(at), None).await;
    assert_eq!(
        list[0]["webhooks"],
        "a webhook tells hura of changes at once"
    );

    // Looked at again: still there, so renewed rather than made again.
    web.on(
        &format!("PUT {api}/webhook/refresh"),
        200,
        r#"{"expirationDate":1900000000000}"#,
    );
    crate::jira_hooks::sync(&t.state, account, "jira")
        .await
        .unwrap();
    assert_eq!(web.sent(&format!("POST {api}/webhook")).len(), 1);
    assert_eq!(
        web.sent(&format!("PUT {api}/webhook/refresh")),
        vec![Some(r#"{"webhookIds":[1000]}"#.to_string())]
    );

    sqlx::query(
        "insert into watches (account_id, next_read_at) values ($1, now() + interval '1 hour')
         on conflict (account_id) do update set next_read_at = now() + interval '1 hour'",
    )
    .bind(account)
    .execute(&t.state.db)
    .await
    .unwrap();
    let due = || async {
        sqlx::query_scalar::<_, bool>(
            "select next_read_at <= now() from watches where account_id = $1",
        )
        .bind(account)
        .fetch_one(&t.state.db)
        .await
        .unwrap()
    };
    let jwt = |secret: &[u8]| {
        use base64::Engine as _;
        let b64 = |s: String| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        let signed = format!(
            "{}.{}",
            b64(json!({ "alg": "HS256", "typ": "JWT" }).to_string()),
            b64(json!({ "iss": "atlassian" }).to_string())
        );
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret);
        let tag = ring::hmac::sign(&key, signed.as_bytes());
        format!(
            "{signed}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tag.as_ref())
        )
    };
    let deliver = |token: String, body: Value| {
        Request::builder()
            .method(Method::POST)
            .uri("/v1/webhooks/jira")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let send = |r| t.app.clone().oneshot(r);
    let somebody_elses = json!({ "webhookEvent": "jira:issue_updated", "matchedWebhookIds": [1000],
        "issue": { "key": "ACME-2", "fields": { "assignee": { "accountId": "you-2" } } } });
    let mine = json!({ "webhookEvent": "jira:issue_updated", "matchedWebhookIds": [1000],
        "issue": { "key": "ACME-1", "fields": { "assignee": { "accountId": "me-1" } } } });

    assert_eq!(
        send(deliver(jwt(b"wrong"), mine.clone()))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(deliver(jwt(b"jira-secret"), somebody_elses))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(
        !due().await,
        "an issue that is not yours waits for the timer"
    );
    assert_eq!(
        send(deliver(jwt(b"jira-secret"), mine))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(due().await, "read at the watcher's next pass");

    // Somebody else's issue, in a filter of the tracker's: queued, then asked
    // of Jira with every filter at once, and due when one holds it.
    let filters = json!({ "source": { "kind": "jira", "name": "jira", "site": "https://acme.atlassian.net",
        "jql_filters": [{ "name": "ready", "jql": "status = Ready ORDER BY rank" },
                        { "name": "mine", "jql": "labels = team" }] } });
    let (status, body) = t
        .call(Method::PUT, "/v1/trackers/jira", Some(at), Some(filters))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let _stream = t.state.live.subscribe(account);
    let put_off = || async {
        sqlx::query(
            "update watches set next_read_at = now() + interval '1 hour' where account_id = $1",
        )
        .bind(account)
        .execute(&t.state.db)
        .await
        .unwrap();
    };
    let settle = || async {
        sqlx::query("update jira_checks set queued_at = now() - interval '1 minute'")
            .execute(&t.state.db)
            .await
            .unwrap();
    };
    put_off().await;
    let in_a_filter = |key: &str| {
        json!({ "webhookEvent": "jira:issue_updated", "matchedWebhookIds": [1000],
        "issue": { "key": key, "fields": { "assignee": { "accountId": "you-2" } } } })
    };
    for key in ["ACME-3", "ACME-4", "ACME-3"] {
        let r = send(deliver(jwt(b"jira-secret"), in_a_filter(key)))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
    }
    assert!(!due().await, "not before Jira is asked");
    let search = format!("POST {api}/search/jql");
    web.on(&search, 200, r#"{"issues":[]}"#);
    web.on(&search, 200, r#"{"issues":[{"key":"ACME-4"}]}"#);
    let pass = crate::jira_filters::tick(&t.state).await.unwrap();
    assert_eq!(pass.issues, 0, "nothing has waited long enough");
    settle().await;
    let pass = crate::jira_filters::tick(&t.state).await.unwrap();
    assert_eq!(
        (pass.issues, pass.searches, pass.due),
        (2, 1, 0),
        "one search for both"
    );
    let asked: Value = serde_json::from_str(web.sent(&search)[0].as_deref().unwrap()).unwrap();
    assert_eq!(
        asked["jql"],
        "issuekey in (ACME-3, ACME-4) AND ((status = Ready) OR (labels = team))"
    );
    assert!(!due().await, "in none of them");
    send(deliver(jwt(b"jira-secret"), in_a_filter("ACME-4")))
        .await
        .unwrap();
    settle().await;
    let pass = crate::jira_filters::tick(&t.state).await.unwrap();
    assert_eq!(pass.due, 1);
    assert!(due().await, "a filter holds it");

    // Gone with its tracker, off the site too.
    let (status, _) = t
        .call(Method::DELETE, "/v1/trackers/jira", Some(at), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        web.sent(&format!("DELETE {api}/webhook"))
            .last()
            .cloned()
            .flatten(),
        Some(r#"{"webhookIds":[1000]}"#.to_string())
    );
    let left: i64 = sqlx::query_scalar("select count(*) from jira_hooks")
        .fetch_one(&t.state.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}
