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
