//! Claude Code's files inside a config directory, read with the narrowest possible view.
//!
//! The credentials file is parsed into a struct that holds only `expiresAt` and whether there
//! is an access token: token values are never stored, and parse errors are discarded rather
//! than shown, because a serde message can quote the value it choked on.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};

use crate::InstanceKind;

/// File name of Claude Code's global config.
const GLOBAL_CONFIG: &str = ".claude.json";
/// File name of Claude Code's credentials, inside the config directory.
const CREDENTIALS: &str = ".credentials.json";

/// Where an instance's global config lives: next to the config directory for the primary
/// instance (`~/.claude.json`), inside it for any other (`$CLAUDE_CONFIG_DIR/.claude.json`).
pub(crate) fn global_config_path(home: &Path, config_dir: &Path, kind: InstanceKind) -> PathBuf {
    match kind {
        InstanceKind::Primary => home.join(GLOBAL_CONFIG),
        InstanceKind::Linked => config_dir.join(GLOBAL_CONFIG),
    }
}

pub(crate) fn credentials_path(config_dir: &Path) -> PathBuf {
    config_dir.join(CREDENTIALS)
}

/// The outcome of reading one of Claude Code's files.
pub(crate) enum Read<T> {
    Missing,
    Unreadable,
    Parsed(T),
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Read<T> {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(value) => Read::Parsed(value),
            Err(_) => Read::Unreadable,
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Read::Missing,
        Err(_) => Read::Unreadable,
    }
}

/// The parts of `.claude.json` mccboard reads to observe an account.
#[derive(Deserialize)]
pub(crate) struct GlobalConfig {
    #[serde(rename = "oauthAccount")]
    pub oauth_account: Option<OAuthAccount>,
}

#[derive(Deserialize)]
pub(crate) struct OAuthAccount {
    #[serde(rename = "accountUuid")]
    pub account_uuid: Option<String>,
    #[serde(rename = "emailAddress")]
    pub email_address: Option<String>,
}

pub(crate) fn read_global_config(path: &Path) -> Read<GlobalConfig> {
    read_json(path)
}

/// The only part of `.credentials.json` mccboard reads to tell the login state.
#[derive(Deserialize)]
pub(crate) struct CredentialsExpiry {
    #[serde(rename = "claudeAiOauth")]
    pub claude_ai_oauth: Option<OAuthExpiry>,
}

#[derive(Deserialize)]
pub(crate) struct OAuthExpiry {
    /// Whether `accessToken` holds a token. Claude Code empties it on logout (a dead refresh
    /// token); the value itself is dropped as soon as it is parsed.
    #[serde(rename = "accessToken", default, deserialize_with = "non_empty")]
    pub has_access_token: bool,
    /// Epoch milliseconds.
    #[serde(rename = "expiresAt")]
    pub expires_at: Option<i64>,
}

fn non_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.is_some_and(|value| !value.is_empty()))
}

pub(crate) fn read_credentials_expiry(config_dir: &Path) -> Read<CredentialsExpiry> {
    read_json(&credentials_path(config_dir))
}
