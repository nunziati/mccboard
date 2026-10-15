use std::path::Path;

use jiff::Timestamp;
use serde::Serialize;

use crate::claude_files::{self, Read};
use crate::{AccountUuid, Environment, ObservedAccount};

/// Whether an instance lives in Claude Code's default config directory (`~/.claude`).
/// Derived from the path, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceKind {
    Primary,
    Linked,
}

impl InstanceKind {
    pub(crate) fn of(home: &Path, config_dir: &Path) -> Self {
        if crate::paths::same(config_dir, &home.join(".claude")) {
            InstanceKind::Primary
        } else {
            InstanceKind::Linked
        }
    }
}

/// An instance's login, told from the credentials' expiry alone. mccboard never renews it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LoginState {
    /// The access token is valid until `expires_at`.
    LoggedIn { expires_at: Timestamp },
    /// The access token expired; Claude Code renews it the next time it runs.
    Expired { expired_at: Timestamp },
    /// No claude.ai credentials in the instance.
    NotLoggedIn,
    /// The credentials exist but can't be read; see the observation's warnings.
    Unknown,
}

/// Something mccboard couldn't read while observing an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Warning {
    /// `.claude.json` exists but can't be read or parsed.
    GlobalConfigUnreadable,
    /// `.credentials.json` exists but can't be read or parsed.
    CredentialsUnreadable,
    /// The config directory itself is gone (a registered instance whose folder was deleted).
    ConfigDirMissing,
}

/// What an instance looks like right now: its observed account and login state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Observation {
    pub account: Option<ObservedAccount>,
    pub login: LoginState,
    pub warnings: Vec<Warning>,
}

pub(crate) fn observe(home: &Path, config_dir: &Path, env: &dyn Environment) -> Observation {
    let mut warnings = Vec::new();
    if !config_dir.is_dir() {
        warnings.push(Warning::ConfigDirMissing);
    }
    let kind = InstanceKind::of(home, config_dir);

    let account = match claude_files::read_global_config(&claude_files::global_config_path(
        home, config_dir, kind,
    )) {
        Read::Missing => None,
        Read::Unreadable => {
            warnings.push(Warning::GlobalConfigUnreadable);
            None
        }
        Read::Parsed(config) => config.oauth_account.and_then(|account| {
            let uuid = account.account_uuid.filter(|uuid| !uuid.is_empty())?;
            Some(ObservedAccount {
                uuid: AccountUuid::new(uuid),
                email: account.email_address.filter(|email| !email.is_empty()),
            })
        }),
    };

    let login = match claude_files::read_credentials_expiry(config_dir) {
        Read::Missing => LoginState::NotLoggedIn,
        Read::Parsed(credentials) => match credentials.claude_ai_oauth {
            None => LoginState::NotLoggedIn,
            Some(oauth) => match oauth
                .expires_at
                .and_then(|ms| Timestamp::from_millisecond(ms).ok())
            {
                Some(expires_at) if expires_at > env.now() => LoginState::LoggedIn { expires_at },
                Some(expired_at) => LoginState::Expired { expired_at },
                None => {
                    warnings.push(Warning::CredentialsUnreadable);
                    LoginState::Unknown
                }
            },
        },
        Read::Unreadable => {
            warnings.push(Warning::CredentialsUnreadable);
            LoginState::Unknown
        }
    };

    Observation {
        account,
        login,
        warnings,
    }
}
