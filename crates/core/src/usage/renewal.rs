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

use super::UsageWarning;
use super::rationing::{Policy, Rationing, Verdict};
use super::state::{RenewalState, State};
use super::token::{self, TokenRead};
use crate::instance::InstanceKind;
use crate::{Environment, Paths, RenewalExit, RenewalInvocation, paths};

/// At most one attempt per instance every 15 minutes; after failures up to 6 hours.
const ATTEMPTS: Policy = Policy {
    base_secs: 15 * 60,
    max_secs: 6 * 60 * 60,
    longest_wait_secs: 6 * 60 * 60,
};

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
    let working_dir = paths.renewal_dir();
    Rationing {
        path: state_path,
        policy: ATTEMPTS,
        table: |state| &mut state.renewals,
    }
    .run(
        state,
        warnings,
        &|| env.now(),
        expired
            .iter()
            .map(|dir| (paths::key(dir), dir.to_path_buf()))
            .collect(),
        |dir| {
            // If it can't be made, starting Claude Code there fails and says so.
            let _ = fs::create_dir_all(&working_dir);
            let exit = env.renew_token(&RenewalInvocation {
                config_dir: dir.clone(),
                kind: InstanceKind::of(home, dir),
                working_dir: working_dir.clone(),
                home: home.to_path_buf(),
            });
            match token::read(dir, env.now()) {
                TokenRead::Valid { .. } => RenewalOutcome::Renewed,
                _ => RenewalOutcome::Failed {
                    failure: failure(exit),
                },
            }
        },
        record,
    )
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

/// Writes an attempt's outcome into the instance's entry.
fn record(entry: &mut RenewalState, outcome: RenewalOutcome, at: Timestamp) -> Verdict {
    let verdict = match outcome {
        RenewalOutcome::Renewed => Verdict::Succeeded,
        RenewalOutcome::Failed { .. } => Verdict::Failed {
            wait_at_least: None,
        },
    };
    entry.last = Some(RenewalAttempt { at, outcome });
    verdict
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
        next_attempt_at: found.schedule.next_at().filter(|_| expired.is_some()),
        config_dir: dir,
    })
}
