//! Usage readings: each registered account's usage windows, read from the usage endpoint once
//! per account through a token from one of its local instances (ADR 0003).
//!
//! Token rules (AGENTS.md, ADR 0002) enforced here:
//! - the token is only read, never refreshed, and only handed to the port's endpoint call;
//! - a token past its `expiresAt` reads as "token expired" and no call is made with it; during
//!   a refresh, with the preference on, Claude Code may be asked to renew it first
//!   ([`renewal`]);
//! - at most one call per 60 s per account; after a 429 or any error the interval doubles, up
//!   to 300 s (or longer when the endpoint's `Retry-After` says so), and a success resets it;
//! - the last known reading is kept, marked stale, and a window past its reset reads as zero.

mod endpoint;
mod renewal;
mod state;
mod token;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

pub(crate) use endpoint::call as call_endpoint;
pub use renewal::{RenewalAttempt, RenewalFailure, RenewalOutcome, TokenRenewal};
pub use token::AccessToken;
#[cfg(feature = "testing")]
pub(crate) use token::fingerprint;

use crate::config::{ConfigFile, InstanceEntry};
use crate::{Account, AccountUuid, EndpointReply, Environment, Error, Paths, instance, paths};
use token::TokenRead;

pub(crate) const BASE_INTERVAL_SECS: i64 = 60;
const MAX_INTERVAL_SECS: i64 = 300;
/// The longest wait a `Retry-After` can impose.
const MAX_RETRY_AFTER_SECS: i64 = 3600;
/// A reading older than this is stale even if nothing failed: several refreshes were missed.
const STALE_AFTER: SignedDuration = SignedDuration::from_mins(10);

/// Every registered account's usage, in label order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageReport {
    pub accounts: Vec<AccountUsage>,
    /// Problems with mccboard's own state file; the readings are still good.
    pub warnings: Vec<UsageWarning>,
}

/// One account's usage as known now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AccountUsage {
    pub account: Account,
    pub status: UsageStatus,
    /// The last known reading, with windows past their reset read as zero. Kept when a later
    /// refresh fails or can't be made.
    pub reading: Option<UsageReading>,
    /// The reading is not current: the latest refresh failed or couldn't be made, or the
    /// reading is more than 10 minutes old.
    pub stale: bool,
    /// Whether this operation called the endpoint for this account.
    pub called: bool,
    /// When the endpoint may next be called for this account; `None` when it may be called
    /// now, or when there is no token to call it with.
    pub next_call_at: Option<Timestamp>,
    /// Claude Code renewing the account's expired token: while it is expired, or after it was
    /// renewed. `None` when the preference is off, or when no renewal was ever needed.
    pub renewal: Option<TokenRenewal>,
}

/// Where an account's usage stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UsageStatus {
    /// The last call succeeded.
    Ok,
    /// A token is available but no call has been made yet.
    NeverRead,
    /// Every token of the account is past its expiry. Claude Code renews it the next time it
    /// runs, or when mccboard asks it to ([`AccountUsage::renewal`]); mccboard itself never
    /// does, and makes no call.
    TokenExpired,
    /// No registered instance is logged in with this account.
    NotLoggedIn,
    /// An instance is logged in with this account, but its credentials can't be read.
    CredentialsUnreadable,
    /// The last call failed.
    Failed { failure: UsageFailure },
}

/// Why a call to the usage endpoint failed. Never carries anything from the request, nor the
/// reply's body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsageFailure {
    /// 429: the endpoint's budget, shared with Claude Code, is used up for now.
    RateLimited,
    /// 401: the token was refused. Claude Code renews it the next time it runs.
    Unauthorized,
    /// 403: not allowed (account or organization without OAuth usage, or a missing scope).
    Forbidden,
    /// Any other non-success status.
    HttpStatus { status: u16 },
    /// No reply: network, proxy, TLS or timeout.
    Unreachable { reason: String },
    /// A reply that isn't a usage reading.
    Malformed,
}

/// All of an account's usage windows at one moment (glossary: usage reading).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageReading {
    pub source: UsageSource,
    pub read_at: Timestamp,
    /// The 5-hour window.
    pub session: Option<UsageWindow>,
    /// The 7-day window covering all models.
    pub weekly: Option<UsageWindow>,
    /// 7-day windows that apply to one model each, as the endpoint names them.
    pub models: Vec<ModelWindow>,
}

/// Where a reading came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Endpoint,
}

/// A usage window: utilization 0–100 and when it resets. Once the reset time passes the
/// window reads as utilization 0 with no reset time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    pub utilization: f64,
    pub resets_at: Option<Timestamp>,
}

/// A model window, named by the endpoint (no model name is built in).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelWindow {
    pub model: String,
    pub utilization: f64,
    pub resets_at: Option<Timestamp>,
}

/// A problem with `~/.mccboard/state/usage.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageWarning {
    /// The file couldn't be read or understood and was discarded: readings start over.
    StateDiscarded,
    /// The file couldn't be written: readings and rate limits won't survive a restart.
    StateUnwritable,
}

/// What an account's instances offer for a call.
enum Source {
    Token(AccessToken),
    /// Every token expired; `usage_instance` is the instance whose token expired last, where
    /// Claude Code is asked to renew it.
    TokenExpired {
        usage_instance: PathBuf,
    },
    CredentialsUnreadable,
    NotLoggedIn,
}

/// An account's source, and the registered instances currently logged in with it.
struct Found {
    source: Source,
    dirs: Vec<PathBuf>,
}

impl Found {
    fn expired_in(&self) -> Option<&Path> {
        match &self.source {
            Source::TokenExpired { usage_instance } => Some(usage_instance),
            _ => None,
        }
    }
}

/// Reports every registered account's usage. With `refresh`, first has Claude Code renew the
/// expired tokens it may (when the preference is on), then calls the endpoint for each account
/// that has a valid token and whose rate limit allows a call.
pub(crate) fn report(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
    lock: &Mutex<()>,
    refresh: bool,
) -> Result<UsageReport, Error> {
    let config = ConfigFile::load(&paths.config_file())?.config;
    let renewal_on = config.renew_expired_tokens;
    let mut accounts = config.accounts;
    accounts
        .sort_by(|a, b| (a.label.to_lowercase(), &a.uuid).cmp(&(b.label.to_lowercase(), &b.uuid)));

    // One refresh at a time in this process; the reservation below covers other processes.
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let state_path = paths.state_dir().join("usage.json");
    let mut warnings = Vec::new();
    let mut state = state::load(&state_path, &mut warnings);
    let mut now = env.now();
    let find = |now: Timestamp| -> Vec<Found> {
        accounts
            .iter()
            .map(|account| source(home, &config.instances, &account.uuid, env, now))
            .collect()
    };
    let mut found = find(now);

    let mut renewed_in = Vec::new();
    if refresh && renewal_on {
        let expired: Vec<&Path> = found.iter().filter_map(Found::expired_in).collect();
        renewed_in = renewal::run(
            home,
            paths,
            env,
            &state_path,
            &mut state,
            &expired,
            &mut warnings,
        );
        if !renewed_in.is_empty() {
            // Read the tokens again: Claude Code may have renewed them.
            now = env.now();
            found = find(now);
        }
    }

    // Reserve the calls before making them, so a concurrent mccboard (the app and the CLI)
    // sees the account as just called.
    let mut due = Vec::new();
    if refresh {
        for (index, (account, found)) in accounts.iter().zip(&found).enumerate() {
            let Source::Token(_) = found.source else {
                continue;
            };
            let entry = state.accounts.entry(account.uuid.to_string()).or_default();
            if entry.next_call_at.is_none_or(|at| at <= now) {
                entry.next_call_at = Some(now + secs(entry.interval_secs));
                due.push(index);
            }
        }
        if !due.is_empty() {
            state::save(&state_path, &state, &mut warnings);
        }
    }

    let mut outcomes = Vec::new();
    for &index in &due {
        let Source::Token(token) = &found[index].source else {
            continue;
        };
        let reply = env.call_usage_endpoint(token);
        outcomes.push((index, interpret(reply, env.now())));
    }

    let now = env.now();
    if !outcomes.is_empty() {
        // Re-read: another mccboard may have written its own accounts meanwhile.
        state = state::load(&state_path, &mut warnings);
        for (index, outcome) in &outcomes {
            let entry = state
                .accounts
                .entry(accounts[*index].uuid.to_string())
                .or_default();
            apply(entry, outcome, now);
        }
        state::save(&state_path, &state, &mut warnings);
    }

    let accounts = accounts
        .into_iter()
        .zip(found)
        .enumerate()
        .map(|(index, (account, found))| {
            let entry = state
                .accounts
                .get(account.uuid.as_str())
                .cloned()
                .unwrap_or_default();
            let called = due.contains(&index);
            let renewal = renewal_on
                .then(|| renewal::view(&state, found.expired_in(), &found.dirs, &renewed_in, now))
                .flatten();
            AccountUsage {
                renewal,
                ..view(account, &found.source, entry, called, now)
            }
        })
        .collect();
    Ok(UsageReport { accounts, warnings })
}

/// The best token among the registered instances currently logged in with `account`: the
/// valid one that lasts longest. Otherwise why there is none.
fn source(
    home: &Path,
    instances: &[InstanceEntry],
    account: &AccountUuid,
    env: &dyn Environment,
    now: Timestamp,
) -> Found {
    let mut best: Option<(Timestamp, AccessToken)> = None;
    let mut expired: Option<(Timestamp, &Path)> = None;
    let mut unreadable = false;
    let mut dirs = Vec::new();
    let mut seen: Vec<&Path> = Vec::new();
    for entry in instances {
        let dir = entry.config_dir.as_path();
        if seen.iter().any(|other| paths::same(other, dir)) {
            continue;
        }
        seen.push(dir);
        let observed = instance::observe(home, dir, env).account;
        if observed.is_none_or(|observed| &observed.uuid != account) {
            continue;
        }
        dirs.push(dir.to_path_buf());
        match token::read(dir, now) {
            TokenRead::Valid { token, expires_at } => {
                if best.as_ref().is_none_or(|(at, _)| expires_at > *at) {
                    best = Some((expires_at, token));
                }
            }
            TokenRead::Expired { expires_at } => {
                if expired.is_none_or(|(at, _)| expires_at > at) {
                    expired = Some((expires_at, dir));
                }
            }
            TokenRead::Unreadable => unreadable = true,
            TokenRead::Missing => {}
        }
    }
    let source = match (best, expired) {
        (Some((_, token)), _) => Source::Token(token),
        (None, Some((_, dir))) => Source::TokenExpired {
            usage_instance: dir.to_path_buf(),
        },
        (None, None) if unreadable => Source::CredentialsUnreadable,
        (None, None) => Source::NotLoggedIn,
    };
    Found { source, dirs }
}

/// What a reply means: a reading, or a failure with the endpoint's `Retry-After`.
enum Outcome {
    Read(UsageReading),
    Failed(UsageFailure, Option<SignedDuration>),
}

fn interpret(reply: EndpointReply, now: Timestamp) -> Outcome {
    match reply {
        EndpointReply::Http {
            status: 200, body, ..
        } => match endpoint::parse(&body) {
            Some(windows) => Outcome::Read(UsageReading {
                source: UsageSource::Endpoint,
                read_at: now,
                session: windows.session,
                weekly: windows.weekly,
                models: windows.models,
            }),
            None => Outcome::Failed(UsageFailure::Malformed, None),
        },
        EndpointReply::Http {
            status,
            retry_after,
            ..
        } => {
            let failure = match status {
                429 => UsageFailure::RateLimited,
                401 => UsageFailure::Unauthorized,
                403 => UsageFailure::Forbidden,
                status => UsageFailure::HttpStatus { status },
            };
            Outcome::Failed(failure, retry_after)
        }
        EndpointReply::Unreachable { reason } => {
            Outcome::Failed(UsageFailure::Unreachable { reason }, None)
        }
    }
}

fn apply(entry: &mut state::AccountState, outcome: &Outcome, now: Timestamp) {
    match outcome {
        Outcome::Read(reading) => {
            entry.reading = Some(reading.clone());
            entry.last_failure = None;
            entry.interval_secs = BASE_INTERVAL_SECS;
            entry.next_call_at = Some(now + secs(BASE_INTERVAL_SECS));
        }
        Outcome::Failed(failure, retry_after) => {
            entry.last_failure = Some(failure.clone());
            entry.interval_secs =
                (entry.interval_secs.max(BASE_INTERVAL_SECS) * 2).min(MAX_INTERVAL_SECS);
            let retry_after = retry_after
                .map_or(0, |d| d.as_secs())
                .clamp(0, MAX_RETRY_AFTER_SECS);
            entry.next_call_at = Some(now + secs(entry.interval_secs.max(retry_after)));
        }
    }
}

fn view(
    account: Account,
    source: &Source,
    entry: state::AccountState,
    called: bool,
    now: Timestamp,
) -> AccountUsage {
    let (status, next_call_at) = match source {
        Source::Token(_) => {
            let status = match (&entry.last_failure, &entry.reading) {
                (Some(failure), _) => UsageStatus::Failed {
                    failure: failure.clone(),
                },
                (None, Some(_)) => UsageStatus::Ok,
                (None, None) => UsageStatus::NeverRead,
            };
            (status, entry.next_call_at.filter(|at| *at > now))
        }
        Source::TokenExpired { .. } => (UsageStatus::TokenExpired, None),
        Source::CredentialsUnreadable => (UsageStatus::CredentialsUnreadable, None),
        Source::NotLoggedIn => (UsageStatus::NotLoggedIn, None),
    };
    let stale = entry
        .reading
        .as_ref()
        .is_some_and(|r| status != UsageStatus::Ok || now.duration_since(r.read_at) > STALE_AFTER);
    AccountUsage {
        account,
        status,
        reading: entry.reading.map(|reading| as_of(reading, now)),
        stale,
        called,
        next_call_at,
        renewal: None,
    }
}

/// The reading with every window whose reset time has passed read as zero.
pub(crate) fn as_of(mut reading: UsageReading, now: Timestamp) -> UsageReading {
    let reset = |utilization: &mut f64, resets_at: &mut Option<Timestamp>| {
        if resets_at.is_some_and(|at| at <= now) {
            *utilization = 0.0;
            *resets_at = None;
        }
    };
    for window in [&mut reading.session, &mut reading.weekly]
        .into_iter()
        .flatten()
    {
        reset(&mut window.utilization, &mut window.resets_at);
    }
    for window in &mut reading.models {
        reset(&mut window.utilization, &mut window.resets_at);
    }
    reading
}

fn secs(n: i64) -> SignedDuration {
    SignedDuration::from_secs(n)
}
