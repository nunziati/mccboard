//! Rationing (#55): how often mccboard may try something whose budget it shares with Claude
//! Code. That is a call to the usage endpoint for an account, or a token renewal in an
//! instance. Each kind of attempt has a [`Policy`]; each entry in `usage.json` (an account, an
//! instance) has a [`Schedule`].
//!
//! - an entry is due once its `next_at` has passed;
//! - each due entry is reserved in the state file before it is tried, so a concurrent mccboard
//!   (the app and the CLI) sees it as just tried (ADR 0002);
//! - after a success the wait is the policy's base. After a failure the wait doubles, up to
//!   the policy's cap. It is longer when the failure asks for it (the endpoint's
//!   `Retry-After`), up to the policy's longest wait;
//! - the outcomes are applied to the state as re-read after the attempts: another mccboard may
//!   have written its own entries meanwhile.

use std::collections::BTreeMap;
use std::path::Path;

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use super::UsageWarning;
use super::state::{self, State};

/// How often one kind of attempt may be made.
#[derive(Debug, Clone, Copy)]
pub(super) struct Policy {
    /// The wait after a success, and the first wait.
    pub base_secs: i64,
    /// The longest wait failures double up to.
    pub max_secs: i64,
    /// The longest wait a failure may ask for.
    pub longest_wait_secs: i64,
}

/// When an entry may next be tried, and how long its wait has grown.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Schedule {
    /// The wait after the last attempt; anything below the policy's base reads as the base.
    #[serde(default)]
    interval_secs: i64,
    /// No attempt before then.
    #[serde(default)]
    next_at: Option<Timestamp>,
}

/// How an attempt went, as far as rationing cares.
pub(super) enum Verdict {
    Succeeded,
    /// The next attempt waits at least `wait_at_least`, when given, besides the doubled wait.
    Failed {
        wait_at_least: Option<SignedDuration>,
    },
}

/// An entry of the state file that has a schedule.
pub(super) trait Rationed: Default {
    fn schedule(&mut self) -> &mut Schedule;
}

impl Schedule {
    /// No attempt before then; `None` when one may be made now.
    pub fn next_at(&self) -> Option<Timestamp> {
        self.next_at
    }

    fn due(&self, now: Timestamp) -> bool {
        self.next_at.is_none_or(|at| at <= now)
    }

    fn interval(&self, policy: Policy) -> i64 {
        self.interval_secs.clamp(policy.base_secs, policy.max_secs)
    }

    fn reserve(&mut self, now: Timestamp, policy: Policy) {
        self.next_at = Some(now + secs(self.interval(policy)));
    }

    fn settle(&mut self, verdict: Verdict, at: Timestamp, policy: Policy) {
        let wait = match verdict {
            Verdict::Succeeded => {
                self.interval_secs = policy.base_secs;
                policy.base_secs
            }
            Verdict::Failed { wait_at_least } => {
                self.interval_secs = (self.interval(policy) * 2).min(policy.max_secs);
                let least = wait_at_least
                    .map_or(0, |wait| wait.as_secs())
                    .clamp(0, policy.longest_wait_secs);
                self.interval_secs.max(least)
            }
        };
        self.next_at = Some(at + secs(wait));
    }
}

/// Rationed attempts on one table of the state file.
pub(super) struct Rationing<'a, E> {
    /// `usage.json`.
    pub path: &'a Path,
    pub policy: Policy,
    /// The table whose entries are tried, keyed as the candidates are.
    pub table: fn(&mut State) -> &mut BTreeMap<String, E>,
}

impl<E: Rationed> Rationing<'_, E> {
    /// Tries each of `candidates` (a key in the table, and what `attempt` needs) whose wait is
    /// over, and returns those it tried. The due entries are reserved and saved first. Then
    /// each is tried, and the time on `clock` is noted after each attempt. Finally the state
    /// is read again, and `record` writes each result into its entry and says how it went.
    /// `state` ends as saved.
    pub fn run<C, R>(
        &self,
        state: &mut State,
        warnings: &mut Vec<UsageWarning>,
        clock: &dyn Fn() -> Timestamp,
        candidates: Vec<(String, C)>,
        mut attempt: impl FnMut(&C) -> R,
        mut record: impl FnMut(&mut E, R, Timestamp) -> Verdict,
    ) -> Vec<C> {
        let now = clock();
        let mut due = Vec::new();
        for (key, candidate) in candidates {
            let schedule = (self.table)(state)
                .entry(key.clone())
                .or_default()
                .schedule();
            if schedule.due(now) {
                schedule.reserve(now, self.policy);
                due.push((key, candidate));
            }
        }
        if due.is_empty() {
            return Vec::new();
        }
        state::save(self.path, state, warnings);

        let results: Vec<(String, R, Timestamp)> = due
            .iter()
            .map(|(key, candidate)| {
                let result = attempt(candidate);
                (key.clone(), result, clock())
            })
            .collect();

        // Re-read: another mccboard may have written meanwhile.
        *state = state::load(self.path, warnings);
        for (key, result, at) in results {
            let entry = (self.table)(state).entry(key).or_default();
            let verdict = record(entry, result, at);
            entry.schedule().settle(verdict, at, self.policy);
        }
        state::save(self.path, state, warnings);
        due.into_iter().map(|(_, candidate)| candidate).collect()
    }
}

fn secs(n: i64) -> SignedDuration {
    SignedDuration::from_secs(n)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::super::state::RenewalState;
    use super::*;

    const POLICY: Policy = Policy {
        base_secs: 60,
        max_secs: 300,
        longest_wait_secs: 3600,
    };

    fn t0() -> Timestamp {
        "2026-10-15T12:00:00Z".parse().unwrap()
    }

    /// A state file in a temporary folder, rationed on its renewals table, with a clock.
    struct Fixture {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
        now: Cell<Timestamp>,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("usage.json");
            Self {
                _dir: dir,
                path,
                now: Cell::new(t0()),
            }
        }

        fn rationing(&self) -> Rationing<'_, RenewalState> {
            Rationing {
                path: &self.path,
                policy: POLICY,
                table: |state| &mut state.renewals,
            }
        }

        fn load(&self) -> State {
            state::load(&self.path, &mut Vec::new())
        }

        fn set(&self, at: Timestamp) {
            self.now.set(at);
        }

        /// Runs once for `keys` from the saved state; each attempt ends with `verdict`.
        fn run_with(&self, keys: &[&str], verdict: impl Fn() -> Verdict) -> Vec<String> {
            let mut state = self.load();
            let clock = || self.now.get();
            self.rationing().run(
                &mut state,
                &mut Vec::new(),
                &clock,
                keys.iter()
                    .map(|k| (k.to_string(), k.to_string()))
                    .collect(),
                |_| (),
                |_, (), _| verdict(),
            )
        }

        fn fail(&self, keys: &[&str]) -> Vec<String> {
            self.run_with(keys, || Verdict::Failed {
                wait_at_least: None,
            })
        }

        fn succeed(&self, keys: &[&str]) -> Vec<String> {
            self.run_with(keys, || Verdict::Succeeded)
        }

        fn next_at(&self, key: &str) -> Option<Timestamp> {
            self.load().renewals.get(key)?.schedule.next_at()
        }
    }

    #[test]
    fn an_entry_never_tried_is_due_and_is_saved_as_reserved_before_it_runs() {
        let f = Fixture::new();
        let mut state = f.load();
        let clock = || f.now.get();
        let mut seen = None;

        let tried = f.rationing().run(
            &mut state,
            &mut Vec::new(),
            &clock,
            vec![("a".to_owned(), ())],
            |_| seen = f.next_at("a"),
            |_, (), _| Verdict::Succeeded,
        );

        assert_eq!(tried.len(), 1);
        assert_eq!(seen, Some(t0() + secs(60)));
    }

    #[test]
    fn an_entry_whose_wait_is_not_over_is_not_tried() {
        let f = Fixture::new();
        f.succeed(&["a"]);

        f.set(t0() + secs(59));
        assert!(f.succeed(&["a"]).is_empty());
        f.set(t0() + secs(60));
        assert_eq!(f.succeed(&["a"]), ["a"]);
    }

    #[test]
    fn failures_double_the_wait_up_to_the_cap() {
        let f = Fixture::new();
        let mut at = t0();
        for wait in [120, 240, 300, 300] {
            f.set(at);
            assert_eq!(f.fail(&["a"]), ["a"]);
            assert_eq!(f.next_at("a"), Some(at + secs(wait)));
            at += secs(wait);
        }
    }

    #[test]
    fn a_success_brings_the_wait_back_to_the_base() {
        let f = Fixture::new();
        f.fail(&["a"]);
        f.set(t0() + secs(120));
        f.fail(&["a"]);
        f.set(t0() + secs(360));

        f.succeed(&["a"]);
        assert_eq!(f.next_at("a"), Some(t0() + secs(420)));

        // The next failure starts doubling from the base again.
        f.set(t0() + secs(420));
        f.fail(&["a"]);
        assert_eq!(f.next_at("a"), Some(t0() + secs(540)));
    }

    #[test]
    fn a_failure_may_ask_for_a_longer_wait_up_to_the_longest() {
        let f = Fixture::new();
        f.run_with(&["a", "b", "c"], || Verdict::Failed {
            wait_at_least: Some(secs(1000)),
        });
        assert_eq!(f.next_at("a"), Some(t0() + secs(1000)));

        f.set(t0() + secs(1000));
        f.run_with(&["a"], || Verdict::Failed {
            wait_at_least: Some(secs(10_000)),
        });
        assert_eq!(f.next_at("a"), Some(t0() + secs(1000 + 3600)));

        // A shorter wait than the doubled one changes nothing.
        f.set(t0() + secs(1000));
        f.run_with(&["b"], || Verdict::Failed {
            wait_at_least: Some(secs(5)),
        });
        assert_eq!(f.next_at("b"), Some(t0() + secs(1000 + 240)));
    }

    #[test]
    fn each_entry_has_its_own_schedule() {
        let f = Fixture::new();
        f.fail(&["a"]);
        f.set(t0() + secs(60));

        assert_eq!(f.fail(&["a", "b"]), ["b"]);
    }

    #[test]
    fn what_another_mccboard_wrote_during_the_attempts_is_kept() {
        let f = Fixture::new();
        let mut state = f.load();
        let clock = || f.now.get();

        f.rationing().run(
            &mut state,
            &mut Vec::new(),
            &clock,
            vec![("a".to_owned(), ())],
            |_| {
                // The CLI tries "b" while the app tries "a".
                let mut other = f.load();
                f.rationing().run(
                    &mut other,
                    &mut Vec::new(),
                    &clock,
                    vec![("b".to_owned(), ())],
                    |_| (),
                    |_, (), _| Verdict::Succeeded,
                );
            },
            |_, (), _| Verdict::Succeeded,
        );

        assert!(state.renewals.contains_key("b"));
        assert_eq!(f.next_at("b"), Some(t0() + secs(60)));
        assert_eq!(f.next_at("a"), Some(t0() + secs(60)));
    }

    #[test]
    fn the_record_writes_the_callers_data_into_the_entry() {
        let f = Fixture::new();
        let mut state = f.load();
        let clock = || f.now.get();

        f.rationing().run(
            &mut state,
            &mut Vec::new(),
            &clock,
            vec![("a".to_owned(), ())],
            |_| super::super::RenewalOutcome::Renewed,
            |entry, outcome, at| {
                entry.last = Some(super::super::RenewalAttempt { at, outcome });
                Verdict::Succeeded
            },
        );

        let last = f.load().renewals["a"].last.clone().unwrap();
        assert_eq!(last.at, t0());
        assert_eq!(last.outcome, super::super::RenewalOutcome::Renewed);
    }
}
