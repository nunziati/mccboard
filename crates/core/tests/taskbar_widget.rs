//! The taskbar widget's view model (#45, ADR 0009): its columns in `config.toml` order, the
//! bar's segments and pace, the time-to-reset texts, the states, the tooltip, when to redraw,
//! and its two preferences.
//!
//! Every test runs on a temporary home with the fake environment port and its fake clock, and
//! gets its usage report the way the app does: `refresh_usage` or `usage`, then
//! `taskbar_widget`. None of it needs Windows.

mod common;

use common::{TempHome, UUID_A, UUID_B, now};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    AccountUpdate, AccountUuid, ColumnState, Core, EndpointReply, Language, ResetPart,
    TaskbarTheme, TaskbarWidget, UsageReport,
};
use serde_json::json;

const TOKEN_A: &str = "sk-ant-oat01-FIXTURE-A-must-never-leak-aaaaaaaaaaaa";
const TOKEN_B: &str = "sk-ant-oat01-FIXTURE-B-must-never-leak-bbbbbbbbbbbb";

fn secs(n: i64) -> SignedDuration {
    SignedDuration::from_secs(n)
}

fn mins(n: i64) -> SignedDuration {
    SignedDuration::from_mins(n)
}

fn hours(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n)
}

fn days(n: i64) -> SignedDuration {
    hours(24 * n)
}

/// A reply whose session window (without a reset time when `session_reset` is `None`: idle)
/// and weekly window reset at the given times.
fn reply(
    session: f64,
    session_reset: Option<Timestamp>,
    weekly: f64,
    weekly_reset: Timestamp,
    models: &[(&str, f64)],
) -> EndpointReply {
    let mut limits = vec![
        json!({"kind": "session", "percent": session,
               "resets_at": session_reset.map(|at| at.to_string())}),
        json!({"kind": "weekly_all", "percent": weekly, "resets_at": weekly_reset.to_string()}),
    ];
    for (model, percent) in models {
        limits.push(json!({"kind": "weekly_scoped", "percent": percent,
                           "resets_at": weekly_reset.to_string(),
                           "scope": {"model": {"display_name": model}}}));
    }
    EndpointReply::Http {
        status: 200,
        retry_after: None,
        body: json!({ "limits": limits }).to_string(),
    }
}

/// A reply with the session resetting in 3 hours and the weekly window in 4 days.
fn ok(session: f64, weekly: f64) -> EndpointReply {
    reply(
        session,
        Some(now() + hours(3)),
        weekly,
        now() + days(4),
        &[],
    )
}

fn rate_limited() -> EndpointReply {
    EndpointReply::Http {
        status: 429,
        retry_after: None,
        body: String::new(),
    }
}

fn widget(core: &Core, report: &UsageReport) -> TaskbarWidget {
    core.taskbar_widget(report, Language::English, &TimeZone::UTC)
        .unwrap()
}

/// A home with A (primary instance) and B (`~/.claude-b`), registered in that order and
/// logged in with valid tokens. Labels "A" and "B" are suggested from the folders.
fn home_with_a_and_b() -> (TempHome, Core, FakeEnvironment) {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A, now() + hours(5));
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com")
        .credentials_with_token(TOKEN_B, now() + hours(5));
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();
    core.register(&b.dir).unwrap();
    (home, core, env)
}

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

// --- Columns ---------------------------------------------------------------------------------

#[test]
fn one_column_per_account_with_its_session_and_weekly_rows() {
    let (_home, core, env) = home_with_a_and_b();
    // Accounts are called in label order: A, then B.
    env.push_usage_reply(ok(7.0, 23.0));
    env.push_usage_reply(ok(31.0, 88.0));

    let view = widget(&core, &core.refresh_usage().unwrap());

    assert!(view.enabled && view.visible());
    assert!(!view.account_labels);
    assert_eq!(view.row_labels, ["5h".to_owned(), "7d".to_owned()]);
    assert_eq!(view.columns.len(), 2);
    let a = &view.columns[0];
    assert_eq!(a.account, AccountUuid::new(UUID_A));
    assert_eq!(a.label, "A");
    assert_eq!(a.state, ColumnState::Current);
    assert_eq!(a.state.placeholder(), None);
    assert_eq!((a.session.percent, a.session.segments), (7, 1));
    assert_eq!((a.weekly.percent, a.weekly.segments), (23, 2));
    assert_eq!(a.session.reset.text(), "3h 0m");
    assert_eq!(a.weekly.reset.text(), "4d 0h 0m");
    let b = &view.columns[1];
    assert_eq!((b.session.segments, b.weekly.segments), (3, 9));
}

#[test]
fn columns_follow_the_order_of_accounts_in_config_toml_not_their_labels() {
    let (_home, core, env) = home_with_a_and_b();
    // A is renamed "Z": in label order it now comes after B, in the file it is still first.
    core.update_account(
        &AccountUuid::new(UUID_A),
        AccountUpdate {
            label: Some("Z".to_owned()),
            color: None,
            launcher: None,
        },
    )
    .unwrap();
    env.push_usage_reply(ok(50.0, 50.0));
    env.push_usage_reply(ok(10.0, 10.0));

    let report = core.refresh_usage().unwrap();
    assert_eq!(report.accounts[0].account.uuid.as_str(), UUID_B);
    let view = widget(&core, &report);

    let order: Vec<&str> = view.columns.iter().map(|c| c.account.as_str()).collect();
    assert_eq!(order, [UUID_A, UUID_B]);
    assert_eq!(view.columns[0].label, "Z");
}

#[test]
fn a_column_order_written_by_hand_is_kept() {
    let (home, core, env) = home_with_a_and_b();
    // Move B's table before A's, as a user editing the file would.
    let text = home.config_text().unwrap();
    let a_at = text.find(&format!("[accounts.{UUID_A}]")).unwrap();
    let b_at = text.find(&format!("[accounts.{UUID_B}]")).unwrap();
    let a_end = b_at;
    let b_end = text[b_at..]
        .find("\n[")
        .map_or(text.len(), |end| b_at + end + 1);
    let reordered = format!(
        "{}{}{}{}",
        &text[..a_at],
        &text[b_at..b_end],
        &text[a_at..a_end],
        &text[b_end..]
    );
    home.write_config(&reordered);
    env.push_usage_reply(ok(1.0, 1.0));
    env.push_usage_reply(ok(2.0, 2.0));

    let view = widget(&core, &core.refresh_usage().unwrap());

    let order: Vec<&str> = view.columns.iter().map(|c| c.account.as_str()).collect();
    assert_eq!(order, [UUID_B, UUID_A]);
}

#[test]
fn with_no_account_there_is_nothing_to_show() {
    let home = TempHome::new();
    let (core, _env) = home.core();

    let view = widget(&core, &core.usage().unwrap());

    assert!(view.columns.is_empty());
    assert!(!view.visible());
}

// --- The bar -------------------------------------------------------------------------------

#[test]
fn a_segment_is_filled_for_each_ten_percent_and_one_above_zero() {
    let cases = [
        (0.0, 0),
        (0.4, 1),
        (4.0, 1),
        (14.9, 1),
        (15.0, 2),
        (55.0, 6),
        (94.0, 9),
        (95.0, 10),
        (100.0, 10),
    ];
    for (utilization, segments) in cases {
        let (_home, core, env) = home_with_a();
        env.push_usage_reply(ok(utilization, utilization));
        let view = widget(&core, &core.refresh_usage().unwrap());
        assert_eq!(view.columns[0].session.segments, segments, "{utilization}%");
        assert_eq!(view.columns[0].weekly.segments, segments, "{utilization}%");
    }
}

#[test]
fn the_pace_is_the_part_of_the_window_already_passed() {
    let (_home, core, env) = home_with_a();
    // Session: 2 of its 5 hours left. Weekly: 3.5 of its 7 days left.
    env.push_usage_reply(reply(
        40.0,
        Some(now() + hours(2)),
        10.0,
        now() + hours(84),
        &[],
    ));
    let report = core.refresh_usage().unwrap();

    let view = widget(&core, &report);
    let column = &view.columns[0];
    assert!((column.session.pace.unwrap() - 0.6).abs() < 1e-9);
    assert!((column.weekly.pace.unwrap() - 0.5).abs() < 1e-9);

    // An hour later, from the same report.
    env.advance(hours(1));
    let later = widget(&core, &report);
    assert!((later.columns[0].session.pace.unwrap() - 0.8).abs() < 1e-9);
}

#[test]
fn an_idle_session_window_has_an_empty_bar_a_dash_and_no_pace() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(reply(0.0, None, 12.0, now() + days(6), &[]));

    let view = widget(&core, &core.refresh_usage().unwrap());

    let session = &view.columns[0].session;
    assert_eq!(session.segments, 0);
    assert_eq!(session.pace, None);
    assert!(session.reset.parts.is_empty());
    assert_eq!(session.reset.text(), "—");
    assert!(view.columns[0].weekly.pace.is_some());
}

#[test]
fn a_window_whose_reset_passed_is_idle_until_the_next_reading() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(90.0, 20.0));
    let report = core.refresh_usage().unwrap();

    env.advance(hours(3) + secs(1));
    let view = widget(&core, &report);

    let session = &view.columns[0].session;
    assert_eq!((session.percent, session.segments), (0, 0));
    assert_eq!(session.pace, None);
    assert_eq!(session.reset.text(), "—");
}

// --- Time to reset ---------------------------------------------------------------------------

#[test]
fn the_time_to_reset_drops_leading_zero_units_and_keeps_later_ones() {
    let (_home, core, env) = home_with_a();
    let weekly_reset = now() + days(7);
    env.push_usage_reply(reply(5.0, None, 30.0, weekly_reset, &[]));
    let report = core.refresh_usage().unwrap();

    let cases = [
        (days(3) + hours(4) + mins(7), "3d 4h 7m"),
        (days(6) + hours(23) + mins(59), "6d 23h 59m"),
        (days(3) + mins(7), "3d 0h 7m"),
        (days(3) + hours(4), "3d 4h 0m"),
        (days(3), "3d 0h 0m"),
        (hours(5) + mins(3), "5h 3m"),
        (hours(5), "5h 0m"),
        (mins(42), "42m"),
        (mins(1), "1m"),
        // Seconds in the last minute.
        (secs(59), "59s"),
        (secs(42), "42s"),
        (secs(1), "1s"),
        // Minutes are whole: 42 minutes and 59 seconds still read 42 minutes.
        (mins(42) + secs(59), "42m"),
        (SignedDuration::from_millis(59_500), "59s"),
    ];
    for (left, text) in cases {
        env.set_now(weekly_reset - left);
        let view = widget(&core, &report);
        assert_eq!(view.columns[0].weekly.reset.text(), text, "{left:?} left");
    }
}

#[test]
fn every_number_keeps_a_slot_so_hours_and_minutes_line_up() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(reply(
        5.0,
        Some(now() + hours(4) + mins(3)),
        30.0,
        now() + days(3) + mins(7),
        &[],
    ));

    let view = widget(&core, &core.refresh_usage().unwrap());

    let column = &view.columns[0];
    assert_eq!(
        column.session.reset.parts,
        [
            ResetPart {
                value: 4,
                unit: 'h',
                slot: 2
            },
            ResetPart {
                value: 3,
                unit: 'm',
                slot: 2
            },
        ]
    );
    assert_eq!(
        column.weekly.reset.parts,
        [
            ResetPart {
                value: 3,
                unit: 'd',
                slot: 1
            },
            ResetPart {
                value: 0,
                unit: 'h',
                slot: 2
            },
            ResetPart {
                value: 7,
                unit: 'm',
                slot: 2
            },
        ]
    );
}

#[test]
fn the_widget_redraws_every_minute_and_every_second_in_the_last_minute() {
    let (_home, core, env) = home_with_a();
    let session_reset = now() + hours(3) + secs(30);
    env.push_usage_reply(reply(
        5.0,
        Some(session_reset),
        30.0,
        now() + days(4) + secs(45),
        &[],
    ));
    let report = core.refresh_usage().unwrap();

    // The session's minutes change first, in 30 seconds.
    let view = widget(&core, &report);
    assert_eq!(view.columns[0].session.reset.text(), "3h 0m");
    assert_eq!(view.redraw_in, std::time::Duration::from_millis(30_001));

    // In the last minute: each second.
    env.set_now(session_reset - SignedDuration::from_millis(42_400));
    let view = widget(&core, &report);
    assert_eq!(view.columns[0].session.reset.text(), "42s");
    assert_eq!(view.redraw_in, std::time::Duration::from_millis(401));

    // Without any reset time, once a minute.
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(rate_limited());
    let view = widget(&core, &core.refresh_usage().unwrap());
    assert_eq!(view.redraw_in, std::time::Duration::from_secs(60));
}

// --- States --------------------------------------------------------------------------------

#[test]
fn a_stale_reading_fades_its_column_and_keeps_its_values() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(ok(35.0, 14.0));
    env.push_usage_reply(ok(10.0, 20.0));
    core.refresh_usage().unwrap();
    env.advance(secs(61));
    env.push_usage_reply(rate_limited());
    env.push_usage_reply(ok(11.0, 20.0));

    let view = widget(&core, &core.refresh_usage().unwrap());

    let a = &view.columns[0];
    assert_eq!(a.state, ColumnState::Stale);
    assert!(a.state.faded());
    assert_eq!(a.state.placeholder(), None);
    assert_eq!(a.session.percent, 35);
    assert_eq!(view.columns[1].state, ColumnState::Current);
    assert!(!view.columns[1].state.faded());
    assert!(
        view.tooltip
            .lines()
            .next()
            .unwrap()
            .ends_with("read 1 min ago"),
        "{}",
        view.tooltip
    );
}

/// The primary instance logged in as A with a token expired a second ago, registered.
fn home_with_expired_a() -> (TempHome, Core) {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A, now() - secs(1));
    let (core, _env) = home.core();
    core.register(&primary.dir).unwrap();
    (home, core)
}

#[test]
fn an_expired_token_without_a_reading_shows_dots_until_claude_code_renews_it() {
    let (_home, core) = home_with_expired_a();

    // Read without refreshing: mccboard hasn't run Claude Code yet.
    let view = widget(&core, &core.usage().unwrap());

    let column = &view.columns[0];
    assert_eq!(column.state, ColumnState::TokenRenewal);
    assert_eq!(column.state.placeholder(), Some("···"));
    assert_eq!(
        view.tooltip,
        "A · token expired, waiting for Claude Code to renew it"
    );
}

#[test]
fn with_renewal_off_an_expired_token_promises_no_renewal_from_mccboard() {
    let (_home, core) = home_with_expired_a();
    core.set_renew_expired_tokens(false).unwrap();

    let view = widget(&core, &core.refresh_usage().unwrap());

    assert_eq!(view.columns[0].state, ColumnState::TokenRenewal);
    assert_eq!(
        view.tooltip,
        "A · token expired, renewed the next time Claude Code runs with this account"
    );
}

#[test]
fn an_expired_token_with_a_reading_shows_its_last_values_faded() {
    let (home, core, env) = home_with_a_and_b();
    env.push_usage_reply(ok(35.0, 14.0));
    env.push_usage_reply(ok(60.0, 20.0));
    core.refresh_usage().unwrap();
    core.set_renew_expired_tokens(false).unwrap();
    home.linked(".claude-b")
        .credentials_with_token(TOKEN_B, now() + secs(30));
    env.advance(secs(61));
    env.push_usage_reply(ok(36.0, 14.0));

    let view = widget(&core, &core.refresh_usage().unwrap());

    let b = &view.columns[1];
    assert_eq!(b.state, ColumnState::Stale);
    assert_eq!(b.session.percent, 60);
    let line = view.tooltip.lines().nth(1).unwrap();
    assert!(line.contains("token expired"), "{line}");
}

#[test]
fn no_reading_shows_a_dash_and_says_why_in_the_tooltip() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(rate_limited());
    env.push_usage_reply(ok(5.0, 7.0));

    let view = widget(&core, &core.refresh_usage().unwrap());

    let a = &view.columns[0];
    assert_eq!(a.state, ColumnState::NoReading);
    assert_eq!(a.state.placeholder(), Some("—"));
    assert_eq!(view.tooltip.lines().next(), Some("A · can't read usage"));
}

// --- Tooltip -------------------------------------------------------------------------------

#[test]
fn the_tooltip_gives_every_window_with_its_reset_as_a_clock_time() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(reply(
        7.0,
        Some(now() + hours(3)),
        23.0,
        // 2026-10-15 is a Thursday: Monday 14:00.
        "2026-10-19T14:00:00Z".parse().unwrap(),
        &[("Fable", 4.0)],
    ));
    env.push_usage_reply(reply(0.0, None, 12.0, now() + hours(30), &[]));

    let report = core.refresh_usage().unwrap();
    let view = widget(&core, &report);

    assert_eq!(
        view.tooltip,
        "A · Session 7%, resets at 15:00 · Weekly 23%, resets Mon 14:00 · Fable 4%\n\
         B · Session idle · Weekly 12%, resets Fri 18:00"
    );

    let italian = core
        .taskbar_widget(&report, Language::Italian, &TimeZone::UTC)
        .unwrap();
    assert_eq!(
        italian.tooltip,
        "A · Sessione 7%, reset alle 15:00 · Settimanale 23%, reset lun 14:00 · Fable 4%\n\
         B · Sessione inattiva · Settimanale 12%, reset ven 18:00"
    );
    // The widget itself writes the same in both languages.
    assert_eq!(italian.row_labels, view.row_labels);
    assert_eq!(italian.columns, view.columns);
}

#[test]
fn reset_times_are_shown_in_the_given_time_zone() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(7.0, 23.0));

    let rome = TimeZone::get("Europe/Rome").unwrap();
    let view = core
        .taskbar_widget(&core.refresh_usage().unwrap(), Language::English, &rome)
        .unwrap();

    assert!(view.tooltip.contains("resets at 17:00"), "{}", view.tooltip);
}

// --- Colors --------------------------------------------------------------------------------

#[test]
fn each_column_has_its_account_color_for_both_taskbars() {
    let (_home, core, env) = home_with_a_and_b();
    core.update_account(
        &AccountUuid::new(UUID_B),
        AccountUpdate {
            label: None,
            color: Some("#facc15".parse().unwrap()),
            launcher: None,
        },
    )
    .unwrap();
    env.push_usage_reply(ok(1.0, 1.0));
    env.push_usage_reply(ok(1.0, 1.0));

    let view = widget(&core, &core.refresh_usage().unwrap());

    let b = &view.columns[1];
    assert_eq!(b.ink(TaskbarTheme::Dark).bar.as_str(), "#facc15");
    assert_eq!(b.ink(TaskbarTheme::Dark).text.as_str(), "#facc15");
    // A light yellow is darkened on the light taskbar.
    assert_ne!(b.ink(TaskbarTheme::Light).bar.as_str(), "#facc15");
    assert_ne!(b.ink(TaskbarTheme::Light).text.as_str(), "#facc15");
    // The taskbar's own colors follow its theme.
    assert_ne!(
        TaskbarTheme::Light.palette().text,
        TaskbarTheme::Dark.palette().text
    );
}

// --- Preferences -----------------------------------------------------------------------------

#[test]
fn both_preferences_are_saved_in_config_toml() {
    let (home, core, env) = home_with_a();
    assert!(core.taskbar_widget_on().unwrap());
    assert!(!core.taskbar_widget_labels().unwrap());

    core.set_taskbar_widget_on(false).unwrap();
    core.set_taskbar_widget_labels(true).unwrap();

    let text = home.config_text().unwrap();
    assert!(text.contains("taskbar_widget = false"), "{text}");
    assert!(text.contains("taskbar_widget_labels = true"), "{text}");
    assert!(!core.taskbar_widget_on().unwrap());
    assert!(core.taskbar_widget_labels().unwrap());
    env.push_usage_reply(ok(1.0, 1.0));
    let view = widget(&core, &core.refresh_usage().unwrap());
    assert!(!view.enabled && !view.visible());
    assert!(view.account_labels);
}

#[test]
fn preferences_written_by_hand_are_read_and_their_comments_kept() {
    let (home, core, _env) = home_with_a();
    let text = home.config_text().unwrap();
    home.write_config(&format!(
        "{text}\n[preferences]\ntaskbar_widget = false # not on this laptop\n"
    ));

    assert!(!core.taskbar_widget_on().unwrap());
    core.set_taskbar_widget_on(true).unwrap();

    let text = home.config_text().unwrap();
    assert!(
        text.contains("taskbar_widget = true # not on this laptop"),
        "{text}"
    );
}

// --- Tokens --------------------------------------------------------------------------------

#[test]
fn no_token_reaches_the_widget() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(ok(35.0, 14.0));
    env.push_usage_reply(rate_limited());

    let view = widget(&core, &core.refresh_usage().unwrap());

    let everything = format!("{view:?}\n{}", serde_json::to_string(&view).unwrap());
    for token in [TOKEN_A, TOKEN_B, common::FIXTURE_REFRESH] {
        assert!(!everything.contains(token));
    }
    assert!(!everything.contains("sk-ant-"));
}
