//! `~/.mccboard/state/usage.json`: the last reading and the rate-limit state of each account.
//!
//! Layout (version 2), keyed by account UUID:
//!
//! ```json
//! { "version": 2,
//!   "accounts": {
//!     "<account uuid>": {
//!       "reading": { "source": "endpoint", "read_at": "…", "session": {…}, "weekly": {…},
//!                    "models": [ { "model": "…", "utilization": 58.0, "resets_at": "…" } ] },
//!       "last_failure": null,          // or { "kind": "rate_limited" }, …
//!       "interval_secs": 60,           // the schedule of its endpoint calls (rationing)
//!       "next_at": "…"
//!     } },
//!   "renewals": {                      // Claude Code renewing expired tokens (#30)
//!     "<instance config dir>": {
//!       "interval_secs": 900,          // the schedule of its renewals (rationing)
//!       "next_at": "…",
//!       "last": { "at": "…", "outcome": "renewed" }  // or "failed", "failure": {…}
//!     } } }
//! ```
//!
//! Windows are stored as read; resets are applied when they are shown. A file of another
//! version is started over silently; one that can't be read or understood is discarded with a
//! warning. It holds nothing that can't be read again.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::rationing::{Rationed, Schedule};
use super::renewal::RenewalAttempt;
use super::{UsageFailure, UsageReading, UsageWarning};

const VERSION: u32 = 2;

/// Just the version of a state file, read before the rest.
#[derive(Deserialize)]
struct Versioned {
    version: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct State {
    version: u32,
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountState>,
    /// Keyed by the instance's config directory, as compared ([`crate::paths::key`]).
    #[serde(default)]
    pub renewals: BTreeMap<String, RenewalState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RenewalState {
    #[serde(flatten)]
    pub schedule: Schedule,
    #[serde(default)]
    pub last: Option<RenewalAttempt>,
}

impl Rationed for RenewalState {
    fn schedule(&mut self) -> &mut Schedule {
        &mut self.schedule
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct AccountState {
    #[serde(default)]
    pub reading: Option<UsageReading>,
    /// Why the last call failed, if it did.
    #[serde(default)]
    pub last_failure: Option<UsageFailure>,
    #[serde(flatten)]
    pub schedule: Schedule,
}

impl Rationed for AccountState {
    fn schedule(&mut self) -> &mut Schedule {
        &mut self.schedule
    }
}

/// Reads the state. A missing file, or one of another version, is an empty state; an
/// unreadable or invalid one is discarded with a warning.
pub(crate) fn load(path: &Path, warnings: &mut Vec<UsageWarning>) -> State {
    let mut discard = || {
        if !warnings.contains(&UsageWarning::StateDiscarded) {
            warnings.push(UsageWarning::StateDiscarded);
        }
        State::default()
    };
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Versioned>(&bytes) {
            Ok(Versioned { version }) if version != VERSION => State::default(),
            Ok(_) => serde_json::from_slice(&bytes).unwrap_or_else(|_| discard()),
            Err(_) => discard(),
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
