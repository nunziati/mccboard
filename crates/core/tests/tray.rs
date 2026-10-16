//! The tray view model (spec stories 60–64; ticket #8): the icon's worst account and
//! percentage, the tooltip, the status lines of the menu, "no instances" and "token expired".
//!
//! Every test runs on a temporary home with the fake environment port, and gets its usage
//! report the way the app does: `refresh_usage` or `usage`, then `tray_view`.

mod common;

use common::{TempHome, UUID_A, UUID_B, now};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use mccboard_core::Environment as _;
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    AccountUuid, Core, EndpointReply, Error, Language, RenewalExit, TrayView, UsageReport,
};
use serde_json::json;

const TOKEN_A: &str = "sk-ant-oat01-FIXTURE-A-must-never-leak-aaaaaaaaaaaa";
const TOKEN_B: &str = "sk-ant-oat01-FIXTURE-B-must-never-leak-bbbbbbbbbbbb";

/// The Windows tray tooltip keeps at most 127 UTF-16 units.
const TOOLTIP_LIMIT: usize = 127;

/// Resets 3 hours and about 4 days after [`now`].
const SESSION_RESET: &str = "2026-10-15T15:00:00Z";
const WEEKLY_RESET: &str = "2026-10-19T14:00:00Z";

fn secs(n: i64) -> SignedDuration {
    SignedDuration::from_secs(n)
}

fn hours(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n)
}

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn ok(session: f64, weekly: f64, models: &[(&str, f64)]) -> EndpointReply {
    let mut limits = vec![
        json!({"kind": "session", "percent": session, "resets_at": SESSION_RESET}),
        json!({"kind": "weekly_all", "percent": weekly, "resets_at": WEEKLY_RESET}),
    ];
    for (model, percent) in models {
        limits.push(json!({"kind": "weekly_scoped", "percent": percent,
                           "resets_at": WEEKLY_RESET,
                           "scope": {"model": {"display_name": model}}}));
    }
    EndpointReply::Http {
        status: 200,
        retry_after: None,
        body: json!({ "limits": limits }).to_string(),
    }
}

fn rate_limited() -> EndpointReply {
    EndpointReply::Http {
        status: 429,
        retry_after: None,
        body: String::new(),
    }
}

fn english(core: &Core, report: &UsageReport) -> TrayView {
    core.tray_view(report, Language::English, &TimeZone::UTC)
        .unwrap()
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// A home with A (primary instance) and B (`~/.claude-b`), both registered and logged in with
/// valid tokens. Labels "A" and "B" are suggested from the folders.
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

fn color_of(core: &Core, uuid: &str) -> String {
    core.registry()
        .unwrap()
        .accounts
        .into_iter()
        .find(|a| a.uuid.as_str() == uuid)
        .unwrap()
        .color
        .to_string()
}

// --- The icon: the account closest to a limit ---------------------------------------------

#[test]
fn the_icon_shows_the_account_whose_session_or_weekly_window_is_highest_in_its_color() {
    let (_home, core, env) = home_with_a_and_b();
    // Accounts are called in label order: A, then B.
    env.push_usage_reply(ok(35.0, 14.0, &[]));
    env.push_usage_reply(ok(10.0, 80.4, &[]));

    let view = english(&core, &core.refresh_usage().unwrap());

    assert_eq!(view.icon.percent, Some(80));
    assert_eq!(view.icon.account, Some(AccountUuid::new(UUID_B)));
    assert_eq!(view.icon.color.to_string(), color_of(&core, UUID_B));
    assert!(!view.icon.stale);
    assert!(!view.no_instances);
    let worst: Vec<_> = view.accounts.iter().filter(|a| a.worst).collect();
    assert_eq!(worst.len(), 1);
    assert_eq!(worst[0].account.uuid.as_str(), UUID_B);
}

#[test]
fn model_windows_do_not_drive_the_icon() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(ok(20.0, 30.0, &[("Fable", 95.0)]));
    env.push_usage_reply(ok(40.0, 10.0, &[]));

    let view = english(&core, &core.refresh_usage().unwrap());

    assert_eq!(view.icon.percent, Some(40));
    assert_eq!(view.icon.account, Some(AccountUuid::new(UUID_B)));
}

#[test]
fn a_tie_goes_to_the_first_account_in_label_order() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(ok(50.0, 10.0, &[]));
    env.push_usage_reply(ok(10.0, 50.0, &[]));

    let view = english(&core, &core.refresh_usage().unwrap());

    assert_eq!(view.icon.percent, Some(50));
    assert_eq!(view.icon.account, Some(AccountUuid::new(UUID_A)));
    // Stable: the same readings pick the same account again.
    let again = english(&core, &core.usage().unwrap());
    assert_eq!(again.icon.account, Some(AccountUuid::new(UUID_A)));
}

#[test]
fn an_account_without_a_reading_never_wins_the_icon() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(rate_limited());
    env.push_usage_reply(ok(5.0, 7.0, &[]));

    let view = english(&core, &core.refresh_usage().unwrap());

    assert_eq!(view.icon.percent, Some(7));
    assert_eq!(view.icon.account, Some(AccountUuid::new(UUID_B)));
}

#[test]
fn with_no_reading_at_all_the_icon_has_no_percentage() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(rate_limited());
    env.push_usage_reply(rate_limited());

    let view = english(&core, &core.refresh_usage().unwrap());

    assert_eq!(view.icon.percent, None);
    assert_eq!(view.icon.account, None);
    assert!(view.accounts.iter().all(|a| !a.worst));
    assert!(view.tooltip.contains('A') && view.tooltip.contains('B'));
}

// --- Stale, expired, reset ----------------------------------------------------------------

#[test]
fn a_stale_worst_reading_marks_the_icon_tooltip_and_menu() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(35.0, 14.0, &[]));
    core.refresh_usage().unwrap();
    env.advance(secs(61));
    env.push_usage_reply(rate_limited());

    let view = english(&core, &core.refresh_usage().unwrap());

    assert_eq!(view.icon.percent, Some(35));
    assert!(view.icon.stale);
    assert!(view.accounts[0].stale);
    assert!(
        view.tooltip.contains("Stale · read at 12:00"),
        "{}",
        view.tooltip
    );
    assert!(
        view.accounts[0].title.contains("stale"),
        "{:?}",
        view.accounts[0]
    );
    // When the last values were read, so the user can judge them.
    assert!(
        view.accounts[0].lines.iter().any(|l| l.contains("12:00")),
        "{:?}",
        view.accounts[0].lines
    );
}

#[test]
fn an_expired_token_shows_token_expired_with_the_last_values() {
    let (home, core, env) = home_with_a_and_b();
    env.push_usage_reply(ok(35.0, 14.0, &[]));
    env.push_usage_reply(ok(60.0, 20.0, &[]));
    core.refresh_usage().unwrap();
    // B's token expires; Claude Code hasn't renewed it yet.
    home.linked(".claude-b")
        .credentials_with_token(TOKEN_B, now() + secs(30));
    env.advance(secs(61));
    env.push_usage_reply(ok(36.0, 14.0, &[]));

    let view = english(&core, &core.refresh_usage().unwrap());

    let b = &view.accounts[1];
    assert!(b.token_expired);
    assert!(b.stale);
    assert_eq!(b.percent, Some(60));
    assert!(b.title.contains("token expired"), "{}", b.title);
    assert!(view.tooltip.contains("token expired"), "{}", view.tooltip);
    // Its last known reading still counts for the icon, marked stale.
    assert_eq!(view.icon.account, Some(AccountUuid::new(UUID_B)));
    assert!(view.icon.stale);
    assert!(view.icon.token_expired);
}

#[test]
fn an_expired_token_with_no_reading_says_so() {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A, now() - secs(1));
    let (core, _env) = home.core();
    core.register(&primary.dir).unwrap();

    let view = english(&core, &core.refresh_usage().unwrap());

    assert_eq!(view.icon.percent, None);
    assert!(view.icon.token_expired);
    assert!(view.accounts[0].token_expired);
    assert!(view.tooltip.contains("Token expired"), "{}", view.tooltip);
    assert_eq!(view.accounts[0].title, "A · token expired");

    let italian = core
        .tray_view(&core.usage().unwrap(), Language::Italian, &TimeZone::UTC)
        .unwrap();
    assert_eq!(italian.accounts[0].title, "A · token scaduto");
}

#[test]
fn a_window_whose_reset_passed_reads_as_zero_even_in_an_older_report() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(90.0, 20.0, &[]));
    let report = core.refresh_usage().unwrap();
    assert_eq!(english(&core, &report).icon.percent, Some(90));

    // The session window resets at 15:00; the tray redraws from the same report afterwards.
    env.set_now(ts(SESSION_RESET) + secs(60));
    let view = english(&core, &report);

    assert_eq!(view.icon.percent, Some(20));
    let session = &view.accounts[0].windows[0];
    assert_eq!(session.percent, 0);
    assert_eq!(session.resets_at, None);
}

// --- No instances, single account ----------------------------------------------------------

#[test]
fn with_no_registered_instance_the_tray_says_so_and_offers_instances() {
    let home = TempHome::new();
    let (core, _env) = home.core();

    let view = english(&core, &core.usage().unwrap());

    assert!(view.no_instances);
    assert_eq!(view.icon.percent, None);
    assert!(view.accounts.is_empty());
    assert!(
        view.tooltip.contains("No instance registered"),
        "{}",
        view.tooltip
    );
    assert!(
        view.notice
            .as_deref()
            .unwrap()
            .contains("No instance registered"),
        "{:?}",
        view.notice
    );
    assert!(!view.menu.open_instances.is_empty());
}

#[test]
fn accounts_left_without_instances_still_mean_no_instances() {
    // The primary instance is never removed, so the account is left by a linked one.
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A, now() + hours(5));
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();
    core.unregister(&b.dir).unwrap();
    assert_eq!(core.registry().unwrap().accounts.len(), 1);

    let view = english(&core, &core.usage().unwrap());

    assert!(view.no_instances);
}

#[test]
fn a_single_account_is_a_plain_usage_indicator() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(35.0, 14.0, &[("Fable", 58.0)]));

    let view = english(&core, &core.refresh_usage().unwrap());

    assert_eq!(view.icon.percent, Some(35));
    assert_eq!(view.icon.color.to_string(), color_of(&core, UUID_A));
    // No account label: one line per window with its reset.
    assert_eq!(
        view.tooltip,
        "Session 35% · resets in 3h\nWeekly 14% · resets in 4d 2h\nFable 58% · resets in 4d 2h"
    );
}

// --- Tooltip and menu lines ----------------------------------------------------------------

#[test]
fn the_tooltip_lists_every_account_with_its_windows_and_resets() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(ok(35.0, 14.0, &[]));
    env.push_usage_reply(ok(10.0, 80.0, &[]));

    let view = english(&core, &core.refresh_usage().unwrap());

    assert_eq!(
        view.tooltip,
        "A: 5h 35% (3h) · 7d 14% (4d 2h)\nB: 5h 10% (3h) · 7d 80% (4d 2h)"
    );
}

#[test]
fn the_tooltip_always_fits_windows_and_still_names_every_account() {
    let home = TempHome::new();
    let (core, env) = home.core();
    let models = [("Fable", 58.0), ("Haiku", 12.0), ("Sonnet", 33.0)];
    let mut labels = Vec::new();
    for n in 0..6 {
        let instance = home.linked(&format!(".claude-acct{n}"));
        let uuid = format!("{n}{n}{n}{n}{n}{n}{n}{n}-0000-4000-8000-000000000000");
        let token = format!("sk-ant-oat01-FIXTURE-{n}-must-never-leak");
        instance
            .logged_in_as(&uuid, "x@example.com")
            .credentials_with_token(&token, now() + hours(5));
        core.register(&instance.dir).unwrap();
        labels.push(format!("acct{n}"));
        env.push_usage_reply(ok(10.0 + n as f64, 20.0, &models));
    }

    let view = english(&core, &core.refresh_usage().unwrap());

    assert!(
        utf16_len(&view.tooltip) <= TOOLTIP_LIMIT,
        "{} units: {}",
        utf16_len(&view.tooltip),
        view.tooltip
    );
    for label in &labels {
        assert!(
            view.tooltip.contains(label.as_str()),
            "{label}: {}",
            view.tooltip
        );
    }
    // The full detail is in the menu: every window of every account.
    assert_eq!(view.accounts.len(), 6);
    for account in &view.accounts {
        assert_eq!(account.lines.iter().filter(|l| l.contains('%')).count(), 5);
        assert!(account.lines.iter().any(|l| l.contains("Sonnet")));
    }
}

#[test]
fn menu_lines_give_every_window_with_its_reset_time() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(ok(35.0, 14.0, &[("Fable", 58.0)]));
    env.push_usage_reply(ok(10.0, 80.0, &[]));

    let view = english(&core, &core.refresh_usage().unwrap());

    let a = &view.accounts[0];
    assert_eq!(a.title, "A · 35%");
    assert_eq!(
        a.lines,
        [
            "Session: 35%, resets in 3h (15:00)",
            "Weekly: 14%, resets in 4d 2h (Mon 14:00)",
            "Fable (weekly): 58%, resets in 4d 2h (Mon 14:00)",
        ]
    );
    assert_eq!(view.accounts[1].title, "B · 80%");
    assert_eq!(view.notice, None);
}

#[test]
fn reset_times_are_shown_in_the_given_time_zone() {
    let (_home, core, env) = home_with_a();
    env.push_usage_reply(ok(35.0, 14.0, &[]));
    let report = core.refresh_usage().unwrap();

    let rome = TimeZone::get("Europe/Rome").unwrap();
    let view = core.tray_view(&report, Language::English, &rome).unwrap();

    assert_eq!(
        view.accounts[0].lines[0],
        "Session: 35%, resets in 3h (17:00)"
    );
}

#[test]
fn accounts_without_readings_say_why() {
    let (home, core, _env) = home_with_a_and_b();
    // B is logged out.
    home.linked(".claude-b").raw_credentials("{}");

    let view = english(&core, &core.usage().unwrap());

    assert!(
        view.accounts[0]
            .title
            .contains("waiting for a first reading"),
        "{:?}",
        view.accounts[0]
    );
    assert!(
        view.accounts[1].title.contains("not logged in"),
        "{:?}",
        view.accounts[1]
    );
    assert!(
        view.tooltip.contains("B: not logged in"),
        "{}",
        view.tooltip
    );
}

// --- Language ----------------------------------------------------------------------------

#[test]
fn tray_strings_come_in_italian() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(ok(35.0, 14.0, &[]));
    env.push_usage_reply(ok(10.0, 80.0, &[]));
    let report = core.refresh_usage().unwrap();

    let view = core
        .tray_view(&report, Language::Italian, &TimeZone::UTC)
        .unwrap();

    assert_eq!(view.language, Language::Italian);
    assert_eq!(view.menu.quit, "Esci");
    assert_eq!(view.menu.refresh, "Aggiorna l'utilizzo");
    assert_eq!(
        view.accounts[0].lines[1],
        "Settimanale: 14%, si azzera tra 4g 2h (lun 14:00)"
    );
    assert_eq!(
        view.tooltip,
        "A: 5h 35% (3h) · 7g 14% (4g 2h)\nB: 5h 10% (3h) · 7g 80% (4g 2h)"
    );
    let english_view = english(&core, &report);
    assert_ne!(english_view.menu.quit, view.menu.quit);
}

#[test]
fn the_menu_offers_both_languages_with_the_current_one_marked() {
    let (_home, core, _env) = home_with_a();
    let view = core
        .tray_view(&core.usage().unwrap(), Language::Italian, &TimeZone::UTC)
        .unwrap();

    let names: Vec<_> = view
        .menu
        .languages
        .iter()
        .map(|choice| (choice.language, choice.name.as_str(), choice.selected))
        .collect();
    assert_eq!(
        names,
        [
            (Language::English, "English", false),
            (Language::Italian, "Italiano", true)
        ]
    );
}

// --- Errors and token safety ---------------------------------------------------------------

#[test]
fn a_corrupt_config_gives_an_error_view() {
    let home = TempHome::new();
    home.write_config("accounts = 3");
    let (core, _env) = home.core();

    let error = core.usage().unwrap_err();
    let view = TrayView::unavailable(&error, Language::English);

    assert!(matches!(error, Error::ConfigCorrupt { .. }));
    assert_eq!(view.icon.percent, None);
    assert!(view.tooltip.contains("config.toml"), "{}", view.tooltip);
    assert!(utf16_len(&view.tooltip) <= TOOLTIP_LIMIT);
}

#[test]
fn no_token_reaches_the_tray() {
    let (_home, core, env) = home_with_a_and_b();
    env.push_usage_reply(ok(35.0, 14.0, &[]));
    env.push_usage_reply(ok(10.0, 80.0, &[]));

    let view = english(&core, &core.refresh_usage().unwrap());

    let text = format!("{view:?}");
    assert!(!text.contains(TOKEN_A) && !text.contains(TOKEN_B));
    assert!(!text.contains(common::FIXTURE_REFRESH));
}

// --- Claude Code renewing an expired token (#30) ------------------------------------------

/// A home with A in the primary instance, its token expired a minute ago, registered.
fn home_with_expired_a() -> (TempHome, Core, FakeEnvironment) {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_with_token(TOKEN_A, now() - secs(60));
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();
    (home, core, env)
}

/// Makes the fake's renewal play Claude Code renewing the token for 8 hours.
fn renews(env: &FakeEnvironment) {
    let clock = env.clone();
    env.on_renewal(move |renewal| {
        let expires_at = clock.now() + hours(8);
        std::fs::write(
            renewal.config_dir.join(".credentials.json"),
            json!({"claudeAiOauth": {"accessToken": TOKEN_A,
                                     "expiresAt": expires_at.as_millisecond()}})
            .to_string(),
        )?;
        Ok(RenewalExit { exit_code: Some(0) })
    });
}

fn claude_not_found(env: &FakeEnvironment) {
    env.on_renewal(|_| Err(std::io::ErrorKind::NotFound.into()));
}

#[test]
fn a_renewed_token_says_when_in_the_menu_and_the_tooltip() {
    let (_home, core, env) = home_with_expired_a();
    renews(&env);
    env.push_usage_reply(ok(35.0, 14.0, &[]));
    let report = core.refresh_usage().unwrap();

    let view = english(&core, &report);

    assert!(!view.icon.token_expired);
    let lines = &view.accounts[0].lines;
    assert!(
        lines.contains(&"Token renewed at 12:00".to_owned()),
        "{lines:?}"
    );
    assert!(
        view.tooltip.contains("Token renewed at 12:00"),
        "{}",
        view.tooltip
    );

    let italian = core
        .tray_view(&report, Language::Italian, &TimeZone::UTC)
        .unwrap();
    let lines = &italian.accounts[0].lines;
    assert!(
        lines.contains(&"Token rinnovato alle 12:00".to_owned()),
        "{lines:?}"
    );
}

#[test]
fn a_failed_renewal_says_why_and_when_it_tries_again() {
    let (_home, core, env) = home_with_expired_a();
    claude_not_found(&env);
    let report = core.refresh_usage().unwrap();

    let view = english(&core, &report);

    assert!(view.icon.token_expired);
    assert_eq!(view.accounts[0].title, "A · token expired");
    let lines = &view.accounts[0].lines;
    assert!(
        lines.contains(&"Renewal failed (Claude Code not found), next try at 12:30".to_owned()),
        "{lines:?}"
    );
    assert!(
        view.tooltip.contains("Renewal failed, retry 12:30"),
        "{}",
        view.tooltip
    );

    let italian = core
        .tray_view(&report, Language::Italian, &TimeZone::UTC)
        .unwrap();
    let lines = &italian.accounts[0].lines;
    assert!(
        lines.contains(
            &"Rinnovo non riuscito (Claude Code non trovato), nuovo tentativo alle 12:30"
                .to_owned()
        ),
        "{lines:?}"
    );
    assert!(
        italian.tooltip.contains("Rinnovo fallito, riprovo 12:30"),
        "{}",
        italian.tooltip
    );
}

#[test]
fn every_renewal_failure_has_a_reason_in_both_languages() {
    type Setup = fn(&FakeEnvironment);
    let failures: [(Setup, &str, &str); 5] = [
        (
            |env| env.on_renewal(|_| Ok(RenewalExit { exit_code: Some(0) })),
            "token still expired",
            "token ancora scaduto",
        ),
        (
            |env| env.on_renewal(|_| Ok(RenewalExit { exit_code: Some(3) })),
            "Claude Code exited with code 3",
            "Claude Code è uscito con codice 3",
        ),
        (
            |env| env.on_renewal(|_| Ok(RenewalExit { exit_code: None })),
            "Claude Code was stopped",
            "Claude Code è stato interrotto",
        ),
        (
            |env| env.on_renewal(|_| Err(std::io::ErrorKind::TimedOut.into())),
            "Claude Code took too long",
            "Claude Code ci ha messo troppo",
        ),
        (
            |env| env.on_renewal(|_| Err(std::io::ErrorKind::PermissionDenied.into())),
            "Claude Code couldn't start",
            "Claude Code non è partito",
        ),
    ];
    for (setup, english_reason, italian_reason) in failures {
        let (_home, core, env) = home_with_expired_a();
        setup(&env);
        let report = core.refresh_usage().unwrap();
        let lines = english(&core, &report).accounts[0].lines.join("\n");
        assert!(lines.contains(english_reason), "{lines}");
        let lines = core
            .tray_view(&report, Language::Italian, &TimeZone::UTC)
            .unwrap()
            .accounts[0]
            .lines
            .join("\n");
        assert!(lines.contains(italian_reason), "{lines}");
    }
}

#[test]
fn among_several_accounts_the_renewal_note_is_short_and_dropped_first() {
    let (home, core, env) = home_with_a_and_b();
    home.linked(".claude-b")
        .credentials_with_token(TOKEN_B, now() - secs(60));
    claude_not_found(&env);
    env.push_usage_reply(ok(35.0, 14.0, &[]));

    let view = english(&core, &core.refresh_usage().unwrap());

    assert!(
        view.tooltip
            .contains("B: token expired · renewal failed, retry 12:30"),
        "{}",
        view.tooltip
    );

    // With many model windows, the note goes before anything else.
    env.advance(SignedDuration::from_mins(2));
    env.push_usage_reply(ok(
        35.0,
        14.0,
        &[("Fable", 50.0), ("Sonnet", 20.0), ("Haiku", 10.0)],
    ));
    let view = english(&core, &core.refresh_usage().unwrap());
    assert!(utf16_len(&view.tooltip) <= TOOLTIP_LIMIT);
    assert!(!view.tooltip.contains("retry"), "{}", view.tooltip);
    assert!(
        view.tooltip.contains("B: token expired"),
        "{}",
        view.tooltip
    );
}

#[test]
fn with_renewal_off_the_tray_says_nothing_about_it() {
    let (_home, core, env) = home_with_expired_a();
    core.set_renew_expired_tokens(false).unwrap();
    claude_not_found(&env);

    let view = english(&core, &core.refresh_usage().unwrap());

    assert!(env.renewal_invocations().is_empty());
    assert_eq!(view.tooltip, "Token expired");
    assert!(
        view.accounts[0].lines.is_empty(),
        "{:?}",
        view.accounts[0].lines
    );
}

#[test]
fn the_settings_menu_offers_the_renewal_checked_as_saved() {
    let (_home, core, _env) = home_with_a();

    let view = english(&core, &core.usage().unwrap());
    assert_eq!(
        view.menu.renew_tokens,
        "Let Claude Code renew an expired token"
    );
    assert!(view.menu.renew_tokens_on);

    core.set_renew_expired_tokens(false).unwrap();
    let view = core
        .tray_view(&core.usage().unwrap(), Language::Italian, &TimeZone::UTC)
        .unwrap();
    assert_eq!(
        view.menu.renew_tokens,
        "Lascia che Claude Code rinnovi un token scaduto"
    );
    assert!(!view.menu.renew_tokens_on);
}
