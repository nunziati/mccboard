//! Letting Claude Code renew an expired token (#30): when a usage reading is due and every
//! token of an account has expired, mccboard runs `claude mcp get` in the account's usage
//! instance, hidden, so that Claude Code renews its own token; then reads the token again and
//! calls the endpoint. mccboard itself never refreshes the token and never writes credentials.
//!
//! Every test runs on a temporary home with the fake environment port, whose renewal handler
//! plays Claude Code (rewriting `.credentials.json`) and whose clock is moved by hand.

mod common;

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{FIXTURE_REFRESH, FIXTURE_TOKEN, Instance, TempHome, UUID_A, UUID_B, now};
use jiff::{SignedDuration, Timestamp};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    AccountUsage, Core, EndpointReply, Environment, InstanceKind, RenewalExit, RenewalFailure,
    RenewalSituation, UsageBlocker, UsageReport,
};
use serde_json::json;

const RENEWED_TOKEN: &str = "sk-ant-oat01-FIXTURE-RENEWED-must-never-leak-dddddddd";

fn mins(n: i64) -> SignedDuration {
    SignedDuration::from_mins(n)
}

fn hours(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n)
}

fn ok() -> EndpointReply {
    EndpointReply::Http {
        status: 200,
        retry_after: None,
        body: json!({"limits": [
            {"kind": "session", "percent": 35, "resets_at": "2026-10-15T15:00:00Z"},
            {"kind": "weekly_all", "percent": 14, "resets_at": "2026-10-20T08:00:00Z"},
        ]})
        .to_string(),
    }
}

/// A home with `~/.claude-b` logged in as A, its token expired an hour ago, registered.
fn home_with_expired_b() -> (TempHome, Instance, Core, FakeEnvironment) {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() - hours(1));
    let (core, env) = home.core();
    core.register(&b.dir).unwrap();
    (home, b, core, env)
}

/// Makes the fake's renewal play Claude Code renewing its token: it rewrites the instance's
/// credentials with [`RENEWED_TOKEN`], valid for `lifetime` from the fake clock, and exits 0.
fn renews(env: &FakeEnvironment, lifetime: SignedDuration) {
    let clock = env.clone();
    env.on_renewal(move |renewal| {
        let expires_at = clock.now() + lifetime;
        std::fs::write(
            renewal.config_dir.join(".credentials.json"),
            json!({"claudeAiOauth": {
                "accessToken": RENEWED_TOKEN,
                "refreshToken": FIXTURE_REFRESH,
                "expiresAt": expires_at.as_millisecond(),
                "scopes": ["user:inference", "user:profile"],
            }})
            .to_string(),
        )?;
        Ok(RenewalExit { exit_code: Some(0) })
    });
}

/// Makes the fake's renewal end with `exit_code` and change nothing.
fn exits(env: &FakeEnvironment, exit_code: i32) {
    env.on_renewal(move |_| {
        Ok(RenewalExit {
            exit_code: Some(exit_code),
        })
    });
}

fn only(report: &UsageReport) -> &AccountUsage {
    assert_eq!(report.accounts.len(), 1, "{report:?}");
    &report.accounts[0]
}

fn renewal_off(home: &TempHome) {
    let config = home.config_text().unwrap();
    home.write_config(&format!(
        "{config}\n[preferences]\nrenew_expired_tokens = false\n"
    ));
}

// --- Running the renewal -----------------------------------------------------------------

#[test]
fn an_expired_token_is_renewed_by_claude_code_then_usage_is_read_with_the_new_token() {
    let (home, b, core, env) = home_with_expired_b();
    renews(&env, hours(8));
    env.push_usage_reply(ok());

    let report = core.refresh_usage().unwrap();

    let renewals = env.renewal_invocations();
    assert_eq!(renewals.len(), 1);
    assert_eq!(renewals[0].config_dir, b.dir);
    assert_eq!(renewals[0].kind, InstanceKind::Linked);
    // A neutral folder of mccboard's own, so no project's hooks or CLAUDE.md apply.
    assert_eq!(
        renewals[0].working_dir,
        home.path().join(".mccboard").join("state").join("renewal")
    );
    assert!(renewals[0].working_dir.is_dir());

    let usage = only(&report);
    assert_eq!(usage.as_reported().blocker, None);
    assert!(usage.called);
    assert_eq!(env.usage_calls_with_token(RENEWED_TOKEN), 1);
    assert_eq!(
        usage.as_reported().renewal,
        RenewalSituation::Renewed {
            config_dir: b.dir.clone(),
            at: now()
        }
    );
}

#[test]
fn the_primary_instance_is_renewed_as_the_primary_instance() {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() - hours(1));
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();
    renews(&env, hours(8));
    env.push_usage_reply(ok());

    core.refresh_usage().unwrap();

    let renewals = env.renewal_invocations();
    assert_eq!(renewals.len(), 1);
    assert_eq!(renewals[0].config_dir, primary.dir);
    assert_eq!(renewals[0].kind, InstanceKind::Primary);
}

#[test]
fn a_valid_token_is_never_renewed() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() + mins(1));
    let (core, env) = home.core();
    core.register(&b.dir).unwrap();
    env.push_usage_reply(ok());

    core.refresh_usage().unwrap();

    assert!(env.renewal_invocations().is_empty());
}

#[test]
fn reading_usage_without_refreshing_never_renews() {
    let (_home, _b, core, env) = home_with_expired_b();

    let report = core.usage().unwrap();

    assert!(env.renewal_invocations().is_empty());
    assert_eq!(
        only(&report).as_reported().blocker,
        Some(UsageBlocker::TokenExpired)
    );
}

#[test]
fn an_account_with_a_valid_token_elsewhere_is_not_renewed() {
    let (home, _b, core, env) = home_with_expired_b();
    let c = home.linked(".claude-c");
    c.logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() + hours(2));
    core.register(&c.dir).unwrap();
    env.push_usage_reply(ok());

    core.refresh_usage().unwrap();

    assert!(env.renewal_invocations().is_empty());
}

#[test]
fn only_the_usage_instance_is_renewed_the_one_whose_token_expired_last() {
    let (home, _b, core, env) = home_with_expired_b();
    let c = home.linked(".claude-c");
    c.logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() - mins(10));
    core.register(&c.dir).unwrap();
    renews(&env, hours(8));
    env.push_usage_reply(ok());

    core.refresh_usage().unwrap();

    let renewals = env.renewal_invocations();
    assert_eq!(renewals.len(), 1);
    assert_eq!(renewals[0].config_dir, c.dir);
}

#[test]
fn each_account_is_renewed_in_its_own_instance() {
    let (home, b, core, env) = home_with_expired_b();
    let c = home.linked(".claude-c");
    c.logged_in_as(UUID_B, "b@example.com")
        .credentials_expiring(now() - hours(3));
    core.register(&c.dir).unwrap();
    renews(&env, hours(8));
    env.push_usage_reply(ok());
    env.push_usage_reply(ok());

    let report = core.refresh_usage().unwrap();

    let dirs: Vec<_> = env
        .renewal_invocations()
        .into_iter()
        .map(|r| r.config_dir)
        .collect();
    assert_eq!(dirs.len(), 2);
    assert!(dirs.contains(&b.dir) && dirs.contains(&c.dir));
    assert!(
        report
            .accounts
            .iter()
            .all(|a| a.as_reported().blocker.is_none())
    );
}

// --- The preference ----------------------------------------------------------------------

#[test]
fn renewal_is_on_by_default_and_can_be_turned_off_in_config_toml() {
    let (home, _b, core, _env) = home_with_expired_b();
    assert!(core.renew_expired_tokens().unwrap());

    core.set_renew_expired_tokens(false).unwrap();

    let config: toml::Table = home.config_text().unwrap().parse().unwrap();
    assert_eq!(
        config["preferences"]["renew_expired_tokens"].as_bool(),
        Some(false)
    );
    assert!(!core.renew_expired_tokens().unwrap());
    core.set_renew_expired_tokens(true).unwrap();
    assert!(core.renew_expired_tokens().unwrap());
}

#[test]
fn turning_renewal_off_keeps_everything_else_in_the_file() {
    let home = TempHome::new();
    home.write_config("# mine\n[preferences]\nlanguage = \"it\" # chosen\n");
    let (core, _env) = home.core();

    core.set_renew_expired_tokens(false).unwrap();

    assert_eq!(
        home.config_text().unwrap(),
        "# mine\n[preferences]\nlanguage = \"it\" # chosen\nrenew_expired_tokens = false\n"
    );
}

#[test]
fn a_hand_written_preference_that_is_not_a_boolean_is_reported() {
    let home = TempHome::new();
    home.write_config("[preferences]\nrenew_expired_tokens = \"yes\"\n");
    let (core, _env) = home.core();

    let error = core.renew_expired_tokens().unwrap_err();

    assert_eq!(error.code(), "config_corrupt");
}

#[test]
fn with_renewal_off_an_expired_token_reads_as_today() {
    let (home, b, core, env) = home_with_expired_b();
    // A reading from when the token was valid.
    b.credentials_with_token(FIXTURE_TOKEN, now() + hours(1));
    env.push_usage_reply(ok());
    core.refresh_usage().unwrap();
    b.credentials_expiring(now() - hours(1));
    renewal_off(&home);
    renews(&env, hours(8));
    env.advance(mins(20));

    let report = core.refresh_usage().unwrap();

    assert!(env.renewal_invocations().is_empty());
    let situation = only(&report).as_reported();
    assert_eq!(situation.blocker, Some(UsageBlocker::TokenExpired));
    assert!(situation.reading.is_some());
    assert_eq!(situation.freshness, mccboard_core::Freshness::Stale);
    assert_eq!(
        situation.renewal,
        RenewalSituation::Off {
            config_dir: b.dir.clone()
        }
    );
    assert_eq!(env.usage_calls(), 1);
}

// --- Rate limits -------------------------------------------------------------------------

fn attempts_at(env: &FakeEnvironment, core: &Core, after: SignedDuration) -> usize {
    env.advance(after);
    core.refresh_usage().unwrap();
    env.renewal_invocations().len()
}

/// The renewal's policy; how rationing doubles, resets and survives a restart is tested with
/// it (`rationing`).
#[test]
fn a_failed_renewal_waits_30_minutes_then_doubles_up_to_6_hours() {
    let (_home, _b, core, env) = home_with_expired_b();
    exits(&env, 0); // Claude Code ran but the token is still expired.

    assert_eq!(attempts_at(&env, &core, mins(0)), 1);
    assert_eq!(attempts_at(&env, &core, mins(1)), 1);
    assert_eq!(attempts_at(&env, &core, mins(28)), 1);
    assert_eq!(attempts_at(&env, &core, mins(1)), 2); // 30 min after the first
    assert_eq!(attempts_at(&env, &core, mins(59)), 2);
    assert_eq!(attempts_at(&env, &core, mins(1)), 3); // then 1 h
    assert_eq!(attempts_at(&env, &core, mins(120)), 4); // 2 h
    assert_eq!(attempts_at(&env, &core, mins(240)), 5); // 4 h
    assert_eq!(attempts_at(&env, &core, mins(359)), 5);
    assert_eq!(attempts_at(&env, &core, mins(1)), 6); // capped at 6 h
    assert_eq!(attempts_at(&env, &core, mins(359)), 6);
    assert_eq!(attempts_at(&env, &core, mins(1)), 7);
}

#[test]
fn a_success_resets_the_interval_to_15_minutes() {
    let (_home, _b, core, env) = home_with_expired_b();
    exits(&env, 1);
    assert_eq!(attempts_at(&env, &core, mins(0)), 1);
    assert_eq!(attempts_at(&env, &core, mins(30)), 2); // next wait: 1 h
    renews(&env, mins(5));
    env.push_usage_reply(ok());
    assert_eq!(attempts_at(&env, &core, mins(60)), 3);

    // The renewed token expires 5 minutes later: one attempt per 15 minutes at most.
    env.push_usage_reply(ok());
    assert_eq!(attempts_at(&env, &core, mins(6)), 3);
    assert_eq!(attempts_at(&env, &core, mins(8)), 3);
    exits(&env, 1);
    assert_eq!(attempts_at(&env, &core, mins(1)), 4);
}

#[test]
fn another_mccboard_refreshing_meanwhile_does_not_renew_again() {
    let (home, _b, core, env) = home_with_expired_b();
    let other = Core::new(home.path(), env.clone());
    let nested = Arc::new(AtomicUsize::new(0));
    let seen = nested.clone();
    env.on_renewal(move |_| {
        // The CLI refreshes while the app's renewal runs.
        if seen.fetch_add(1, Ordering::SeqCst) == 0 {
            other.refresh_usage().unwrap();
        }
        Ok(RenewalExit { exit_code: Some(1) })
    });

    core.refresh_usage().unwrap();

    assert_eq!(nested.load(Ordering::SeqCst), 1);
    assert_eq!(env.renewal_invocations().len(), 1);
}

// --- Outcomes ----------------------------------------------------------------------------

fn failure_after(setup: impl FnOnce(&FakeEnvironment)) -> RenewalFailure {
    let (_home, _b, core, env) = home_with_expired_b();
    setup(&env);
    let report = core.refresh_usage().unwrap();
    let usage = only(&report);
    assert_eq!(
        usage.as_reported().blocker,
        Some(UsageBlocker::TokenExpired)
    );
    assert!(!usage.called);
    assert_eq!(env.renewal_invocations().len(), 1);
    match &usage.as_reported().renewal {
        RenewalSituation::Failed {
            failure, retry_at, ..
        } => {
            assert_eq!(*retry_at, Some(now() + mins(30)));
            failure.clone()
        }
        other => panic!("not a failure: {other:?}"),
    }
}

#[test]
fn why_a_renewal_failed_is_reported() {
    assert_eq!(
        failure_after(|env| exits(env, 0)),
        RenewalFailure::StillExpired
    );
    assert_eq!(
        failure_after(|env| exits(env, 2)),
        RenewalFailure::Exited { code: Some(2) }
    );
    assert_eq!(
        failure_after(|env| env.on_renewal(|_| Ok(RenewalExit { exit_code: None }))),
        RenewalFailure::Exited { code: None }
    );
    assert_eq!(
        failure_after(|env| env.on_renewal(|_| Err(io::ErrorKind::NotFound.into()))),
        RenewalFailure::ClaudeNotFound
    );
    assert_eq!(
        failure_after(|env| env.on_renewal(|_| Err(io::ErrorKind::TimedOut.into()))),
        RenewalFailure::TimedOut
    );
    assert_eq!(
        failure_after(|env| env.on_renewal(|_| Err(io::ErrorKind::PermissionDenied.into()))),
        RenewalFailure::NotStarted
    );
}

#[test]
fn a_token_valid_afterwards_counts_as_renewed_whatever_the_exit_code() {
    let (_home, _b, core, env) = home_with_expired_b();
    let clock = env.clone();
    env.on_renewal(move |renewal| {
        write_valid(&renewal.config_dir, clock.now() + hours(8));
        Ok(RenewalExit { exit_code: Some(1) })
    });
    env.push_usage_reply(ok());

    let report = core.refresh_usage().unwrap();

    let usage = only(&report);
    assert_eq!(usage.as_reported().blocker, None);
    assert!(matches!(
        usage.as_reported().renewal,
        RenewalSituation::Renewed { .. }
    ));
}

/// Claude Code's renewed credentials, as it would write them.
fn write_valid(dir: &Path, expires_at: Timestamp) {
    std::fs::write(
        dir.join(".credentials.json"),
        json!({"claudeAiOauth": {
            "accessToken": RENEWED_TOKEN,
            "refreshToken": FIXTURE_REFRESH,
            "expiresAt": expires_at.as_millisecond(),
        }})
        .to_string(),
    )
    .unwrap();
}

#[test]
fn the_last_renewal_stays_known_while_the_token_is_valid() {
    let (_home, b, core, env) = home_with_expired_b();
    renews(&env, hours(8));
    env.push_usage_reply(ok());
    core.refresh_usage().unwrap();
    env.advance(hours(1));

    let report = core.usage().unwrap();

    assert_eq!(env.renewal_invocations().len(), 1);
    assert_eq!(
        only(&report).as_reported().renewal,
        RenewalSituation::Renewed {
            config_dir: b.dir.clone(),
            at: now()
        }
    );
}

#[test]
fn an_expired_token_not_yet_tried_has_a_renewal_due_now() {
    let (_home, b, core, env) = home_with_expired_b();

    let report = core.usage().unwrap();

    assert_eq!(
        only(&report).as_reported().renewal,
        RenewalSituation::Waiting {
            config_dir: b.dir.clone(),
            next_at: None
        }
    );
    assert!(env.renewal_invocations().is_empty());
}

// --- Token rules -------------------------------------------------------------------------

#[test]
fn mccboard_never_writes_the_credentials_and_no_token_reaches_its_output() {
    let (home, b, core, env) = home_with_expired_b();
    let before = b.credentials_bytes();
    exits(&env, 1);

    let report = core.refresh_usage().unwrap();
    env.advance(hours(1));
    let later = core.refresh_usage().unwrap();

    assert_eq!(b.credentials_bytes(), before);
    let mut outputs = vec![
        serde_json::to_string(&report).unwrap(),
        format!("{report:?}"),
        serde_json::to_string(&later).unwrap(),
        format!("{:?}", env.renewal_invocations()),
    ];
    outputs.extend(home.mccboard_files().into_iter().map(|(_, text)| text));
    for output in outputs {
        for secret in [FIXTURE_TOKEN, FIXTURE_REFRESH] {
            assert!(!output.contains(secret), "{output}");
        }
    }
}
