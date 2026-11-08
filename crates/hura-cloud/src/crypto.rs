//! Tracker tokens, encrypted at rest.
//!
//! **Two levels of key.** The master key, from `HURA_CLOUD_MASTER_KEY`, wraps
//! one key per account; an account's key encrypts its tokens. A copy of the
//! database alone opens nothing, and moving the master key into a KMS later
//! changes [`Keys`] and nothing that calls it.
//!
//! AES-256-GCM, a random nonce per value, and the value's place as associated
//! data: a token sealed for one account's tracker does not open as another's,
//! even moved there by somebody who can write to the database.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rand::TryRng as _;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{Error, Result};

/// What every sealed value starts with, so a later scheme can be told apart.
const VERSION: u8 = 1;

pub struct Keys {
    master: LessSafeKey,
}

impl Keys {
    /// The master key, as 32 bytes in base64 (`openssl rand -base64 32`).
    pub fn from_base64(text: &str) -> std::result::Result<Keys, String> {
        let bytes = STANDARD
            .decode(text.trim())
            .map_err(|e| format!("HURA_CLOUD_MASTER_KEY is not base64: {e}"))?;
        Ok(Keys {
            master: key(&bytes).ok_or("HURA_CLOUD_MASTER_KEY is not 32 bytes")?,
        })
    }

    /// The service's own secrets, not any account's: its apps on GitHub and
    /// Atlassian. Sealed with the master key itself, under a label.
    pub fn seal_service(&self, label: &str, plain: &[u8]) -> Result<Vec<u8>> {
        seal(&self.master, &format!("service\0{label}"), plain)
    }

    pub fn open_service(&self, label: &str, sealed: &[u8]) -> Result<Vec<u8>> {
        open(&self.master, &format!("service\0{label}"), sealed)
    }

    /// An account's key, made the first time it is asked for.
    pub async fn account(&self, db: &PgPool, account: Uuid) -> Result<AccountKey> {
        let aad = format!("account-key\0{account}");
        let wrapped: Option<Vec<u8>> =
            sqlx::query_scalar("select wrapped from account_keys where account_id = $1")
                .bind(account)
                .fetch_optional(db)
                .await?;
        let wrapped = match wrapped {
            Some(w) => w,
            None => {
                let mut fresh = [0u8; 32];
                random(&mut fresh)?;
                let sealed = seal(&self.master, &aad, &fresh)?;
                // Two requests making one at once: the first wins and both
                // use it.
                sqlx::query(
                    "insert into account_keys (account_id, wrapped) values ($1, $2)
                     on conflict (account_id) do nothing",
                )
                .bind(account)
                .bind(&sealed)
                .execute(db)
                .await?;
                sqlx::query_scalar("select wrapped from account_keys where account_id = $1")
                    .bind(account)
                    .fetch_one(db)
                    .await?
            }
        };
        let bytes = open(&self.master, &aad, &wrapped)?;
        Ok(AccountKey {
            account,
            key: key(&bytes).ok_or_else(|| Error::internal("an account key is not 32 bytes"))?,
        })
    }
}

/// One account's key, which seals and opens its trackers' tokens.
pub struct AccountKey {
    account: Uuid,
    key: LessSafeKey,
}

impl AccountKey {
    pub fn seal_token(&self, tracker: &str, token: &str) -> Result<Vec<u8>> {
        seal(&self.key, &self.token_aad(tracker), token.as_bytes())
    }

    pub fn open_token(&self, tracker: &str, sealed: &[u8]) -> Result<String> {
        let plain = open(&self.key, &self.token_aad(tracker), sealed)?;
        String::from_utf8(plain).map_err(Error::internal)
    }

    /// Anything else of the account's that is secret, under a label that
    /// says what it is and so where it may be opened.
    pub fn seal(&self, label: &str, plain: &[u8]) -> Result<Vec<u8>> {
        seal(&self.key, &format!("{label}\0{}", self.account), plain)
    }

    pub fn open(&self, label: &str, sealed: &[u8]) -> Result<Vec<u8>> {
        open(&self.key, &format!("{label}\0{}", self.account), sealed)
    }

    fn token_aad(&self, tracker: &str) -> String {
        format!("tracker-token\0{}\0{tracker}", self.account)
    }
}

fn key(bytes: &[u8]) -> Option<LessSafeKey> {
    UnboundKey::new(&AES_256_GCM, bytes)
        .ok()
        .map(LessSafeKey::new)
}

fn random(out: &mut [u8]) -> Result<()> {
    rand::rngs::SysRng
        .try_fill_bytes(out)
        .map_err(Error::internal)
}

/// `VERSION`, the nonce, then the ciphertext and its tag.
fn seal(key: &LessSafeKey, aad: &str, plain: &[u8]) -> Result<Vec<u8>> {
    let mut nonce = [0u8; NONCE_LEN];
    random(&mut nonce)?;
    let mut data = plain.to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(aad.as_bytes()),
        &mut data,
    )
    .map_err(|_| Error::internal("could not encrypt"))?;
    let mut out = Vec::with_capacity(1 + NONCE_LEN + data.len());
    out.push(VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&data);
    Ok(out)
}

fn open(key: &LessSafeKey, aad: &str, sealed: &[u8]) -> Result<Vec<u8>> {
    let refused = || Error::internal("a sealed value would not open");
    if sealed.len() < 1 + NONCE_LEN || sealed[0] != VERSION {
        return Err(refused());
    }
    let nonce =
        Nonce::try_assume_unique_for_key(&sealed[1..1 + NONCE_LEN]).map_err(|_| refused())?;
    let mut data = sealed[1 + NONCE_LEN..].to_vec();
    let plain = key
        .open_in_place(nonce, Aad::from(aad.as_bytes()), &mut data)
        .map_err(|_| refused())?;
    Ok(plain.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account_key(account: Uuid) -> AccountKey {
        AccountKey {
            account,
            key: key(&[7u8; 32]).unwrap(),
        }
    }

    #[test]
    fn a_token_opens_only_where_it_was_sealed() {
        let a = account_key(Uuid::from_u128(1));
        let sealed = a.seal_token("work", "secret").unwrap();
        assert_eq!(a.open_token("work", &sealed).unwrap(), "secret");
        assert!(!sealed.windows(6).any(|w| w == b"secret"));
        assert!(a.open_token("home", &sealed).is_err(), "another tracker");
        let b = account_key(Uuid::from_u128(2));
        assert!(b.open_token("work", &sealed).is_err(), "another account");
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(a.open_token("work", &tampered).is_err());
    }

    #[test]
    fn the_same_token_seals_differently_each_time() {
        let a = account_key(Uuid::from_u128(1));
        assert_ne!(
            a.seal_token("work", "t").unwrap(),
            a.seal_token("work", "t").unwrap()
        );
    }

    #[test]
    fn a_master_key_is_32_bytes_of_base64() {
        assert!(Keys::from_base64(&STANDARD.encode([1u8; 32])).is_ok());
        assert!(Keys::from_base64(&STANDARD.encode([1u8; 16])).is_err());
        assert!(Keys::from_base64("not base64!").is_err());
    }
}
