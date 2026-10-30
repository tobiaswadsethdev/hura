//! Notification text encrypted end to end, so the relay and Apple see only
//! that something happened and the phone shows what.
//!
//! `docs/relay-api.md`, "Encrypted text": AES-256-GCM under the client's
//! 32-byte key, a fresh 12-byte nonce per attempt, and additional data that
//! ties the ciphertext to its server, kind, session and request so the relay
//! cannot move it onto another notification. `push-crypto.json` beside this
//! file is the test vector; the app's tests read the same file, so the two
//! ends cannot drift apart.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::TryRng;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use serde::Serialize;

use super::Notification;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// The longest `encrypted` the relay accepts, which keeps the APNs payload under
/// its 4 KB limit.
const MAX_ENCRYPTED: usize = 3000;

/// Decode a `push_key`: 32 bytes, base64url without padding.
pub fn parse_key(text: &str) -> Result<[u8; KEY_LEN], String> {
    URL_SAFE_NO_PAD
        .decode(text)
        .ok()
        .and_then(|k| <[u8; KEY_LEN]>::try_from(k).ok())
        .ok_or_else(|| "push_key must be 32 bytes in base64url without padding".to_string())
}

/// Seal a notification's text with a fresh random nonce.
pub fn encrypt(key: &[u8; KEY_LEN], server_id: &str, n: &Notification) -> Result<String, String> {
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::SysRng
        .try_fill_bytes(&mut nonce)
        .map_err(|e| format!("no randomness for a nonce: {e}"))?;
    seal(key, nonce, server_id, n)
}

/// What gets encrypted. Field order and the absent empty subtitle are part of
/// the test vector, so this is a struct rather than a map.
#[derive(Serialize)]
struct Text<'a> {
    title: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    subtitle: &'a str,
    body: &'a str,
}

/// The fields the ciphertext is bound to, absent ones empty.
fn additional_data(server_id: &str, n: &Notification) -> String {
    format!(
        "hura-push-v1\n{server_id}\n{}\n{}\n{}",
        n.kind,
        n.session.as_deref().unwrap_or_default(),
        n.ask_id.as_deref().unwrap_or_default(),
    )
}

/// Cut to at most `max` code points, ending a cut string with `…`, as the
/// relay clips plaintext.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn seal(
    key: &[u8; KEY_LEN],
    nonce: [u8; NONCE_LEN],
    server_id: &str,
    n: &Notification,
) -> Result<String, String> {
    let sealer = LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, key).map_err(|_| "not an AES-256 key".to_string())?,
    );
    let aad = additional_data(server_id, n);
    let title = clip(&n.title, 100);
    let subtitle = clip(&n.subtitle, 100);
    let mut limit = 500;
    loop {
        let body = clip(&n.body, limit);
        let mut sealed = serde_json::to_vec(&Text {
            title: &title,
            subtitle: &subtitle,
            body: &body,
        })
        .map_err(|e| e.to_string())?;
        sealer
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad.as_bytes()),
                &mut sealed,
            )
            .map_err(|_| "could not encrypt the notification".to_string())?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&sealed);
        let encoded = URL_SAFE_NO_PAD.encode(out);
        // A body of many multi-byte characters can be 500 code points and
        // still too long once encrypted and encoded: shorten until it fits.
        if encoded.len() <= MAX_ENCRYPTED || limit <= 1 {
            return Ok(encoded);
        }
        limit /= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    /// One case of `push-crypto.json`.
    #[derive(Deserialize)]
    struct Vector {
        name: String,
        key: String,
        #[serde(default)]
        nonce: Option<String>,
        server_id: String,
        kind: String,
        #[serde(default)]
        session: Option<String>,
        #[serde(default)]
        ask_id: Option<String>,
        #[serde(default)]
        title: String,
        #[serde(default)]
        subtitle: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        plaintext: Option<String>,
        encrypted: String,
        /// Sealed for another notification, so it must not open as this one.
        #[serde(default)]
        fails: bool,
    }

    fn vectors() -> Vec<Vector> {
        serde_json::from_str(include_str!("push-crypto.json")).unwrap()
    }

    fn notification(v: &Vector) -> Notification {
        Notification {
            event_id: "e".into(),
            kind: v.kind.clone(),
            title: v.title.clone(),
            subtitle: v.subtitle.clone(),
            body: v.body.clone(),
            session: v.session.clone(),
            ask_id: v.ask_id.clone(),
        }
    }

    /// The reverse of [`seal`], which only the phone needs; here to check the
    /// vectors that are meant not to open.
    fn open(
        key: &[u8; KEY_LEN],
        encrypted: &str,
        server_id: &str,
        n: &Notification,
    ) -> Option<String> {
        let raw = URL_SAFE_NO_PAD.decode(encrypted).ok()?;
        let (nonce, sealed) = raw.split_at(NONCE_LEN);
        let opener = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
        let mut buf = sealed.to_vec();
        let plain = opener
            .open_in_place(
                Nonce::try_assume_unique_for_key(nonce).ok()?,
                Aad::from(additional_data(server_id, n).as_bytes()),
                &mut buf,
            )
            .ok()?;
        String::from_utf8(plain.to_vec()).ok()
    }

    #[test]
    fn agrees_with_the_vectors() {
        for v in vectors() {
            let key = parse_key(&v.key).unwrap();
            let n = notification(&v);
            if v.fails {
                assert_eq!(
                    open(&key, &v.encrypted, &v.server_id, &n),
                    None,
                    "{}",
                    v.name
                );
                continue;
            }
            let nonce: [u8; NONCE_LEN] = URL_SAFE_NO_PAD
                .decode(v.nonce.as_deref().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            assert_eq!(
                seal(&key, nonce, &v.server_id, &n).unwrap(),
                v.encrypted,
                "{}",
                v.name
            );
            assert_eq!(
                open(&key, &v.encrypted, &v.server_id, &n).as_deref(),
                v.plaintext.as_deref(),
                "{}",
                v.name
            );
        }
    }

    #[test]
    fn a_random_nonce_still_opens() {
        let key = [7u8; KEY_LEN];
        let n = Notification {
            event_id: "e".into(),
            kind: "claude_needs_input".into(),
            title: "fix-tests".into(),
            subtitle: "box".into(),
            body: "Claude is asking to use Bash".into(),
            session: Some("fix-tests".into()),
            ask_id: None,
        };
        let enc = encrypt(&key, "h_aaaabbbbccccdddd", &n).unwrap();
        let plain = open(&key, &enc, "h_aaaabbbbccccdddd", &n).unwrap();
        assert!(plain.contains("Claude is asking to use Bash"), "{plain}");
        assert_eq!(open(&key, &enc, "h_eeeeffffgggghhhh", &n), None);
    }

    #[test]
    fn a_long_wide_body_is_shortened_to_fit() {
        let n = Notification {
            event_id: "e".into(),
            kind: "claude_needs_input".into(),
            // Four bytes each, so 100 + 100 + 500 of them is well past the
            // limit before the body is cut.
            title: "😀".repeat(100),
            subtitle: "😀".repeat(100),
            body: "😀".repeat(500),
            session: None,
            ask_id: None,
        };
        let enc = encrypt(&[1u8; KEY_LEN], "h_aaaabbbbccccdddd", &n).unwrap();
        assert!(enc.len() <= MAX_ENCRYPTED, "{}", enc.len());
    }

    #[test]
    fn a_request_id_is_bound_to_the_text() {
        let key = [7u8; KEY_LEN];
        let n = Notification {
            event_id: "e".into(),
            kind: "claude_permission".into(),
            title: "fix-tests".into(),
            subtitle: String::new(),
            body: "Bash: npm test".into(),
            session: Some("fix-tests".into()),
            ask_id: Some("toolu_1".into()),
        };
        let enc = encrypt(&key, "h_aaaabbbbccccdddd", &n).unwrap();
        assert!(open(&key, &enc, "h_aaaabbbbccccdddd", &n).is_some());
        let moved = Notification {
            ask_id: Some("toolu_2".into()),
            ..n.clone()
        };
        assert_eq!(open(&key, &enc, "h_aaaabbbbccccdddd", &moved), None);
        let dropped = Notification { ask_id: None, ..n };
        assert_eq!(open(&key, &enc, "h_aaaabbbbccccdddd", &dropped), None);
    }

    #[test]
    fn keys_of_the_wrong_length_are_refused() {
        assert!(parse_key("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8").is_ok());
        assert!(parse_key("AAEC").is_err());
        assert!(parse_key("not base64 at all!").is_err());
    }

    /// Writes `push-crypto.json` from the cases below. Run by hand after a
    /// deliberate change to the scheme:
    /// `HURA_WRITE_VECTORS=1 cargo test -p hurad write_vectors`.
    #[test]
    fn write_vectors() {
        if std::env::var_os("HURA_WRITE_VECTORS").is_none() {
            return;
        }
        let key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
        let nonce = "oKGio6Slpqeoqaqr";
        let server = "h_aaaabbbbccccdddd";
        let k = parse_key(key).unwrap();
        let nb: [u8; NONCE_LEN] = URL_SAFE_NO_PAD.decode(nonce).unwrap().try_into().unwrap();
        let note = |kind: &str,
                    title: &str,
                    subtitle: &str,
                    body: &str,
                    session: Option<&str>,
                    ask: Option<&str>| Notification {
            event_id: "e".into(),
            kind: kind.into(),
            title: title.into(),
            subtitle: subtitle.into(),
            body: body.into(),
            session: session.map(Into::into),
            ask_id: ask.map(Into::into),
        };
        let ok = |name: &str, n: Notification| {
            let sealed = seal(&k, nb, server, &n).unwrap();
            let plain = open(&k, &sealed, server, &n).unwrap();
            serde_json::json!({"name": name, "key": key, "nonce": nonce, "server_id": server,
                "kind": n.kind, "session": n.session, "ask_id": n.ask_id, "title": n.title,
                "subtitle": n.subtitle, "body": n.body, "plaintext": plain, "encrypted": sealed})
        };
        let permission = note(
            "claude_permission",
            "fix-tests",
            "studio",
            "Bash: npm test -- --runInBand && echo \"<done>\"",
            Some("fix-tests"),
            Some("toolu_01abc"),
        );
        let sealed = seal(&k, nb, server, &permission).unwrap();
        let fails =
            |name: &str, server_id: &str, kind: &str, session: Option<&str>, ask: Option<&str>| {
                serde_json::json!({"name": name, "key": key, "server_id": server_id, "kind": kind,
                "session": session, "ask_id": ask, "encrypted": sealed, "fails": true})
            };
        let all = serde_json::json!([
            ok("permission request", permission.clone()),
            ok(
                "needs input",
                note(
                    "claude_needs_input",
                    "fix-tests",
                    "studio",
                    "Claude needs your input",
                    Some("fix-tests"),
                    None
                )
            ),
            ok(
                "test without session or subtitle",
                note(
                    "test",
                    "hura",
                    "",
                    "Test notification from studio",
                    None,
                    None
                )
            ),
            ok(
                "clipped title",
                note(
                    "claude_turn_finished",
                    &"é".repeat(120),
                    "studio",
                    "Done.",
                    Some("readme"),
                    None
                )
            ),
            fails(
                "another request",
                server,
                "claude_permission",
                Some("fix-tests"),
                Some("toolu_02xyz")
            ),
            fails(
                "no request",
                server,
                "claude_permission",
                Some("fix-tests"),
                None
            ),
            fails(
                "another session",
                server,
                "claude_permission",
                Some("readme"),
                Some("toolu_01abc")
            ),
            fails(
                "another server",
                "h_eeeeffffgggghhhh",
                "claude_permission",
                Some("fix-tests"),
                Some("toolu_01abc")
            ),
            fails(
                "another kind",
                server,
                "claude_needs_input",
                Some("fix-tests"),
                Some("toolu_01abc")
            ),
        ]);
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/push/push-crypto.json");
        std::fs::write(path, serde_json::to_string_pretty(&all).unwrap() + "\n").unwrap();
    }
}
