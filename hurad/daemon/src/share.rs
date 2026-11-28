//! Previews at an address a browser can open: a port inside a sandbox, served
//! by this server over HTTPS, for a phone to hand to Safari.
//!
//! A client's own preview (`previews.rs` on the desktop, `PreviewHub` on the
//! phone) is a listener on its loopback, tunnelled here over the paired
//! connection. That needs the client running, and iOS stops an app's listeners
//! soon after it leaves the screen -- so a page handed from the phone to Safari
//! loaded and then stopped answering. A shared preview is the same forward
//! into the sandbox ([`crate::forward`]), reached instead through a listener
//! of its own on this server, on the address `hurad serve` listens on.
//!
//! **One listener per port, at the root.** A dev server writes its pages for
//! `/`: its scripts are `/@vite/client` and `/src/main.ts`, and a preview under
//! a path of this server's would break every one of them. So each shared port
//! gets a port of this server's, and the page is at its root.
//!
//! **Behind a token, and then a cookie.** The link carries a token, made for
//! this share and compared in constant time; the first request swaps it for a
//! cookie (`Secure`, `HttpOnly`) and is sent on without it, so the token is in
//! the address bar once. A request with neither is refused. The cookie is
//! named for the listener's port, because a browser keeps one cookie jar per
//! host whatever the port, and two previews on one server must not share one.
//!
//! **This server's certificate.** The same self-signed one the clients pin.
//! Safari cannot pin, so it warns, once per preview, and is told to go on;
//! the user chose that over plain HTTP, which would put the page and the
//! token on the network in the clear.
//!
//! **The page thinks it is on localhost.** Each request goes to the service
//! with `Host` (and `Origin`, when there is one) saying `localhost:<port>`,
//! because a dev server refuses a host it does not know -- Vite's
//! `allowedHosts` -- and a redirect to `http://localhost:<port>/...` comes
//! back as a path. Websockets are passed through, so a dev server's reload
//! works.
//!
//! **Gone when unused.** A share nothing has asked of for [`IDLE`] is stopped,
//! and asking for it again makes a new one with a new token.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use hura_core::ports::Loopback;
use hura_core::session::Session;
use hura_proto::SharedPreview;
use hyper_util::rt::TokioIo;
use rand::TryRng as _;

/// How long a share may go unasked before it is stopped.
const IDLE: Duration = Duration::from_secs(30 * 60);

/// How often shares are looked over for ones gone idle.
const SWEEP: Duration = Duration::from_secs(60);

/// The query parameter a link carries its token in.
pub const PARAM: &str = "hura_preview";

/// Where shares listen, and with what: what `hurad serve` listens on and
/// presents, set once as it starts. Nothing can be shared before then.
struct Listening {
    bind: IpAddr,
    cert_pem: String,
    key_pem: String,
}

static LISTENING: OnceLock<Listening> = OnceLock::new();

/// Called by `serve` before it takes requests.
pub fn listen_on(bind: IpAddr, cert_pem: &str, key_pem: &str) {
    let _ = LISTENING.set(Listening {
        bind,
        cert_pem: cert_pem.to_string(),
        key_pem: key_pem.to_string(),
    });
}

/// A share that is running.
struct Running {
    port: u16,
    token: String,
    used: Arc<Mutex<Instant>>,
    handle: axum_server::Handle<SocketAddr>,
}

type Key = (String, u16, Loopback);

fn shares() -> &'static Mutex<HashMap<Key, Running>> {
    static SHARES: OnceLock<Mutex<HashMap<Key, Running>>> = OnceLock::new();
    SHARES.get_or_init(Mutex::default)
}

/// The share of `port` in the session's sandbox, started if there is none.
pub async fn share(session: &Session, port: u16, host: Loopback) -> Result<SharedPreview, String> {
    let listening = LISTENING
        .get()
        .ok_or("this server is not listening, so it cannot share a preview")?;
    let key = (session.sandbox.clone(), port, host);
    {
        let all = shares()
            .lock()
            .map_err(|_| "the share registry is poisoned")?;
        if let Some(running) = all.get(&key) {
            touch(&running.used);
            return Ok(SharedPreview {
                port: running.port,
                token: running.token.clone(),
            });
        }
    }

    let listener = std::net::TcpListener::bind(SocketAddr::new(listening.bind, 0))
        .map_err(|e| format!("could not listen for the preview: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("could not listen for the preview: {e}"))?;
    let local = listener
        .local_addr()
        .map_err(|e| format!("could not listen for the preview: {e}"))?
        .port();
    let config = axum_server::tls_rustls::RustlsConfig::from_pem(
        listening.cert_pem.clone().into_bytes(),
        listening.key_pem.clone().into_bytes(),
    )
    .await
    .map_err(|e| format!("could not load this server's certificate: {e}"))?;

    let token = new_token()?;
    let used = Arc::new(Mutex::new(Instant::now()));
    let target = Arc::new(Target {
        session: session.clone(),
        port,
        host,
        token: token.clone(),
        cookie: format!("{PARAM}_{local}"),
        used: used.clone(),
    });
    let app = axum::Router::new().fallback(proxy).with_state(target);
    let handle = axum_server::Handle::new();
    let server = axum_server::tls_rustls::from_tcp_rustls(listener, config)
        .map_err(|e| format!("could not listen for the preview: {e}"))?
        .handle(handle.clone());
    tokio::spawn(async move {
        let _ = server.serve(app.into_make_service()).await;
    });
    start_sweeping();

    shares()
        .lock()
        .map_err(|_| "the share registry is poisoned")?
        .insert(
            key,
            Running {
                port: local,
                token: token.clone(),
                used,
                handle,
            },
        );
    Ok(SharedPreview { port: local, token })
}

/// Every share of a sandbox's port stopped, as its forward is.
pub fn stop(sandbox: &str, port: u16) {
    let Ok(mut all) = shares().lock() else { return };
    all.retain(|(s, p, _), running| {
        let keep = !(s == sandbox && *p == port);
        if !keep {
            running.handle.shutdown();
        }
        keep
    });
}

/// Started with the first share, and looking over them from then on.
fn start_sweeping() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    tokio::spawn(async {
        loop {
            tokio::time::sleep(SWEEP).await;
            let Ok(mut all) = shares().lock() else {
                continue;
            };
            all.retain(|_, running| {
                let idle = running
                    .used
                    .lock()
                    .map(|at| at.elapsed() >= IDLE)
                    .unwrap_or(true);
                if idle {
                    running.handle.shutdown();
                }
                !idle
            });
        }
    });
}

fn touch(used: &Mutex<Instant>) {
    if let Ok(mut at) = used.lock() {
        *at = Instant::now();
    }
}

/// Thirty-two bytes from the OS, as hex, as a pairing token is made.
fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .map_err(|e| format!("could not make a token: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// What one share's requests go to.
struct Target {
    session: Session,
    port: u16,
    host: Loopback,
    token: String,
    /// The cookie it swaps the token for.
    cookie: String,
    used: Arc<Mutex<Instant>>,
}

async fn proxy(State(target): State<Arc<Target>>, mut req: Request) -> Response {
    touch(&target.used);

    // The link: its token for a cookie, and the same address without it.
    if let Some(given) = query_value(req.uri().query(), PARAM) {
        if !same(&given, &target.token) {
            return refused();
        }
        let rest = without_param(req.uri().query(), PARAM);
        let location = match rest {
            Some(q) => format!("{}?{q}", req.uri().path()),
            None => req.uri().path().to_string(),
        };
        let cookie = format!(
            "{}={}; Path=/; Secure; HttpOnly; SameSite=Lax",
            target.cookie, target.token
        );
        return Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(header::LOCATION, location)
            .header(header::SET_COOKIE, cookie)
            .body(Body::empty())
            .unwrap_or_else(|_| refused());
    }
    match cookie_value(req.headers(), &target.cookie) {
        Some(given) if same(&given, &target.token) => {}
        _ => return refused(),
    }

    let lease = match crate::forward::acquire(&target.session, target.port, target.host).await {
        Ok(lease) => lease,
        Err(why) => return unreachable(target.port, &why),
    };
    let stream = match lease.connect().await {
        Ok(stream) => stream,
        Err(e) => return unreachable(target.port, &e.to_string()),
    };
    let (mut sender, conn) = match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await
    {
        Ok(pair) => pair,
        Err(e) => return unreachable(target.port, &e.to_string()),
    };
    // The connection holds the lease, so the forward stays up while either
    // the request or an upgraded websocket is using it.
    tokio::spawn(async move {
        let _ = conn.with_upgrades().await;
        drop(lease);
    });

    let wants_upgrade = req.headers().contains_key(header::UPGRADE);
    let client_side = wants_upgrade.then(|| hyper::upgrade::on(&mut req));
    let mut out = req;
    as_local(out.headers_mut(), target.port, &target.cookie);
    // The path alone, as HTTP/1.1 asks for a page. A browser speaking HTTP/2
    // here, as Safari does, names the whole address, and the dev server was
    // handed `https://box.lan:62278/page` as the path it was asked for.
    *out.uri_mut() = origin_form(out.uri());
    *out.version_mut() = axum::http::Version::HTTP_11;

    let mut answer = match sender.send_request(out).await {
        Ok(answer) => answer,
        Err(e) => return unreachable(target.port, &e.to_string()),
    };
    if answer.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(client_side) = client_side
    {
        let service_side = hyper::upgrade::on(&mut answer);
        tokio::spawn(async move {
            if let (Ok(client), Ok(service)) = (client_side.await, service_side.await) {
                let _ = tokio::io::copy_bidirectional(
                    &mut TokioIo::new(client),
                    &mut TokioIo::new(service),
                )
                .await;
            }
        });
    }
    let local = format!("http://localhost:{}", target.port);
    if let Some(location) = answer
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        && let Some(path) = location.strip_prefix(&local)
        && let Ok(path) = HeaderValue::from_str(if path.is_empty() { "/" } else { path })
    {
        answer.headers_mut().insert(header::LOCATION, path);
    }
    answer.map(Body::new)
}

/// A request's headers, as the service expects to be asked: from
/// `localhost:<port>`, and without the share's own cookie.
fn as_local(headers: &mut HeaderMap, port: u16, cookie: &str) {
    if let Ok(host) = HeaderValue::from_str(&format!("localhost:{port}")) {
        headers.insert(header::HOST, host);
    }
    if headers.contains_key(header::ORIGIN)
        && let Ok(origin) = HeaderValue::from_str(&format!("http://localhost:{port}"))
    {
        headers.insert(header::ORIGIN, origin);
    }
    let kept: Vec<String> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .filter(|c| !c.is_empty() && c.split('=').next() != Some(cookie))
        .map(String::from)
        .collect();
    headers.remove(header::COOKIE);
    if !kept.is_empty()
        && let Ok(value) = HeaderValue::from_str(&kept.join("; "))
    {
        headers.insert(header::COOKIE, value);
    }
}

/// An address as a path and query, without its scheme and host.
fn origin_form(uri: &axum::http::Uri) -> axum::http::Uri {
    let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    path.parse()
        .unwrap_or_else(|_| axum::http::Uri::from_static("/"))
}

/// One query parameter's value, as it was written.
fn query_value(query: Option<&str>, name: &str) -> Option<String> {
    query?
        .split('&')
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
        .map(String::from)
}

/// The query without one parameter, or nothing if that was all of it.
fn without_param(query: Option<&str>, name: &str) -> Option<String> {
    let rest: Vec<&str> = query?
        .split('&')
        .filter(|pair| pair.split('=').next() != Some(name) && !pair.is_empty())
        .collect();
    (!rest.is_empty()).then(|| rest.join("&"))
}

/// One cookie's value, from every `Cookie` header.
fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| c.trim().strip_prefix(name)?.strip_prefix('='))
        .map(String::from)
}

/// Two tokens compared in constant time.
fn same(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq as _;
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

fn refused() -> Response {
    page(
        StatusCode::FORBIDDEN,
        "This preview opens from its link",
        "Open it from hura again: the link carries what lets this browser in.",
    )
}

fn unreachable(port: u16, why: &str) -> Response {
    page(
        StatusCode::BAD_GATEWAY,
        &format!("Nothing answered on port {port}"),
        &format!("The sandbox could not be reached on it: {why}"),
    )
}

/// A page saying why, in words a browser shows.
fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
         <title>{t}</title><body style=\"font:16px -apple-system,system-ui;margin:3em 1.5em;color:#18181b\">\
         <h1 style=\"font-size:20px\">{t}</h1><p style=\"color:#71717b\">{b}</p>",
        t = escape(title),
        b = escape(body)
    );
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_gives_its_token_and_the_address_without_it() {
        let q = Some("a=1&hura_preview=abc&b=2");
        assert_eq!(query_value(q, PARAM).as_deref(), Some("abc"));
        assert_eq!(without_param(q, PARAM).as_deref(), Some("a=1&b=2"));
        assert_eq!(without_param(Some("hura_preview=abc"), PARAM), None);
        assert_eq!(query_value(Some("hura_preview_x=abc"), PARAM), None);
    }

    #[test]
    fn the_service_is_asked_from_localhost_without_the_shares_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("box.lan:41234"));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://box.lan:41234"),
        );
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("theme=dark; hura_preview_41234=t0k; sid=1"),
        );
        assert_eq!(
            cookie_value(&headers, "hura_preview_41234").as_deref(),
            Some("t0k")
        );
        as_local(&mut headers, 5173, "hura_preview_41234");
        assert_eq!(headers[header::HOST], "localhost:5173");
        assert_eq!(headers[header::ORIGIN], "http://localhost:5173");
        assert_eq!(headers[header::COOKIE], "theme=dark; sid=1");
    }

    /// What HTTP/2 names in full goes on as the path it is.
    #[test]
    fn the_service_is_asked_for_a_path_not_an_address() {
        let full: axum::http::Uri = "https://box.lan:62278/page?x=1".parse().unwrap();
        assert_eq!(origin_form(&full), "/page?x=1");
        assert_eq!(origin_form(&"https://box.lan:62278".parse().unwrap()), "/");
    }

    #[test]
    fn only_the_shares_own_cookie_lets_a_request_in() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("hura_preview_1=t0k"),
        );
        assert_eq!(cookie_value(&headers, "hura_preview_2"), None);
        assert!(same("t0k", "t0k"));
        assert!(!same("t0k", "t0K"));
        assert!(!same("t0k", "t0k0"));
    }

    #[test]
    fn a_token_is_long_and_new_each_time() {
        let a = new_token().unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, new_token().unwrap());
    }
}
