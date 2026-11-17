//! The dashboard on temporary homes through the fake environment port: the state driven by
//! keys and messages, frames drawn on ratatui's `TestBackend` in both languages and never
//! with a token, and the whole screen in the TUI's loop within the usage rate limits.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::KeyCode;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{Core, EndpointReply, Environment, Language};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;
use tempfile::TempDir;

use super::state::{Dashboard, Message, Notice, Request};
use super::{Runner, view};
use crate::tui::app::testing::{Script, Step, key};
use crate::tui::app::{self};
use crate::tui::terminal::testing::{Recorded, serial};
use crate::tui::terminal::{TerminalGuard, Tui};

/// Token-shaped fixture values: no frame may show them.
const TOKEN_A: &str = "sk-ant-oat01-FIXTURE-A-must-never-leak-aaaaaaaaaaaa";
const TOKEN_B: &str = "sk-ant-oat01-FIXTURE-B-must-never-leak-bbbbbbbbbbbb";
const UUID_A: &str = "11111111-1111-4111-8111-111111111111";
const UUID_B: &str = "22222222-2222-4222-8222-222222222222";

/// A Thursday, noon UTC.
fn now() -> Timestamp {
    "2026-10-15T12:00:00Z".parse().unwrap()
}

fn hours(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n)
}

fn days(n: i64) -> SignedDuration {
    hours(24 * n)
}

/// A home with A in the primary instance and B in `~/.claude-b`, both logged in with valid
/// tokens and registered. The primary instance has `agents/`, which B doesn't link yet: one
/// drift item.
struct Home {
    dir: TempDir,
    core: Core,
    env: FakeEnvironment,
}

impl Home {
    fn new() -> Home {
        let dir = tempfile::tempdir().unwrap();
        let env = FakeEnvironment::new(now());
        let core = Core::new(dir.path(), env.clone());
        let home = Home { dir, core, env };
        let primary = home.path().join(".claude");
        fs::create_dir_all(primary.join("agents")).unwrap();
        log_in(
            &home.path().join(".claude.json"),
            &primary,
            (UUID_A, "a"),
            TOKEN_A,
        );
        let b = home.b();
        fs::create_dir_all(&b).unwrap();
        log_in(&b.join(".claude.json"), &b, (UUID_B, "b"), TOKEN_B);
        home.core.register(&primary).unwrap();
        home.core.register(&b).unwrap();
        home
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn b(&self) -> PathBuf {
        self.path().join(".claude-b")
    }

    fn config(&self) -> String {
        fs::read_to_string(self.path().join(".mccboard").join("config.toml")).unwrap()
    }

    fn dashboard(&self, language: Language) -> Dashboard {
        Dashboard::new(self.core.clone(), TimeZone::UTC, language)
    }

    /// Readings for A and B, as a refresh made before the dashboard started left them.
    fn read_usage(&self) {
        // Accounts are called in label order: A, then B.
        self.env
            .push_usage_reply(reply(35.0, 14.0, &[("Fable", 58.0)]));
        self.env.push_usage_reply(reply(10.0, 72.0, &[]));
        self.core.refresh_usage().unwrap();
    }
}

/// Writes what Claude Code's login leaves: the account in `global_config`, credentials with
/// `token` valid for five hours in `config_dir`.
fn log_in(global_config: &Path, config_dir: &Path, (uuid, name): (&str, &str), token: &str) {
    let email = format!("{name}@example.com");
    let account = json!({"oauthAccount": {"accountUuid": uuid, "emailAddress": email}});
    fs::write(global_config, account.to_string()).unwrap();
    let expires = (now() + hours(5)).as_millisecond();
    let credentials = json!({"claudeAiOauth": {
        "accessToken": token, "refreshToken": token, "expiresAt": expires,
        "scopes": ["user:inference", "user:profile"],
    }});
    fs::write(
        config_dir.join(".credentials.json"),
        credentials.to_string(),
    )
    .unwrap();
}

/// A reading whose session window resets in 3 hours and weekly and model windows in 4 days.
fn reply(session: f64, weekly: f64, models: &[(&str, f64)]) -> EndpointReply {
    let mut limits = vec![
        json!({"kind": "session", "percent": session,
               "resets_at": (now() + hours(3)).to_string()}),
        json!({"kind": "weekly_all", "percent": weekly,
               "resets_at": (now() + days(4)).to_string()}),
    ];
    for (model, percent) in models {
        limits.push(json!({"kind": "weekly_scoped", "percent": percent,
                           "resets_at": (now() + days(4)).to_string(),
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

/// What a terminal of 100 by 34 shows of the dashboard, line by line. No frame ever shows a
/// token.
fn screen(dashboard: &Dashboard) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 34)).unwrap();
    terminal.draw(|frame| view::draw(dashboard, frame)).unwrap();
    let text = buffer_text(&terminal);
    assert_no_token(&text);
    text
}

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .map(|line| line.trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_no_token(text: &str) {
    for secret in ["FIXTURE", "sk-ant-"] {
        assert!(!text.contains(secret), "a token on screen:\n{text}");
    }
}

fn press(dashboard: &mut Dashboard, code: KeyCode) -> Option<Request> {
    dashboard.key(key(code))
}

/// Runs a refresh as the dashboard's thread would, and hands its outcome to the dashboard.
fn refresh(home: &Home, dashboard: &mut Dashboard) {
    dashboard.message(Message::RefreshStarted);
    dashboard.message(Message::Usage(home.core.refresh_usage()));
}

/// `text` with home-relative paths as this OS shows them: `~\.claude-b` on Windows.
fn os(text: &str) -> String {
    text.replace("~/", &format!("~{}", std::path::MAIN_SEPARATOR))
}

/// The line of the screen that contains `needle`.
fn line_with<'a>(screen: &'a str, needle: &str) -> &'a str {
    screen
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line with {needle:?} in:\n{screen}"))
}

// --- What it shows -----------------------------------------------------------------------------

#[test]
fn each_account_shows_its_windows_as_bars_with_percentage_and_time_to_reset() {
    let home = Home::new();
    home.read_usage();

    let dashboard = home.dashboard(Language::English);
    let screen = screen(&dashboard);

    for line in [
        "■ A · 35%  a@example.com",
        "  Session  ████░░░░░░   35%      3h  0m",
        "  Weekly   █░░░░░░░░░   14%  4d  0h  0m",
        "  Fable    ██████░░░░   58%  4d  0h  0m",
        "■ B · 72%  b@example.com",
        "  Session  █░░░░░░░░░   10%      3h  0m",
        "  Weekly   ███████░░░   72%  4d  0h  0m",
    ] {
        assert!(screen.contains(line), "{line:?} missing in:\n{screen}");
    }
}

#[test]
fn each_instance_shows_its_folder_kind_account_login_drift_and_launcher() {
    let home = Home::new();

    let screen = screen(&home.dashboard(Language::English));

    let columns = line_with(&screen, "Folder");
    for heading in ["Type", "Account", "Login", "Drift", "Launcher"] {
        assert!(columns.contains(heading), "{screen}");
    }
    let words = |line: &str| {
        line.split("  ")
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        words(line_with(&screen, &os("~/.claude "))),
        [
            "│› ~/.claude",
            "primary",
            "A",
            "logged in",
            "no drift",
            "claude",
            "│"
        ]
        .map(os)
    );
    assert_eq!(
        words(line_with(&screen, &os("~/.claude-b"))),
        [
            "│",
            "~/.claude-b",
            "linked",
            "B",
            "logged in",
            "1 item to fix",
            "claude-b",
            "│"
        ]
        .map(os)
    );
}

#[test]
fn the_dashboard_speaks_italian_when_asked() {
    let home = Home::new();
    home.read_usage();

    let screen = screen(&home.dashboard(Language::Italian));

    for text in [
        " Account ",
        "Sessione     ████░░░░░░   35%      3h  0m",
        "Settimanale  █░░░░░░░░░   14%  4d  0h  0m",
        " Istanze ",
        "Cartella",
        "Tipo",
        "Differenze",
        "Comando di avvio",
        "primaria",
        "collegata",
        "login effettuato",
        "nessuna differenza",
        "1 elemento da sistemare",
        "r aggiorna  p ripara  s configura  l lingua  q esci  ↑↓ seleziona",
    ] {
        assert!(screen.contains(text), "{text:?} missing in:\n{screen}");
    }
    for english in ["Session ", "Weekly", "Instances", "logged in", "refresh"] {
        assert!(!screen.contains(english), "{english:?} in:\n{screen}");
    }
}

/// Makes B's token expired a second ago.
fn expire_b(home: &Home) {
    let expired = json!({"claudeAiOauth": {
        "accessToken": TOKEN_B, "refreshToken": TOKEN_B,
        "expiresAt": (home.env.now() - SignedDuration::from_secs(1)).as_millisecond(),
    }});
    fs::write(home.b().join(".credentials.json"), expired.to_string()).unwrap();
}

#[test]
fn a_stale_reading_and_an_expired_token_are_marked_as_the_tray_marks_them() {
    let home = Home::new();
    home.read_usage();
    home.env.advance(SignedDuration::from_mins(11));
    // B's token expires, and Claude Code, asked to renew it, leaves it expired. A's call fails.
    expire_b(&home);
    home.env.push_usage_reply(rate_limited());

    let mut dashboard = home.dashboard(Language::English);
    refresh(&home, &mut dashboard);
    let text = screen(&dashboard);

    assert!(text.contains("■ A · 35% (stale)"), "{text}");
    assert!(text.contains("Last reading: 12:00"), "{text}");
    assert!(text.contains("■ B · 72% (token expired)"), "{text}");
    assert!(
        text.contains("Renewal failed (token still expired)"),
        "{text}"
    );
    assert!(
        line_with(&text, &os("~/.claude-b")).contains("token expired"),
        "{text}"
    );

    // The stale columns are drawn dim, as the widget fades them.
    let mut terminal = Terminal::new(TestBackend::new(100, 34)).unwrap();
    terminal
        .draw(|frame| view::draw(&dashboard, frame))
        .unwrap();
    let row = text
        .lines()
        .position(|line| line.contains("Session  ████"))
        .unwrap();
    let cell = &terminal.backend().buffer()[(12, row as u16)];
    assert_eq!(cell.symbol(), "█");
    assert!(cell.modifier.contains(ratatui::style::Modifier::DIM));

    let italian = screen(&home.dashboard(Language::Italian));
    assert!(italian.contains("■ A · 35% (non aggiornato)"), "{italian}");
    assert!(italian.contains("Ultima lettura: 12:00"), "{italian}");
    assert!(italian.contains("token scaduto"), "{italian}");
}

#[test]
fn an_expired_token_without_a_reading_shows_the_widget_dots() {
    let home = Home::new();
    expire_b(&home);

    let text = screen(&home.dashboard(Language::English));

    assert!(text.contains("■ B · token expired"), "{text}");
    let after = &text[text.find("■ B").unwrap()..];
    assert!(after.lines().nth(1).unwrap().contains("···"), "{text}");
    assert!(text.contains("■ A · waiting for a first reading"), "{text}");
}

#[test]
fn without_instances_or_with_a_broken_config_it_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::new(dir.path(), FakeEnvironment::new(now()));

    let empty = screen(&Dashboard::new(
        core.clone(),
        TimeZone::UTC,
        Language::English,
    ));
    assert!(
        empty.contains("No instance registered yet: run `mccboard scan`"),
        "{empty}"
    );

    let config = dir.path().join(".mccboard").join("config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, "this is not toml [").unwrap();
    let broken = screen(&Dashboard::new(core, TimeZone::UTC, Language::English));
    assert!(
        broken.contains("config.toml isn't valid: fix it by hand."),
        "{broken}"
    );
    assert!(broken.contains("Instances can't be read"), "{broken}");
}

// --- Keys ----------------------------------------------------------------------------------

#[test]
fn the_arrows_move_the_selection_within_the_instances() {
    let home = Home::new();
    let mut dashboard = home.dashboard(Language::English);
    assert_eq!(dashboard.selected, 0);
    assert!(line_with(&screen(&dashboard), &os("~/.claude ")).starts_with("│›"));

    assert_eq!(press(&mut dashboard, KeyCode::Down), None);
    assert_eq!(dashboard.selected, 1);
    assert!(line_with(&screen(&dashboard), &os("~/.claude-b")).starts_with("│›"));
    // The last instance stays selected.
    press(&mut dashboard, KeyCode::Down);
    assert_eq!(dashboard.selected, 1);
    press(&mut dashboard, KeyCode::Up);
    press(&mut dashboard, KeyCode::Up);
    assert_eq!(dashboard.selected, 0);
    press(&mut dashboard, KeyCode::Char('j'));
    assert_eq!(dashboard.selected_instance().unwrap().config_dir, home.b());
}

#[test]
fn p_asks_before_repairing_and_n_leaves_everything_as_it_was() {
    let home = Home::new();
    let mut dashboard = home.dashboard(Language::English);
    // The primary instance has no drift: nothing to ask.
    press(&mut dashboard, KeyCode::Char('p'));
    assert!(dashboard.confirm.is_none());
    assert!(screen(&dashboard).contains(&os("Nothing to repair in ~/.claude.")));
    press(&mut dashboard, KeyCode::Down);

    assert_eq!(press(&mut dashboard, KeyCode::Char('p')), None);
    let asking = screen(&dashboard);
    assert!(asking.contains(&os("Repair ~/.claude-b?")), "{asking}");
    assert!(asking.contains("It fixes 1 item:"), "{asking}");
    assert!(asking.contains("link:agents"), "{asking}");
    assert!(asking.contains("y/Enter repair  n/Esc cancel"), "{asking}");
    // While it asks, the other keys do nothing.
    assert_eq!(press(&mut dashboard, KeyCode::Char('q')), None);
    assert_eq!(press(&mut dashboard, KeyCode::Char('r')), None);

    assert_eq!(press(&mut dashboard, KeyCode::Char('n')), None);
    let after = screen(&dashboard);
    assert!(!after.contains(&os("Repair ~/.claude-b?")), "{after}");
    assert!(after.contains("Repair cancelled."), "{after}");
    assert!(!home.b().join("agents").exists());

    press(&mut dashboard, KeyCode::Char('l'));
    press(&mut dashboard, KeyCode::Char('p'));
    let italian = screen(&dashboard);
    assert!(italian.contains(&os("Riparare ~/.claude-b?")), "{italian}");
    assert!(italian.contains("Sistema 1 elemento:"), "{italian}");
    assert!(
        italian.contains("s/Invio ripara  n/Esc annulla"),
        "{italian}"
    );
    press(&mut dashboard, KeyCode::Esc);
    assert!(dashboard.confirm.is_none());
    assert!(screen(&dashboard).contains("Riparazione annullata."));
}

#[test]
fn s_opens_the_setup_unless_a_repair_runs() {
    let home = Home::new();
    let mut dashboard = home.dashboard(Language::English);
    assert!(screen(&dashboard).contains("s setup"));

    assert_eq!(
        press(&mut dashboard, KeyCode::Char('s')),
        Some(Request::Setup)
    );

    press(&mut dashboard, KeyCode::Down);
    press(&mut dashboard, KeyCode::Char('p'));
    press(&mut dashboard, KeyCode::Char('y'));
    // The setup would change the instance under the repair.
    assert_eq!(press(&mut dashboard, KeyCode::Char('s')), None);
}

#[test]
fn y_repairs_the_selected_instance_and_tells_how_it_went() {
    let home = Home::new();
    let mut dashboard = home.dashboard(Language::English);
    press(&mut dashboard, KeyCode::Down);
    press(&mut dashboard, KeyCode::Char('p'));

    let request = press(&mut dashboard, KeyCode::Char('y'));

    assert_eq!(request, Some(Request::Repair(home.b())));
    assert!(screen(&dashboard).contains(&os("Repairing ~/.claude-b…")));
    // Nothing more starts while it runs.
    assert_eq!(press(&mut dashboard, KeyCode::Char('p')), None);
    assert!(dashboard.confirm.is_none());

    // The runner repairs on its thread and hands the outcome over.
    let result = home.core.repair(&home.b(), None);
    dashboard.message(Message::Repaired {
        config_dir: home.b(),
        result,
    });

    let text = screen(&dashboard);
    assert!(
        text.contains(&os("Repaired 1 item in ~/.claude-b.")),
        "{text}"
    );
    assert!(
        line_with(&text, &os("~/.claude-b")).contains("no drift"),
        "{text}"
    );
    assert!(home.b().join("agents").exists());
    // Without drift there is nothing to ask.
    press(&mut dashboard, KeyCode::Char('p'));
    assert!(dashboard.confirm.is_none());
    assert!(screen(&dashboard).contains(&os("Nothing to repair in ~/.claude-b.")));
}

#[test]
fn a_repair_refused_while_claude_code_runs_says_why() {
    let home = Home::new();
    home.env.add_process(mccboard_core::ClaudeProcess {
        pid: 4242,
        name: "claude".to_owned(),
        config_dir: mccboard_core::ProcessConfigDir::Dir { path: home.b() },
        cwd: None,
    });
    let mut dashboard = home.dashboard(Language::Italian);
    press(&mut dashboard, KeyCode::Down);
    press(&mut dashboard, KeyCode::Char('p'));
    let Some(Request::Repair(dir)) = press(&mut dashboard, KeyCode::Enter) else {
        panic!("no repair asked");
    };

    let result = home.core.repair(&dir, None);
    dashboard.message(Message::Repaired {
        config_dir: dir,
        result,
    });

    assert!(dashboard.notice.as_ref().unwrap().is_error());
    let text = screen(&dashboard);
    assert!(
        text.contains(&os("~/.claude-b non riparata: Claude Code è in esecuzione")),
        "{text}"
    );
    assert!(!home.b().join("agents").exists());
}

#[test]
fn l_switches_the_language_and_saves_it() {
    let home = Home::new();
    home.read_usage();
    let mut dashboard = home.dashboard(Language::English);

    assert_eq!(press(&mut dashboard, KeyCode::Char('l')), None);

    assert_eq!(dashboard.language, Language::Italian);
    assert!(
        home.config().contains("language = \"it\""),
        "{}",
        home.config()
    );
    let text = screen(&dashboard);
    assert!(text.contains("Lingua: Italiano"), "{text}");
    assert!(text.contains("Sessione"), "{text}");

    press(&mut dashboard, KeyCode::Char('l'));
    assert_eq!(dashboard.language, Language::English);
    assert!(home.config().contains("language = \"en\""));
    assert!(screen(&dashboard).contains("Language: English"));
}

#[test]
fn r_refreshes_within_the_rate_limits_and_says_when_it_could_not() {
    let home = Home::new();
    let mut dashboard = home.dashboard(Language::English);
    home.env.push_usage_reply(reply(35.0, 14.0, &[]));
    home.env.push_usage_reply(reply(10.0, 72.0, &[]));

    assert_eq!(
        press(&mut dashboard, KeyCode::Char('r')),
        Some(Request::Refresh)
    );
    assert!(screen(&dashboard).contains("Refreshing usage…"));
    refresh(&home, &mut dashboard);

    let text = screen(&dashboard);
    assert!(!text.contains("Refreshing usage…"), "{text}");
    assert!(text.contains("Usage refreshed."), "{text}");
    assert!(text.contains("■ A · 35%"), "{text}");
    assert_eq!(home.env.usage_calls(), 2);

    // Again at once: the rate limits allow no call before a minute has passed.
    press(&mut dashboard, KeyCode::Char('r'));
    refresh(&home, &mut dashboard);

    assert_eq!(home.env.usage_calls(), 2);
    let next = now() + SignedDuration::from_mins(1);
    assert_eq!(dashboard.notice, Some(Notice::NextCallAt(next)));
    let text = screen(&dashboard);
    assert!(
        text.contains("The usage endpoint allows the next call at 12:01: the last readings stay."),
        "{text}"
    );

    // A refresh the user didn't ask for leaves the notice alone.
    dashboard.message(Message::Usage(home.core.refresh_usage()));
    assert_eq!(dashboard.notice, Some(Notice::NextCallAt(next)));
}

#[test]
fn the_times_to_reset_follow_the_clock_when_the_views_are_computed_again() {
    let home = Home::new();
    home.read_usage();
    let mut dashboard = home.dashboard(Language::English);
    // The session resets at 15:00 sharp: its minutes change just after now.
    assert_eq!(dashboard.redraw_in(), Duration::from_millis(1));
    let views = dashboard.views();
    home.env.advance(SignedDuration::from_mins(75));

    // Until the views are computed again the screen keeps what it showed.
    assert!(line_with(&screen(&dashboard), "Session  ████").contains("35%      3h  0m"));
    dashboard.tick();

    assert_eq!(dashboard.views(), views + 1);
    let text = screen(&dashboard);
    assert!(
        line_with(&text, "Session  ████").contains("1h 45m"),
        "{text}"
    );
    assert!(
        line_with(&text, "Weekly   █░░").contains("3d 22h 45m"),
        "{text}"
    );
}

// --- In the TUI's loop ---------------------------------------------------------------------

fn tui(modes: &Recorded) -> Tui<TestBackend> {
    let guard = TerminalGuard::enter(modes.clone()).unwrap();
    Tui::new(Terminal::new(TestBackend::new(100, 34)).unwrap(), guard)
}

fn pause() -> Step {
    Step::Do(Box::new(|| std::thread::sleep(Duration::from_millis(150))))
}

#[test]
fn while_it_runs_the_endpoint_is_called_at_most_once_per_account_per_minute() {
    let _serial = serial();
    let home = Home::new();
    for _ in 0..8 {
        home.env.push_usage_reply(reply(35.0, 14.0, &[]));
    }
    let modes = Recorded::default();
    // Refreshing every few milliseconds and on every `r`: only the core's rate limits stand
    // between the dashboard and the endpoint.
    let mut screen =
        Runner::start(home.dashboard(Language::English), Duration::from_millis(5)).unwrap();
    let (env, clock) = (home.env.clone(), home.env.clone());
    let mut events = Script::new(vec![
        Step::Until("the first calls", Box::new(move || env.usage_calls() == 2)),
        Step::Key(KeyCode::Char('r')),
        Step::Key(KeyCode::Char('r')),
        pause(),
        Step::Key(KeyCode::Char('r')),
        pause(),
        Step::Do(Box::new(move || {
            clock.advance(SignedDuration::from_secs(59))
        })),
        Step::Key(KeyCode::Char('r')),
        pause(),
    ]);

    let mut tui = tui(&modes);
    app::run(&mut tui, &mut events, &mut screen).unwrap();
    assert_eq!(home.env.usage_calls(), 2, "within the first minute");

    // A minute after the first calls: one more each, and no more.
    home.env.advance(SignedDuration::from_secs(2));
    let env = home.env.clone();
    let mut events = Script::new(vec![
        Step::Until("the next calls", Box::new(move || env.usage_calls() == 4)),
        Step::Key(KeyCode::Char('r')),
        pause(),
        Step::Key(KeyCode::Char('r')),
        pause(),
    ]);
    app::run(&mut tui, &mut events, &mut screen).unwrap();
    assert_eq!(home.env.usage_calls(), 4);

    let shown = buffer_text(tui.terminal());
    assert_no_token(&shown);
    assert!(shown.contains("■ A · 35%"), "{shown}");
    drop(tui);
    assert_eq!(modes.log(), ["enter", "leave"]);
}

#[test]
fn a_repair_asked_in_the_loop_runs_on_its_thread_and_q_gives_the_terminal_back() {
    let _serial = serial();
    let home = Home::new();
    let modes = Recorded::default();
    let mut screen =
        Runner::start(home.dashboard(Language::English), super::REFRESH_EVERY).unwrap();
    let agents = home.b().join("agents");
    let mut events = Script::new(vec![
        Step::Key(KeyCode::Down),
        Step::Key(KeyCode::Char('p')),
        Step::Key(KeyCode::Char('y')),
        Step::Until("the repair", Box::new(move || agents.exists())),
        // Its outcome reaches the screen at the next update.
        pause(),
        Step::Key(KeyCode::Null),
    ]);

    let mut tui = tui(&modes);
    app::run(&mut tui, &mut events, &mut screen).unwrap();

    let shown = buffer_text(tui.terminal());
    assert!(
        shown.contains(&os("Repaired 1 item in ~/.claude-b.")),
        "{shown}"
    );
    assert_no_token(&shown);
    drop(tui);
    assert_eq!(modes.log(), ["enter", "leave"]);
}
