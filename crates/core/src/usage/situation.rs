//! An account's usage situation at a given time (#53): its reading as of then, how fresh it
//! is, what keeps it from being refreshed, and where its token renewal stands. The core
//! decides it once; the CLI, the tray, the taskbar widget and the UI only put it into words.
//!
//! A report is drawn again long after it was made (the taskbar widget counts down every
//! second), so the situation is re-derived at the time of each drawing: windows reset, a
//! reading goes stale, a retry time passes.

use std::path::{Path, PathBuf};

use jiff::{SignedDuration, Timestamp};
use serde::Serialize;

use super::renewal::{RenewalFailure, RenewalOutcome, TokenRenewal};
use super::{UsageFailure, UsageReading, as_of};

/// A reading older than this is stale even if nothing failed: several refreshes were missed.
const STALE_AFTER: SignedDuration = SignedDuration::from_mins(10);

/// Where an account's usage stands at one time.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageSituation {
    /// The last known reading, with every window past its reset read as zero. Kept when a
    /// later refresh fails or can't be made.
    pub reading: Option<UsageReading>,
    pub freshness: Freshness,
    /// What keeps the usage from being refreshed; `None` when nothing does.
    pub blocker: Option<UsageBlocker>,
    pub renewal: RenewalSituation,
}

/// How far the reading can be trusted as current.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Current,
    /// Shown, but not current: something blocks the refresh, or the reading is more than 10
    /// minutes old.
    Stale,
    /// No reading yet.
    NoReading,
}

/// Why an account's usage isn't being refreshed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsageBlocker {
    /// Every token of the account is past its expiry (glossary: expired token). No call is
    /// made with it.
    TokenExpired,
    /// No registered instance is logged in with this account.
    NotLoggedIn,
    /// An instance is logged in with this account, but its credentials can't be read.
    CredentialsUnreadable,
    /// The last call to the usage endpoint failed.
    RefreshFailed { failure: UsageFailure },
}

/// Where the token renewal of an account stands. Every case but `Quiet` names the instance
/// Claude Code is (or would be) run in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RenewalSituation {
    /// The token is valid, and the last renewal of an instance of the account succeeded.
    Renewed { config_dir: PathBuf, at: Timestamp },
    /// The token is expired and the last attempt failed. `retry_at` is `None` when the next
    /// refresh may try again.
    Failed {
        config_dir: PathBuf,
        at: Timestamp,
        failure: RenewalFailure,
        retry_at: Option<Timestamp>,
    },
    /// The token is expired and no attempt has failed since it last was renewed. `next_at` is
    /// `None` when the next refresh may try.
    Waiting {
        config_dir: PathBuf,
        next_at: Option<Timestamp>,
    },
    /// The token is expired and token renewal is turned off: Claude Code renews it the next
    /// time it runs with this account.
    Off { config_dir: PathBuf },
    /// Nothing to say.
    Quiet,
}

impl RenewalSituation {
    /// The instance the renewal is about, if any.
    pub fn config_dir(&self) -> Option<&Path> {
        match self {
            Self::Renewed { config_dir, .. }
            | Self::Failed { config_dir, .. }
            | Self::Waiting { config_dir, .. }
            | Self::Off { config_dir } => Some(config_dir),
            Self::Quiet => None,
        }
    }

    /// The situation of a renewal that mccboard keeps track of: the account's token is
    /// `expired` or not, and `renewal` is the instance's last attempt and its wait.
    pub(super) fn of(renewal: Option<TokenRenewal>, expired: Option<&Path>, on: bool) -> Self {
        match (expired, on) {
            (Some(dir), false) => {
                return Self::Off {
                    config_dir: dir.to_path_buf(),
                };
            }
            (None, false) => return Self::Quiet,
            _ => {}
        }
        let Some(renewal) = renewal else {
            return Self::Quiet;
        };
        let config_dir = renewal.config_dir;
        match (renewal.last, expired.is_some()) {
            (Some(last), false) => match last.outcome {
                RenewalOutcome::Renewed => Self::Renewed {
                    config_dir,
                    at: last.at,
                },
                RenewalOutcome::Failed { .. } => Self::Quiet,
            },
            (None, false) => Self::Quiet,
            (Some(last), true) => match last.outcome {
                RenewalOutcome::Failed { failure } => Self::Failed {
                    config_dir,
                    at: last.at,
                    failure,
                    retry_at: renewal.next_attempt_at,
                },
                RenewalOutcome::Renewed => Self::Waiting {
                    config_dir,
                    next_at: renewal.next_attempt_at,
                },
            },
            (None, true) => Self::Waiting {
                config_dir,
                next_at: renewal.next_attempt_at,
            },
        }
    }

    fn at(&self, now: Timestamp) -> Self {
        let pending = |at: &Option<Timestamp>| at.filter(|at| *at > now);
        match self {
            Self::Failed {
                config_dir,
                at,
                failure,
                retry_at,
            } => Self::Failed {
                config_dir: config_dir.clone(),
                at: *at,
                failure: failure.clone(),
                retry_at: pending(retry_at),
            },
            Self::Waiting {
                config_dir,
                next_at,
            } => Self::Waiting {
                config_dir: config_dir.clone(),
                next_at: pending(next_at),
            },
            other => other.clone(),
        }
    }
}

impl UsageSituation {
    /// The situation of a reading, a blocker and a renewal, as of `now`.
    pub(super) fn new(
        reading: Option<UsageReading>,
        blocker: Option<UsageBlocker>,
        renewal: RenewalSituation,
        now: Timestamp,
    ) -> Self {
        Self {
            reading,
            freshness: Freshness::NoReading,
            blocker,
            renewal,
        }
        .at(now)
    }

    /// The same situation as of `now`: windows past their reset read as zero, a reading older
    /// than 10 minutes is stale, a retry time that has passed is the next refresh.
    pub(super) fn at(&self, now: Timestamp) -> Self {
        let reading = self.reading.clone().map(|reading| as_of(reading, now));
        let freshness = match &reading {
            None => Freshness::NoReading,
            Some(reading)
                if self.blocker.is_some() || now.duration_since(reading.read_at) > STALE_AFTER =>
            {
                Freshness::Stale
            }
            Some(_) => Freshness::Current,
        };
        Self {
            reading,
            freshness,
            blocker: self.blocker.clone(),
            renewal: self.renewal.at(now),
        }
    }

    /// Whether the account's token is expired.
    pub fn token_expired(&self) -> bool {
        self.blocker == Some(UsageBlocker::TokenExpired)
    }
}
