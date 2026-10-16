//! `~/.mccboard/state/usage.json`: the last reading and the rate-limit state of each account.
//!
//! Layout (version 1), keyed by account UUID:
//!
//! ```json
//! { "version": 1,
//!   "accounts": {
//!     "<account uuid>": {
//!       "reading": { "source": "endpoint", "read_at": "…", "session": {…}, "weekly": {…},
//!                    "models": [ { "model": "…", "utilization": 58.0, "resets_at": "…" } ] },
//!       "last_failure": null,          // or { "kind": "rate_limited" }, …
//!       "interval_secs": 60,           // 60 after a success, doubled after each failure up to 300
//!       "next_call_at": "…"            // no call for this account before then
//!     } },
//!   "renewals": {                      // Claude Code renewing expired tokens (#30)
//!     "<instance config dir>": {
//!       "interval_secs": 900,          // 900 after a success, doubled after each failure up
//!                                      // to 21600
//!       "next_attempt_at": "…",        // no renewal in this instance before then
//!       "last": { "at": "…", "outcome": "renewed" }  // or "failed", "failure": {…}
//!     } } }
//! ```
//!
//! Windows are stored as read; resets are applied when they are shown. A file that can't be
//! read or understood is discarded with a warning: it holds nothing that can't be read again.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::renewal::{self, RenewalAttempt};
use super::{UsageFailure, UsageReading, UsageWarning};

const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct State {
    version: u32,
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountState>,
    /// Keyed by the instance's config directory, as compared ([`crate::paths::key`]).
    #[serde(default)]
    pub renewals: BTreeMap<String, RenewalState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RenewalState {
    #[serde(default = "renewal_base_interval")]
    pub interval_secs: i64,
    #[serde(default)]
    pub next_attempt_at: Option<Timestamp>,
    #[serde(default)]
    pub last: Option<RenewalAttempt>,
}

fn renewal_base_interval() -> i64 {
    renewal::BASE_INTERVAL_SECS
}

impl Default for RenewalState {
    fn default() -> Self {
        Self {
            interval_secs: renewal_base_interval(),
            next_attempt_at: None,
            last: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AccountState {
    #[serde(default)]
    pub reading: Option<UsageReading>,
    /// Why the last call failed, if it did.
    #[serde(default)]
    pub last_failure: Option<UsageFailure>,
    #[serde(default = "base_interval")]
    pub interval_secs: i64,
    #[serde(default)]
    pub next_call_at: Option<Timestamp>,
}

fn base_interval() -> i64 {
    super::BASE_INTERVAL_SECS
}

impl Default for AccountState {
    fn default() -> Self {
        Self {
            reading: None,
            last_failure: None,
            interval_secs: base_interval(),
            next_call_at: None,
        }
    }
}

/// Reads the state. A missing file is an empty state; an unreadable or invalid one is
/// discarded with a warning.
pub(crate) fn load(path: &Path, warnings: &mut Vec<UsageWarning>) -> State {
    let mut discard = || {
        if !warnings.contains(&UsageWarning::StateDiscarded) {
            warnings.push(UsageWarning::StateDiscarded);
        }
        State::default()
    };
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<State>(&bytes) {
            Ok(state) if state.version == VERSION => state,
            _ => discard(),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => State::default(),
        Err(_) => discard(),
    }
}

/// Writes the state atomically (a temporary file in the same folder, then a rename).
pub(crate) fn save(path: &Path, state: &State, warnings: &mut Vec<UsageWarning>) {
    let state = State {
        version: VERSION,
        accounts: state.accounts.clone(),
        renewals: state.renewals.clone(),
    };
    let written = (|| -> io::Result<()> {
        let dir = path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let text = serde_json::to_string_pretty(&state).map_err(io::Error::other)?;
        let temporary = dir.join(format!(".usage.json.{}.tmp", std::process::id()));
        fs::write(&temporary, text)?;
        fs::rename(&temporary, path).inspect_err(|_| {
            let _ = fs::remove_file(&temporary);
        })
    })();
    if written.is_err() && !warnings.contains(&UsageWarning::StateUnwritable) {
        warnings.push(UsageWarning::StateUnwritable);
    }
}
