//! Usage readings from the endpoint (spec stories 51–59; ticket #5).
//!
//! Every test runs on a temporary home with the fake environment port: scripted endpoint
//! replies and a controllable clock. Nothing here reaches the network.

mod common;

use std::fs;

use common::{FIXTURE_REFRESH, FIXTURE_TOKEN, TempHome, UUID_A, UUID_B, now};
use jiff::{SignedDuration, Timestamp};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    AccountUsage, AccountUuid, Core, EndpointReply, LoginState, UsageFailure, UsageReport,
    UsageSource, UsageStatus, UsageWarning, UsageWindow,
};
use serde_json::json;

const TOKEN_A: &str = "sk-ant-oat01-FIXTURE-A-must-never-leak-aaaaaaaaaaaa";
const TOKEN_A2: &str = "sk-ant-oat01-FIXTURE-A2-must-never-leak-cccccccccccc";
const TOKEN_B: &str = "sk-ant-oat01-FIXTURE-B-must-never-leak-bbbbbbbbbbbb";

const SESSION_RESET: &str = "2026-10-15T15:00:00Z";
const WEEKLY_RESET: &str = "2026-10-20T08:00:00Z";

fn secs(n: i64) -> SignedDuration {
    SignedDuration::from_secs(n)
}

fn hours(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n)
}

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn ok(body: serde_json::Value) -> EndpointReply {
    EndpointReply::Http {
        status: 200,
        retry_after: None,
        body: body.to_string(),
    }
}

fn status(code: u16) -> EndpointReply {
    EndpointReply::Http {
        status: code,
        retry_after: None,
        body: r#"{"error":{"type":"error","message":"nope"}}"#.to_owned(),
    }
}

/// A reply in today's shape: windows in `limits[]`, flat keys null.
fn limits_body(session: f64, weekly: f64) -> serde_json::Value {
    json!({
        "five_hour": null,
        "seven_day": null,
        "seven_day_opus": null,
        "limits": [
            {"kind": "session", "group": "session", "percent": session,
             "resets_at": "2026-10-15T15:00:00+00:00", "severity": "normal", "is_active": true},
            {"kind": "weekly_all", "group": "weekly", "percent": weekly,
             "resets_at": "2026-10-20T08:00:00.000Z", "severity": "normal", "is_active": true},
        ]
    })
}

fn window(utilization: f64, resets_at: &str) -> Option<UsageWindow> {
    Some(UsageWindow {
        utilization,
        resets_at: Some(ts(resets_at)),
    })
}

/// A home with the primary instance logged in as A with a valid token, registered.
fn home_with_a() -> (TempHome, Core, FakeEnvironment) {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A, now() + hours(5));
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();
    (home, core, env)
}

fn only(report: &UsageReport) -> &AccountUsage {
    assert_eq!(report.accounts.len(), 1, "{report:?}");
    &report.accounts[0]
}

fn failed(failure: UsageFailure) -> UsageStatus {
    UsageStatus::Failed { failure }
}

// --- Reading and mapping the reply -------------------------------------------------------

#[test]
fn a_refresh_reads_session_weekly_and_model_windows_from_limits() {
    let (_home, core, env) = home_with_a();
    let mut body = limits_body(35.0, 14.0);
    body["limits"].as_array_mut().unwrap().extend([
        json!({"kind": "weekly_scoped", "group": "weekly", "percent": 58,
               "resets_at": "2026-10-20T08:00:00+00:00",
               "scope": {"model": {"id": null, "display_name": "Fable"}}}),
        // A scope without a model (a surface) is not a model window.
        json!({"kind": "weekly_scoped", "group": "weekly", "percent": 90,
               "resets_at": "2026-10-20T08:00:00+00:00",
               "scope": {"model": null, "surface": {"display_name": "Cowork"}}}),
        // A model row with no data yet is skipped.
        json!({"kind": "weekly_scoped", "group": "weekly", "percent": null, "resets_at": null,
               "scope": {"model": {"display_name": "Haiku"}}}),
        // Kinds mccboard doesn't know are ignored.
        json!({"kind": "monthly_spend", "group": "spend", "percent": 12, "resets_at": null}),
    ]);
    env.push_usage_reply(ok(body));

    let report = core.refresh_usage().unwrap();

    let usage = only(&report);
    assert_eq!(usage.account.uuid, AccountUuid::new(UUID_A));
    assert_eq!(usage.status, UsageStatus::Ok);
    assert!(usage.called);
    assert!(!usage.stale);
    let reading = usage.reading.as_ref().unwrap();
    assert_eq!(reading.source, UsageSource::Endpoint);
    assert_eq!(reading.read_at, now());
    assert_eq!(reading.session, window(35.0, SESSION_RESET));
    assert_eq!(reading.weekly, window(14.0, WEEKLY_RESET));
    assert_eq!(reading.models.len(), 1);
    assert_eq!(reading.models[0].model, "Fable");
    assert_eq!(reading.models[0].utilization, 58.0);
    assert_eq!(env.usage_calls(), 1);
    assert_eq!(env.usage_calls_with_token(TOKEN_A), 1);
}

#[test]
fn the_flat_five_hour_and_seven_day_keys_are_read_when_limits_are_absent() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(json!({
        "five_hour": {"utilization": 35.0, "resets_at": "2026-10-15T15:00:00+00:00"},
        "seven_day": {"utilization": 14, "resets_at": "2026-10-20T08:00:00+00:00"},
        "seven_day_sonnet": {"utilization": 39.0, "resets_at": "2026-10-20T08:00:00+00:00"},
        "extra_usage": {"is_enabled": true, "monthly_limit": 100000, "used_credits": 0.0,
                        "utilization": null},
    })));

    let report = core.refresh_usage().unwrap();

    let reading = only(&report).reading.clone().unwrap();
    assert_eq!(reading.session, window(35.0, SESSION_RESET));
    assert_eq!(reading.weekly, window(14.0, WEEKLY_RESET));
    // No model name is hard-coded, so flat per-model keys don't become model windows.
    assert!(reading.models.is_empty());
}

#[test]
fn limits_rows_win_over_flat_keys_and_flat_keys_fill_gaps() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(json!({
        "five_hour": {"utilization": 99.0, "resets_at": "2026-10-15T15:00:00+00:00"},
        "seven_day": {"utilization": 14.0, "resets_at": "2026-10-20T08:00:00+00:00"},
        "limits": [
            {"kind": "session", "percent": 35, "resets_at": "2026-10-15T15:00:00+00:00"},
        ],
    })));

    let report = core.refresh_usage().unwrap();

    let reading = only(&report).reading.clone().unwrap();
    assert_eq!(reading.session, window(35.0, SESSION_RESET));
    assert_eq!(reading.weekly, window(14.0, WEEKLY_RESET));
}

#[test]
fn utilization_outside_0_100_is_clamped() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(limits_body(140.0, -3.0)));

    let report = core.refresh_usage().unwrap();

    let reading = only(&report).reading.clone().unwrap();
    assert_eq!(reading.session.unwrap().utilization, 100.0);
    assert_eq!(reading.weekly.unwrap().utilization, 0.0);
}

#[test]
fn malformed_replies_fail_as_malformed() {
    for body in [
        "not json at all",
        "[]",
        r#"{"limits": "nope"}"#,
        r#"{"five_hour": null, "seven_day": null, "limits": []}"#,
    ] {
        let (_home, core, env) = home_with_a();
        env.push_usage_reply(EndpointReply::Http {
            status: 200,
            retry_after: None,
            body: body.to_owned(),
        });

        let report = core.refresh_usage().unwrap();

        let usage = only(&report);
        assert_eq!(usage.status, failed(UsageFailure::Malformed), "{body}");
        assert!(usage.reading.is_none());
    }
}

// --- Which token, and when no call is made -----------------------------------------------

#[test]
fn usage_is_read_once_per_account_not_once_per_instance() {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A, now() + hours(5));
    let a2 = home.linked(".claude-work");
    a2.logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A2, now() + hours(5));
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com")
        .credentials_with_token(TOKEN_B, now() + hours(5));
    let (core, env) = home.core();
    for dir in [&primary.dir, &a2.dir, &b.dir] {
        core.register(dir).unwrap();
    }
    env.push_usage_reply(ok(limits_body(10.0, 20.0)));
    env.push_usage_reply(ok(limits_body(30.0, 40.0)));

    let report = core.refresh_usage().unwrap();

    assert_eq!(report.accounts.len(), 2);
    assert_eq!(env.usage_calls(), 2);
    assert_eq!(env.usage_calls_with_token(TOKEN_B), 1);
    assert_eq!(
        env.usage_calls_with_token(TOKEN_A) + env.usage_calls_with_token(TOKEN_A2),
        1
    );
    assert!(report.accounts.iter().all(|u| u.status == UsageStatus::Ok));
}

#[test]
fn an_expired_token_reads_as_token_expired_and_no_call_is_made() {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A, now() - secs(1));
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();
    let before = primary.credentials_bytes();

    let report = core.refresh_usage().unwrap();

    let usage = only(&report);
    assert_eq!(usage.status, UsageStatus::TokenExpired);
    assert!(!usage.called);
    assert_eq!(env.usage_calls(), 0);
    assert_eq!(primary.credentials_bytes(), before);
}

#[test]
fn a_valid_token_from_another_instance_of_the_account_is_used() {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A, now() - hours(1));
    let a2 = home.linked(".claude-work");
    a2.logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A2, now() + hours(5));
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();
    core.register(&a2.dir).unwrap();
    env.push_usage_reply(ok(limits_body(10.0, 20.0)));

    let report = core.refresh_usage().unwrap();

    assert_eq!(only(&report).status, UsageStatus::Ok);
    assert_eq!(env.usage_calls_with_token(TOKEN_A2), 1);
    assert_eq!(env.usage_calls_with_token(TOKEN_A), 0);
}

#[test]
fn an_account_with_no_logged_in_instance_is_not_logged_in() {
    let home = TempHome::new();
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();

    let report = core.refresh_usage().unwrap();

    assert_eq!(only(&report).status, UsageStatus::NotLoggedIn);
    assert_eq!(env.usage_calls(), 0);
}

#[test]
fn an_instance_now_logged_in_as_another_account_lends_no_token() {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A, now() + hours(5));
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();
    // The instance switched to account B after registration; B isn't registered.
    primary.logged_in_as(UUID_B, "b@example.com");

    let report = core.refresh_usage().unwrap();

    assert_eq!(only(&report).account.uuid, AccountUuid::new(UUID_A));
    assert_eq!(only(&report).status, UsageStatus::NotLoggedIn);
    assert_eq!(env.usage_calls(), 0);
}

#[test]
fn the_login_state_and_the_usage_status_agree_on_the_same_credentials() {
    let future = (now() + hours(5)).as_millisecond();
    let past = (now() - hours(1)).as_millisecond();
    let token = TOKEN_A;
    let cases = [
        (
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{token}","expiresAt":{future}}}}}"#),
            UsageStatus::NeverRead,
        ),
        (
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{token}","expiresAt":{past}}}}}"#),
            UsageStatus::TokenExpired,
        ),
        (
            r#"{"claudeAiOauth":{"accessToken":"","expiresAt":0}}"#.to_owned(),
            UsageStatus::NotLoggedIn,
        ),
        (
            format!(r#"{{"claudeAiOauth":{{"expiresAt":{future}}}}}"#),
            UsageStatus::NotLoggedIn,
        ),
        (
            format!(r#"{{"claudeAiOauth":{{"accessToken":null,"expiresAt":{future}}}}}"#),
            UsageStatus::NotLoggedIn,
        ),
        (
            format!(r#"{{"claudeAiOauth":{{"accessToken":42,"expiresAt":{future}}}}}"#),
            UsageStatus::CredentialsUnreadable,
        ),
        (
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{token}"}}}}"#),
            UsageStatus::CredentialsUnreadable,
        ),
    ];
    for (credentials, status) in cases {
        let home = TempHome::new();
        let primary = home.primary();
        primary
            .logged_in_as(UUID_A, "a@example.com")
            .raw_credentials(&credentials);
        let (core, env) = home.core();
        core.register(&primary.dir).unwrap();

        let login = core.observe(&primary.dir).login;
        let usage = core.usage().unwrap();

        assert_eq!(only(&usage).status, status, "{credentials}");
        let agrees = match login {
            LoginState::LoggedIn { .. } => status == UsageStatus::NeverRead,
            LoginState::Expired { .. } => status == UsageStatus::TokenExpired,
            LoginState::NotLoggedIn => status == UsageStatus::NotLoggedIn,
            LoginState::Unknown => status == UsageStatus::CredentialsUnreadable,
        };
        assert!(agrees, "{credentials}: {login:?} vs {status:?}");
        assert_eq!(env.usage_calls(), 0);
    }
}

#[test]
fn unreadable_credentials_make_no_call() {
    for credentials in [
        "{not json",
        r#"{"claudeAiOauth": {"accessToken": "sk-ant-oat01-FIXTURE-x", "expiresAt": "soon"}}"#,
        r#"{"claudeAiOauth": {"accessToken": "sk-ant-oat01-FIXTURE-x"}}"#,
    ] {
        let home = TempHome::new();
        let primary = home.primary();
        primary
            .logged_in_as(UUID_A, "a@example.com")
            .raw_credentials(credentials);
        let (core, env) = home.core();
        core.register(&primary.dir).unwrap();

        let report = core.refresh_usage().unwrap();

        assert_eq!(
            only(&report).status,
            UsageStatus::CredentialsUnreadable,
            "{credentials}"
        );
        assert_eq!(env.usage_calls(), 0);
    }
}

#[test]
fn an_emptied_token_reads_as_not_logged_in() {
    let home = TempHome::new();
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();
    // An empty token is what Claude Code leaves after a dead refresh: logged out.
    primary.raw_credentials(r#"{"claudeAiOauth":{"accessToken":"","expiresAt":0}}"#);

    let report = core.refresh_usage().unwrap();

    assert_eq!(only(&report).status, UsageStatus::NotLoggedIn);
    assert_eq!(env.usage_calls(), 0);
}

#[test]
fn reading_usage_without_refreshing_never_calls() {
    let (_home, core, env) = home_with_a();

    let report = core.usage().unwrap();

    let usage = only(&report);
    assert_eq!(usage.status, UsageStatus::NeverRead);
    assert!(usage.reading.is_none());
    assert!(!usage.called);
    assert_eq!(env.usage_calls(), 0);
}

#[test]
fn nothing_registered_means_no_accounts_and_no_call() {
    let home = TempHome::new();
    let (core, env) = home.core();

    let report = core.refresh_usage().unwrap();

    assert!(report.accounts.is_empty());
    assert_eq!(env.usage_calls(), 0);
}

// --- Rate limits -------------------------------------------------------------------------

#[test]
fn at_most_one_call_per_60_seconds_per_account_even_on_manual_refresh() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(limits_body(10.0, 20.0)));
    env.push_usage_reply(ok(limits_body(11.0, 21.0)));
    core.refresh_usage().unwrap();

    env.advance(secs(59));
    let report = core.refresh_usage().unwrap();

    let usage = only(&report);
    assert_eq!(env.usage_calls(), 1);
    assert!(!usage.called);
    assert_eq!(usage.status, UsageStatus::Ok);
    assert_eq!(usage.next_call_at, Some(now() + secs(60)));
    assert_eq!(usage.reading.as_ref().unwrap().read_at, now());

    env.advance(secs(1));
    let report = core.refresh_usage().unwrap();

    assert_eq!(env.usage_calls(), 2);
    assert!(only(&report).called);
    assert_eq!(
        only(&report)
            .reading
            .as_ref()
            .unwrap()
            .session
            .unwrap()
            .utilization,
        11.0
    );
}

#[test]
fn after_a_429_the_next_call_waits_120_s_then_doubles_up_to_300_s_and_resets_on_success() {
    let (_home, core, env) = home_with_a();
    let mut at = now();
    // 429 → 120 s, error → 240 s, 429 → 300 s (cap), error → 300 s.
    for (reply, wait) in [
        (status(429), 120),
        (status(500), 240),
        (status(429), 300),
        (status(503), 300),
    ] {
        env.set_now(at);
        env.push_usage_reply(reply);
        let report = core.refresh_usage().unwrap();
        assert!(only(&report).called);
        assert_eq!(only(&report).next_call_at, Some(at + secs(wait)));

        env.set_now(at + secs(wait - 1));
        let report = core.refresh_usage().unwrap();
        assert!(!only(&report).called, "called before {wait} s");
        at += secs(wait);
    }
    let calls = env.usage_calls();
    assert_eq!(calls, 4);

    env.set_now(at);
    env.push_usage_reply(ok(limits_body(10.0, 20.0)));
    let report = core.refresh_usage().unwrap();
    assert_eq!(only(&report).status, UsageStatus::Ok);
    assert_eq!(only(&report).next_call_at, Some(at + secs(60)));

    // After a success, the next failure starts from 120 s again.
    env.set_now(at + secs(60));
    env.push_usage_reply(status(429));
    let report = core.refresh_usage().unwrap();
    assert_eq!(only(&report).status, failed(UsageFailure::RateLimited));
    assert_eq!(only(&report).next_call_at, Some(at + secs(180)));
}

#[test]
fn a_longer_retry_after_is_honoured() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(EndpointReply::Http {
        status: 429,
        retry_after: Some(secs(600)),
        body: String::new(),
    });

    let report = core.refresh_usage().unwrap();

    assert_eq!(only(&report).next_call_at, Some(now() + secs(600)));
}

#[test]
fn rate_limits_and_readings_survive_a_restart() {
    let (home, core, env) = home_with_a();
    env.push_usage_reply(ok(limits_body(10.0, 20.0)));
    core.refresh_usage().unwrap();

    let (restarted, env2) = home.core();
    env2.set_now(now() + secs(30));
    let report = restarted.refresh_usage().unwrap();

    assert_eq!(env2.usage_calls(), 0);
    let usage = only(&report);
    assert_eq!(usage.status, UsageStatus::Ok);
    assert_eq!(
        usage.reading.as_ref().unwrap().session,
        window(10.0, SESSION_RESET)
    );
    assert_eq!(restarted.usage().unwrap(), report);
}

#[test]
fn errors_of_every_kind_back_off() {
    for (reply, failure) in [
        (status(429), UsageFailure::RateLimited),
        (status(401), UsageFailure::Unauthorized),
        (status(403), UsageFailure::Forbidden),
        (status(500), UsageFailure::HttpStatus { status: 500 }),
        (
            EndpointReply::Unreachable {
                reason: "timeout".to_owned(),
            },
            UsageFailure::Unreachable {
                reason: "timeout".to_owned(),
            },
        ),
    ] {
        let (_home, core, env) = home_with_a();
        env.push_usage_reply(reply);

        let report = core.refresh_usage().unwrap();

        assert_eq!(only(&report).status, failed(failure));
        assert_eq!(only(&report).next_call_at, Some(now() + secs(120)));
    }
}

// --- Stale readings and resets -----------------------------------------------------------

#[test]
fn a_failed_refresh_keeps_the_last_reading_marked_stale() {
    let (home, core, env) = home_with_a();
    env.push_usage_reply(ok(limits_body(10.0, 20.0)));
    core.refresh_usage().unwrap();

    env.advance(secs(60));
    env.push_usage_reply(status(500));
    let report = core.refresh_usage().unwrap();

    let usage = only(&report);
    assert_eq!(
        usage.status,
        failed(UsageFailure::HttpStatus { status: 500 })
    );
    assert!(usage.stale);
    let reading = usage.reading.as_ref().unwrap();
    assert_eq!(reading.read_at, now());
    assert_eq!(reading.session, window(10.0, SESSION_RESET));

    // Still there, still stale, after a restart.
    let (restarted, env2) = home.core();
    env2.set_now(now() + secs(61));
    let usage = restarted.usage().unwrap().accounts.remove(0);
    assert!(usage.stale);
    assert!(usage.reading.is_some());
}

#[test]
fn an_expired_token_keeps_the_last_reading_marked_stale() {
    let (home, core, env) = home_with_a();
    env.push_usage_reply(ok(limits_body(10.0, 20.0)));
    core.refresh_usage().unwrap();
    home.primary()
        .credentials_with_token(TOKEN_A, now() + secs(30));

    env.advance(secs(60));
    let report = core.refresh_usage().unwrap();

    let usage = only(&report);
    assert_eq!(usage.status, UsageStatus::TokenExpired);
    assert!(usage.stale);
    assert!(usage.reading.is_some());
    assert_eq!(env.usage_calls(), 1);
}

#[test]
fn an_old_reading_is_stale_even_without_a_failure() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(limits_body(10.0, 20.0)));
    core.refresh_usage().unwrap();

    env.advance(SignedDuration::from_mins(9));
    assert!(!only(&core.usage().unwrap()).stale);

    env.advance(SignedDuration::from_mins(2));
    assert!(only(&core.usage().unwrap()).stale);
}

#[test]
fn a_window_whose_reset_time_passed_reads_as_zero() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(limits_body(80.0, 20.0)));
    core.refresh_usage().unwrap();

    env.set_now(ts(SESSION_RESET));
    let report = core.usage().unwrap();

    let reading = only(&report).reading.clone().unwrap();
    assert_eq!(
        reading.session,
        Some(UsageWindow {
            utilization: 0.0,
            resets_at: None
        })
    );
    assert_eq!(reading.weekly, window(20.0, WEEKLY_RESET));
}

#[test]
fn a_corrupt_state_file_is_discarded_with_a_warning() {
    let (home, core, env) = home_with_a();
    home.write(".mccboard/state/usage.json", "{ this is not json");

    let report = core.usage().unwrap();
    assert_eq!(report.warnings, vec![UsageWarning::StateDiscarded]);
    assert_eq!(only(&report).status, UsageStatus::NeverRead);

    env.push_usage_reply(ok(limits_body(10.0, 20.0)));
    let report = core.refresh_usage().unwrap();
    assert_eq!(only(&report).status, UsageStatus::Ok);
    // Replaced by a valid file: no warning the next time.
    assert!(core.usage().unwrap().warnings.is_empty());
}

#[test]
fn readings_live_in_mccboard_state_not_in_claude_code_folders() {
    let (home, core, env) = home_with_a();
    env.push_usage_reply(ok(limits_body(10.0, 20.0)));

    core.refresh_usage().unwrap();

    let state = home.path().join(".mccboard/state/usage.json");
    let text = fs::read_to_string(state).unwrap();
    assert!(text.contains(UUID_A));
    let claude_dir: Vec<_> = fs::read_dir(home.path().join(".claude"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(claude_dir, vec![".credentials.json"]);
}

// --- Token safety ------------------------------------------------------------------------

#[test]
fn no_token_reaches_readings_errors_or_persisted_state() {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() + hours(5));
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com")
        .credentials_expiring(now() - hours(1));
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();
    core.register(&b.dir).unwrap();
    let credentials_before = primary.credentials_bytes();

    // Replies that echo the token must not carry it anywhere either.
    let echo = format!("{{\"error\": \"bad token {FIXTURE_TOKEN}\"}}");
    let replies = [
        ok(limits_body(10.0, 20.0)),
        EndpointReply::Http {
            status: 401,
            retry_after: None,
            body: echo.clone(),
        },
        EndpointReply::Http {
            status: 200,
            retry_after: None,
            body: echo.clone(),
        },
        EndpointReply::Http {
            status: 429,
            retry_after: None,
            body: echo,
        },
        EndpointReply::Unreachable {
            reason: "connection refused".to_owned(),
        },
    ];
    let mut outputs = Vec::new();
    for reply in replies {
        env.push_usage_reply(reply);
        let report = core.refresh_usage().unwrap();
        outputs.push(format!("{report:?}"));
        outputs.push(serde_json::to_string(&report).unwrap());
        outputs.push(format!("{:?}", core.usage().unwrap()));
        env.advance(secs(600));
    }
    assert!(env.usage_calls() >= 5);

    // A corrupt config.toml fails with an error: it must not carry the token either.
    let config = home.config_text().unwrap();
    home.write_config("[[instances]\n");
    for result in [core.usage(), core.refresh_usage()] {
        let error = result.unwrap_err();
        outputs.push(format!("{error} {error:?}"));
    }
    home.write_config(&config);

    for (path, text) in home.mccboard_files() {
        outputs.push(format!("{}: {text}", path.display()));
    }
    for text in &outputs {
        for secret in [FIXTURE_TOKEN, FIXTURE_REFRESH, "FIXTURE"] {
            assert!(!text.contains(secret), "{text}");
        }
    }
    // Credentials are read-only: never written, never refreshed.
    assert_eq!(primary.credentials_bytes(), credentials_before);
}
