//! Letting Claude Code renew an expired token (#30, #51, ADR 0002).
//!
//! When a usage refresh finds every token of an account expired, mccboard runs Claude Code's
//! `mcp get` with a missing name in the account's usage instance: the one whose token expired
//! last. Claude Code
//! renews its own token and rewrites its own credentials; mccboard then reads the token again.
//! mccboard itself never refreshes a token and never writes credentials.
//!
//! - only during a refresh, only while the preference is on, never while a token is valid;
//! - at most one attempt per instance every 15 minutes; after a failure (Claude Code missing,
//!   a non-zero exit, a timeout, or a token still expired) the wait doubles, up to 6 hours,
//!   and a success resets it;
//! - each attempt is reserved in the state file before it runs, so a concurrent mccboard (the
//!   app and the CLI) doesn't run it again.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::state::{self, RenewalState, State};
use super::token::{self, TokenRead};
use super::{UsageWarning, secs};
use crate::instance::InstanceKind;
use crate::{Environment, Paths, RenewalExit, RenewalInvocation, paths};

pub(crate) const BASE_INTERVAL_SECS: i64 = 15 * 60;
const MAX_INTERVAL_SECS: i64 = 6 * 60 * 60;

/// What mccboard keeps about the renewal of an account's token, as
/// [`super::RenewalSituation`] reads it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TokenRenewal {
    /// The instance Claude Code is (or was last) run in: while the token is expired, the
    /// account's usage instance; otherwise the one renewed last.
    pub(super) config_dir: PathBuf,
    /// The last attempt, if there was one.
    pub(super) last: Option<RenewalAttempt>,
    /// While the token is expired: no attempt before then. `None` when the next refresh may
    /// try, or when the token is valid.
    pub(super) next_attempt_at: Option<Timestamp>,
}

/// One attempt to have Claude Code renew a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenewalAttempt {
    /// When it ended.
    pub at: Timestamp,
    #[serde(flatten)]
    pub outcome: RenewalOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RenewalOutcome {
    /// The token was valid afterwards.
    Renewed,
    Failed {
        failure: RenewalFailure,
    },
}

/// Why a renewal didn't leave a valid token. Never carries anything Claude Code printed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenewalFailure {
    /// `claude` isn't on the PATH.
    ClaudeNotFound,
    /// Claude Code didn't finish within a minute and was stopped.
    TimedOut,
    /// Claude Code exited with a non-zero code (`None`: ended by a signal), token still expired.
    Exited { code: Option<i32> },
    /// Claude Code exited normally, but the token is still expired.
    StillExpired,
    /// Claude Code couldn't be started.
    NotStarted,
}

/// Has Claude Code renew the expired token of each instance in `expired` (one usage instance
/// per account) whose wait has passed. Returns the instances it ran in.
pub(super) fn run(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
    state_path: &Path,
    state: &mut State,
    expired: &[&Path],
    warnings: &mut Vec<UsageWarning>,
) -> Vec<PathBuf> {
    let now = env.now();
    let mut due = Vec::new();
    for dir in expired {
        let entry = state.renewals.entry(paths::key(dir)).or_default();
        if entry.next_attempt_at.is_none_or(|at| at <= now) {
            entry.next_attempt_at = Some(now + secs(entry.interval_secs));
            due.push(dir.to_path_buf());
        }
    }
    if due.is_empty() {
        return due;
    }
    state::save(state_path, state, warnings);

    let working_dir = paths.renewal_dir();
    // If it can't be made, starting Claude Code there fails and says so.
    let _ = fs::create_dir_all(&working_dir);
    let mut attempts = Vec::new();
    for dir in &due {
        let exit = env.renew_token(&RenewalInvocation {
            config_dir: dir.clone(),
            kind: InstanceKind::of(home, dir),
            working_dir: working_dir.clone(),
        });
        let at = env.now();
        let renewed = matches!(token::read(dir, at), TokenRead::Valid { .. });
        let outcome = if renewed {
            RenewalOutcome::Renewed
        } else {
            RenewalOutcome::Failed {
                failure: failure(exit),
            }
        };
        attempts.push((paths::key(dir), RenewalAttempt { at, outcome }));
    }

    // Re-read: another mccboard may have written meanwhile.
    *state = state::load(state_path, warnings);
    for (key, attempt) in attempts {
        apply(state.renewals.entry(key).or_default(), attempt);
    }
    state::save(state_path, state, warnings);
    due
}

fn failure(exit: io::Result<RenewalExit>) -> RenewalFailure {
    match exit {
        Ok(RenewalExit { exit_code: Some(0) }) => RenewalFailure::StillExpired,
        Ok(RenewalExit { exit_code }) => RenewalFailure::Exited { code: exit_code },
        Err(e) => match e.kind() {
            io::ErrorKind::NotFound => RenewalFailure::ClaudeNotFound,
            io::ErrorKind::TimedOut => RenewalFailure::TimedOut,
            _ => RenewalFailure::NotStarted,
        },
    }
}

fn apply(entry: &mut RenewalState, attempt: RenewalAttempt) {
    entry.interval_secs = match attempt.outcome {
        RenewalOutcome::Renewed => BASE_INTERVAL_SECS,
        RenewalOutcome::Failed { .. } => {
            (entry.interval_secs.max(BASE_INTERVAL_SECS) * 2).min(MAX_INTERVAL_SECS)
        }
    };
    entry.next_attempt_at = Some(attempt.at + secs(entry.interval_secs));
    entry.last = Some(attempt);
}

/// The renewal to show for an account: while its token is expired, that of its usage instance
/// `expired`; otherwise that of the instance among `dirs` renewed last, if any.
pub(super) fn view(
    state: &State,
    expired: Option<&Path>,
    dirs: &[PathBuf],
) -> Option<TokenRenewal> {
    let entry = |dir: &Path| state.renewals.get(&paths::key(dir));
    let dir = match expired {
        Some(dir) => dir.to_path_buf(),
        None => dirs
            .iter()
            .filter_map(|dir| Some((entry(dir)?.last.as_ref()?.at, dir)))
            .max_by_key(|(at, _)| *at)?
            .1
            .clone(),
    };
    let found = entry(&dir).cloned().unwrap_or_default();
    Some(TokenRenewal {
        last: found.last,
        next_attempt_at: found.next_attempt_at.filter(|_| expired.is_some()),
        config_dir: dir,
    })
}
