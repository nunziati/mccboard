//! An account's usage situation (#53): the core decides, at the time a view is drawn, how fresh
//! the reading is, what blocks the refresh and where the token renewal stands, so the CLI,
//! the tray, the taskbar widget and the UI only put it into words.
//!
//! Every test runs on a temporary home with the fake environment port and a clock moved by
//! hand.

mod common;

use common::{FIXTURE_REFRESH, Instance, TempHome, UUID_A, now};
use jiff::{SignedDuration, Timestamp};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    AccountUsage, Core, EndpointReply, Environment, Freshness, RenewalExit, RenewalFailure,
    RenewalSituation, UsageBlocker, UsageFailure, UsageReport, UsageWindow,
};
use serde_json::json;

const SESSION_RESET: &str = "2026-10-15T15:00:00Z";

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
            {"kind": "session", "percent": 35, "resets_at": SESSION_RESET},
            {"kind": "weekly_all", "percent": 14, "resets_at": "2026-10-20T08:00:00Z"},
        ]})
        .to_string(),
    }
}

fn only(report: &UsageReport) -> &AccountUsage {
    assert_eq!(report.accounts.len(), 1, "{report:?}");
    &report.accounts[0]
}

/// `~/.claude-b` logged in as A with a token valid for `valid_for` (negative: expired),
/// registered.
fn home_with_b(valid_for: SignedDuration) -> (TempHome, Instance, Core, FakeEnvironment) {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(common::FIXTURE_TOKEN, now() + valid_for);
    let (core, env) = home.core();
    core.register(&b.dir).unwrap();
    (home, b, core, env)
}

fn renewal_off(home: &TempHome) {
    let config = home.config_text().unwrap();
    home.write_config(&format!(
        "{config}\n[preferences]\nrenew_expired_tokens = false\n"
    ));
}

/// Makes the fake's renewal play Claude Code renewing its token for `lifetime`.
fn renews(env: &FakeEnvironment, lifetime: SignedDuration) {
    let clock = env.clone();
    env.on_renewal(move |renewal| {
        std::fs::write(
            renewal.config_dir.join(".credentials.json"),
            json!({"claudeAiOauth": {
                "accessToken": "sk-ant-oat01-FIXTURE-RENEWED-must-never-leak-eeeeeeee",
                "refreshToken": FIXTURE_REFRESH,
                "expiresAt": (clock.now() + lifetime).as_millisecond(),
            }})
            .to_string(),
        )?;
        Ok(RenewalExit { exit_code: Some(0) })
    });
}

// --- Freshness and blocker ---------------------------------------------------------------

#[test]
fn a_fresh_reading_is_current_with_nothing_blocking_and_nothing_to_say_about_renewal() {
    let (_home, _b, core, env) = home_with_b(hours(5));
    env.push_usage_reply(ok());

    let report = core.refresh_usage().unwrap();

    let situation = only(&report).at(now());
    assert_eq!(situation.freshness, Freshness::Current);
    assert_eq!(situation.blocker, None);
    assert_eq!(situation.renewal, RenewalSituation::Quiet);
    assert!(situation.reading.is_some());
}

#[test]
fn an_account_never_read_has_no_reading_and_nothing_blocking() {
    let (_home, _b, core, _env) = home_with_b(hours(5));

    let situation = only(&core.usage().unwrap()).at(now());

    assert_eq!(situation.freshness, Freshness::NoReading);
    assert_eq!(situation.blocker, None);
}

#[test]
fn the_same_report_goes_stale_when_drawn_more_than_10_minutes_later() {
    let (_home, _b, core, env) = home_with_b(hours(5));
    env.push_usage_reply(ok());
    let report = core.refresh_usage().unwrap();
    let usage = only(&report);

    assert_eq!(usage.at(now() + mins(10)).freshness, Freshness::Current);
    assert_eq!(usage.at(now() + mins(11)).freshness, Freshness::Stale);
    assert_eq!(usage.as_reported().freshness, Freshness::Current);
}

#[test]
fn a_window_resets_between_two_drawings_of_the_same_report() {
    let (_home, _b, core, env) = home_with_b(hours(5));
    env.push_usage_reply(ok());
    let report = core.refresh_usage().unwrap();
    let usage = only(&report);
    let reset: Timestamp = SESSION_RESET.parse().unwrap();

    let before = usage
        .at(reset - SignedDuration::from_secs(1))
        .reading
        .unwrap();
    let after = usage.at(reset).reading.unwrap();

    assert_eq!(before.session.unwrap().utilization, 35.0);
    assert_eq!(
        after.session,
        Some(UsageWindow {
            utilization: 0.0,
            resets_at: None
        })
    );
    assert_eq!(after.weekly, before.weekly);
}

#[test]
fn a_failed_refresh_blocks_and_keeps_the_last_reading_stale() {
    let (_home, _b, core, env) = home_with_b(hours(5));
    env.push_usage_reply(ok());
    core.refresh_usage().unwrap();
    env.advance(mins(2));
    env.push_usage_reply(EndpointReply::Http {
        status: 429,
        retry_after: None,
        body: String::new(),
    });

    let report = core.refresh_usage().unwrap();

    let situation = only(&report).at(env.now());
    assert_eq!(
        situation.blocker,
        Some(UsageBlocker::RefreshFailed {
            failure: UsageFailure::RateLimited
        })
    );
    assert_eq!(situation.freshness, Freshness::Stale);
    assert!(situation.reading.is_some());
}

#[test]
fn an_expired_token_with_a_reading_is_stale_and_says_why() {
    let (_home, b, core, env) = home_with_b(hours(1));
    env.push_usage_reply(ok());
    core.refresh_usage().unwrap();
    b.credentials_expiring(now() - mins(1));

    let situation = only(&core.usage().unwrap()).at(now());

    assert_eq!(situation.freshness, Freshness::Stale);
    assert_eq!(situation.blocker, Some(UsageBlocker::TokenExpired));
    assert!(situation.token_expired());
}

#[test]
fn an_account_with_no_logged_in_instance_is_blocked_by_it() {
    let (_home, b, core, _env) = home_with_b(hours(5));
    std::fs::remove_file(b.dir.join(".credentials.json")).unwrap();

    let situation = only(&core.usage().unwrap()).at(now());

    assert_eq!(situation.blocker, Some(UsageBlocker::NotLoggedIn));
    assert_eq!(situation.renewal, RenewalSituation::Quiet);
}

// --- Token renewal -----------------------------------------------------------------------

#[test]
fn with_renewal_off_an_expired_token_says_claude_code_renews_it_in_its_instance() {
    let (home, b, core, env) = home_with_b(-hours(1));
    renewal_off(&home);

    let report = core.refresh_usage().unwrap();

    assert!(env.renewal_invocations().is_empty());
    assert_eq!(
        only(&report).at(now()).renewal,
        RenewalSituation::Off { config_dir: b.dir }
    );
}

#[test]
fn with_renewal_off_a_valid_token_says_nothing() {
    let (home, _b, core, _env) = home_with_b(hours(5));
    renewal_off(&home);

    assert_eq!(
        only(&core.usage().unwrap()).at(now()).renewal,
        RenewalSituation::Quiet
    );
}

#[test]
fn an_expired_token_not_yet_tried_waits_for_the_next_refresh() {
    let (_home, b, core, _env) = home_with_b(-hours(1));

    let situation = only(&core.usage().unwrap()).at(now());

    assert_eq!(
        situation.renewal,
        RenewalSituation::Waiting {
            config_dir: b.dir,
            next_at: None
        }
    );
}

#[test]
fn a_failed_renewal_retries_at_its_time_then_at_the_next_refresh_once_that_passed() {
    let (_home, b, core, env) = home_with_b(-hours(1));
    env.on_renewal(|_| Ok(RenewalExit { exit_code: Some(0) }));

    let report = core.refresh_usage().unwrap();
    let usage = only(&report);

    let failed = |retry_at| RenewalSituation::Failed {
        config_dir: b.dir.clone(),
        at: now(),
        failure: RenewalFailure::StillExpired,
        retry_at,
    };
    assert_eq!(usage.at(now()).renewal, failed(Some(now() + mins(30))));
    assert_eq!(usage.at(now() + mins(31)).renewal, failed(None));
}

#[test]
fn a_renewed_token_says_when_while_it_stays_valid() {
    let (_home, b, core, env) = home_with_b(-hours(1));
    renews(&env, hours(8));
    env.push_usage_reply(ok());

    let report = core.refresh_usage().unwrap();

    let situation = only(&report).at(now());
    assert_eq!(
        situation.renewal,
        RenewalSituation::Renewed {
            config_dir: b.dir,
            at: now()
        }
    );
    assert_eq!(situation.freshness, Freshness::Current);
}

#[test]
fn a_token_renewed_once_that_expired_again_is_waiting_not_renewed() {
    let (_home, b, core, env) = home_with_b(-hours(1));
    renews(&env, hours(1));
    env.push_usage_reply(ok());
    core.refresh_usage().unwrap();

    env.advance(hours(2));
    let situation = only(&core.usage().unwrap()).at(env.now());

    assert_eq!(
        situation.renewal,
        RenewalSituation::Waiting {
            config_dir: b.dir,
            next_at: None
        }
    );
    assert_eq!(situation.blocker, Some(UsageBlocker::TokenExpired));
}

// --- What the JSON carries and where the UI finds it -------------------------------------

#[test]
fn the_report_serializes_the_situation_not_the_raw_fields() {
    let (_home, _b, core, env) = home_with_b(-hours(1));
    env.on_renewal(|_| Ok(RenewalExit { exit_code: Some(0) }));

    let report = core.refresh_usage().unwrap();
    let json = serde_json::to_value(&report).unwrap();

    let account = &json["accounts"][0];
    assert_eq!(account["situation"]["freshness"], "no_reading");
    assert_eq!(account["situation"]["blocker"]["kind"], "token_expired");
    assert_eq!(account["situation"]["renewal"]["state"], "failed");
    assert_eq!(
        account["situation"]["renewal"]["failure"]["kind"],
        "still_expired"
    );
    for raw in ["status", "stale", "reading", "renewal"] {
        assert!(account.get(raw).is_none(), "{raw} in {account}");
    }
}

#[test]
fn the_instance_where_the_renewal_runs_carries_it_and_the_others_do_not() {
    let (home, b, core, env) = home_with_b(-hours(1));
    let c = home.linked(".claude-c");
    c.logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() - hours(2));
    core.register(&c.dir).unwrap();
    env.on_renewal(|_| Ok(RenewalExit { exit_code: Some(0) }));
    core.refresh_usage().unwrap();

    let registry = core.registry().unwrap();

    let renewal_of = |dir: &std::path::Path| {
        registry
            .instances
            .iter()
            .find(|i| i.config_dir == dir)
            .unwrap()
            .renewal
            .clone()
    };
    // B's token expired last: the renewal runs there.
    assert!(matches!(
        renewal_of(&b.dir),
        RenewalSituation::Failed { config_dir, .. } if config_dir == b.dir
    ));
    assert_eq!(renewal_of(&c.dir), RenewalSituation::Quiet);
}
