//! The OAuth access token, read from an instance's credentials and handed to the endpoint
//! call only.
//!
//! Token rules (AGENTS.md, ADR 0002): the token is read-only, never refreshed, and never
//! printed. [`AccessToken`] has no `Debug`, `Display`, `Clone` or `Serialize`, no public
//! constructor and no public accessor: outside the core it can only be passed along.

use std::fs;
use std::io;
use std::path::Path;

use jiff::Timestamp;
use serde::Deserialize;

use crate::claude_files;

/// An instance's OAuth access token. Opaque: it can't be printed, copied, serialized or read
/// outside the core. The core only ever gives one to [`crate::Environment::call_usage_endpoint`].
///
/// ```compile_fail
/// fn leak(token: &mccboard_core::AccessToken) -> String { format!("{token:?}") }
/// ```
/// ```compile_fail
/// fn leak(token: &mccboard_core::AccessToken) -> String { format!("{token}") }
/// ```
/// ```compile_fail
/// fn leak(token: &mccboard_core::AccessToken) -> String { serde_json::to_string(token).unwrap() }
/// ```
/// ```compile_fail
/// fn leak(token: &mccboard_core::AccessToken) -> &str { token.expose() }
/// ```
pub struct AccessToken(String);

impl AccessToken {
    /// The token's value, for the `Authorization` header and nothing else.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    /// For the fake port: tells tokens apart without handing the value out.
    #[cfg(feature = "testing")]
    pub(crate) fn fingerprint(&self) -> u64 {
        fingerprint(&self.0)
    }
}

/// What an instance's credentials offer for the endpoint call.
pub(crate) enum TokenRead {
    /// A token valid until `expires_at`.
    Valid {
        token: AccessToken,
        expires_at: Timestamp,
    },
    /// A token past its `expiresAt`: "token expired", and no call is made with it.
    Expired,
    /// No credentials file, no claude.ai login in it, or an emptied token (logged out).
    Missing,
    /// The file exists but can't be read, or its token or expiry can't be understood.
    Unreadable,
}

#[derive(Deserialize)]
struct Credentials {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<OAuth>,
}

#[derive(Deserialize)]
struct OAuth {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    /// Epoch milliseconds.
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
}

/// Reads the access token of the instance in `config_dir`. Only reads: the file is never
/// written, and parse errors are dropped unseen, since they can quote the file's contents.
pub(crate) fn read(config_dir: &Path, now: Timestamp) -> TokenRead {
    let bytes = match fs::read(claude_files::credentials_path(config_dir)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return TokenRead::Missing,
        Err(_) => return TokenRead::Unreadable,
    };
    let credentials: Credentials = match serde_json::from_slice(&bytes) {
        Ok(credentials) => credentials,
        Err(_) => return TokenRead::Unreadable,
    };
    let Some(oauth) = credentials.claude_ai_oauth else {
        return TokenRead::Missing;
    };
    let token = match oauth.access_token {
        Some(token) if !token.is_empty() => AccessToken(token),
        _ => return TokenRead::Missing,
    };
    match oauth
        .expires_at
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
    {
        Some(expires_at) if expires_at > now => TokenRead::Valid { token, expires_at },
        Some(_) => TokenRead::Expired,
        None => TokenRead::Unreadable,
    }
}

/// A non-reversible fingerprint of a token value, for the fake port's call records.
#[cfg(feature = "testing")]
pub(crate) fn fingerprint(value: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
