//! Tokens: random, prefixed by what they are, and kept only as hashes.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::TryRng as _;
use sha2::{Digest as _, Sha256};

use crate::error::{Error, Result};

/// A refresh token, which a device keeps until it signs out.
pub const REFRESH: &str = "hura_rt_";
/// An access token, which lasts [`ACCESS_SECONDS`].
pub const ACCESS: &str = "hura_at_";

pub const ACCESS_SECONDS: i32 = 60 * 60;

/// 32 bytes from the OS behind `prefix`. The prefix says what a token is to
/// anybody who finds one, a secret scanner included.
pub fn mint(prefix: &str) -> Result<String> {
    let mut bytes = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .map_err(Error::internal)?;
    Ok(format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes)))
}

/// A number below `below`, from the OS, without the bias of a remainder.
pub fn number(below: u32) -> Result<u32> {
    let zone = u32::MAX - u32::MAX % below;
    loop {
        let mut bytes = [0u8; 4];
        rand::rngs::SysRng
            .try_fill_bytes(&mut bytes)
            .map_err(Error::internal)?;
        let n = u32::from_le_bytes(bytes);
        if n < zone {
            return Ok(n % below);
        }
    }
}

pub fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// A random UUID (version 4), from the OS: for an ID that is also part of an
/// address nobody should be able to guess.
pub fn id() -> Result<uuid::Uuid> {
    let mut bytes = [0u8; 16];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .map_err(Error::internal)?;
    Ok(uuid::Builder::from_random_bytes(bytes).into_uuid())
}
