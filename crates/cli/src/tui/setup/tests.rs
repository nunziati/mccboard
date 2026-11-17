//! The setup on temporary homes through the fake environment port. Each step is driven as a
//! state by keys and by the messages the runner would send, on a fresh home, on a home already
//! set up and when a step fails. Frames are drawn on ratatui's `TestBackend` in both languages
//! and never show a token. In the TUI's loop, the setup hands the terminal to Claude Code and
//! takes it back whatever the outcome.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::KeyCode;
use jiff::{SignedDuration, Timestamp};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    AddRequest, ClaudeProcess, Core, InstallerExit, Language, LoginExit, LoginTerminal, OsKind,
    ProcessConfigDir, SharingAnswer,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;
use tempfile::TempDir;

use super::state::{Message, Notice, Request, Setup, StepId};
use super::steps::AddForm;
use super::{Console, Runner, view};
use crate::tui::Show;
use crate::tui::app;
use crate::tui::app::testing::{Script, Step, key};
use crate::tui::terminal::testing::{Recorded, serial};
use crate::tui::terminal::{TerminalGuard, Tui};

/// Token-shaped fixture values: no frame and no text in the terminal may show them.
const TOKEN_A: &str = "sk-ant-oat01-FIXTURE-A-must-never-leak-aaaaaaaaaaaa";
const TOKEN_C: &str = "sk-ant-oat01-FIXTURE-C-must-never-leak-cccccccccccc";
const UUID_A: &str = "11111111-1111-4111-8111-111111111111";
const UUID_B: &str = "22222222-2222-4222-8222-222222222222";
const UUID_C: &str = "33333333-3333-4333-8333-333333333333";

fn now() -> Timestamp {
    "2026-10-15T12:00:00Z".parse().unwrap()
}

/// A temporary home, its core and the fake environment the core runs on.
struct Home {
    dir: TempDir,
    core: Core,
    env: FakeEnvironment,
}

impl Home {
    /// A home where Claude Code was used before but isn't installed now: `~/.claude` is
    /// logged in as A and holds `agents/` (linked instances link it) and `gsd-core/` (an item
    /// mccboard doesn't know). Nothing is registered. `curl` and `bash` are on the PATH, for
    /// the installer.
    fn fresh() -> Home {
        let dir = tempfile::tempdir().unwrap();
        let env = FakeEnvironment::new(now());
        let core = Core::new(dir.path(), env.clone());
        let home = Home { dir, core, env };
        let primary = home.primary();
        fs::create_dir_all(primary.join("agents")).unwrap();
        fs::create_dir_all(primary.join("gsd-core")).unwrap();
        log_in(&home.path().join(".claude.json"), &primary, UUID_A, TOKEN_A);
        home.env
            .set_program("curl", Some(Path::new("/usr/bin/curl")));
        home.env
            .set_program("bash", Some(Path::new("/usr/bin/bash")));
        // The setup's language, whatever the system's.
        home.core.set_language(Language::English).unwrap();
        home
    }

    /// The fresh home with its primary instance registered and B added in `~/.claude-b`
    /// before anyone answered about `gsd-core`: B has one drift item, `link:gsd-core`, once
    /// `gsd-core` is answered "link".
    fn with_drift() -> Home {
        let home = Home::fresh();
        home.env
            .set_program("claude", Some(Path::new("/usr/bin/claude")));
        home.core.register(&home.primary()).unwrap();
        home.env.login_as(UUID_B, "b@example.com");
        home.core
            .add(&AddRequest::new("B", LoginTerminal::CurrentTerminal))
            .unwrap();
        home.core
            .answer_sharing("gsd-core", SharingAnswer::Link)
            .unwrap();
        home
    }

    /// A home as the setup leaves it: Claude Code on the PATH, A in the primary instance and B
    /// in `~/.claude-b`, both registered and logged in, `gsd-core` linked, no drift, and the
    /// launchers written.
    fn set_up() -> Home {
        let home = Home::with_drift();
        home.core.repair(&home.b(), None).unwrap();
        home.core.sync_launchers().unwrap();
        assert!(!home.core.doctor().unwrap().has_drift());
        home
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn primary(&self) -> PathBuf {
        self.path().join(".claude")
    }

    fn b(&self) -> PathBuf {
        self.path().join(".claude-b")
    }

    fn config(&self) -> String {
        fs::read_to_string(self.path().join(".mccboard").join("config.toml")).unwrap()
    }

    /// The shell rc file holding the launchers, empty while there is none.
    fn rc(&self) -> String {
        fs::read_to_string(self.path().join(".bashrc")).unwrap_or_default()
    }

    fn setup(&self, language: Language) -> Setup {
        Setup::new(self.core.clone(), language, OsKind::Linux)
    }

    /// Makes the official installer put an executable `claude` in `~/.local/bin`, as the real
    /// one does, running `also` first.
    fn installer_installs(&self, also: impl Fn() + Send + Sync + 'static) {
        self.env.on_claude_install(move |install| {
            also();
            let bin = install.home.join(".local").join("bin");
            fs::create_dir_all(&bin)?;
            fs::write(bin.join("claude"), "#!/bin/sh\n")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o755))?;
            }
            Ok(InstallerExit {
                download_exit_code: Some(0),
                exit_code: Some(0),
            })
        });
    }
}

/// Writes what Claude Code's login leaves: the account in `global_config`, credentials with
/// `token` valid for five hours in `config_dir`.
fn log_in(global_config: &Path, config_dir: &Path, uuid: &str, token: &str) {
    let name = match uuid {
        UUID_A => "a",
        UUID_B => "b",
        _ => "c",
    };
    let email = format!("{name}@example.com");
    let account = json!({"oauthAccount": {"accountUuid": uuid, "emailAddress": email}});
    fs::write(global_config, account.to_string()).unwrap();
    let expires = (now() + SignedDuration::from_hours(5)).as_millisecond();
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

/// What a terminal of 120 by 32 shows of the setup, line by line. No frame ever shows a
/// token.
fn screen(setup: &Setup) -> String {
    sized_screen(setup, 120, 32)
}

/// What a terminal of `width` by `height` shows of the setup.
fn sized_screen(setup: &Setup, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| view::draw(setup, frame)).unwrap();
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

fn assert_shows(screen: &str, texts: &[&str]) {
    for text in texts {
        assert!(screen.contains(text), "{text:?} missing in:\n{screen}");
    }
}

fn press(setup: &mut Setup, code: KeyCode) -> Option<Request> {
    setup.key(key(code))
}

fn type_text(setup: &mut Setup, text: &str) {
    for c in text.chars() {
        assert_eq!(press(setup, KeyCode::Char(c)), None);
    }
}

/// A path under the home as this OS shows it: `~\.claude-b` on Windows.
fn os(path: &str) -> String {
    path.replace('/', std::path::MAIN_SEPARATOR_STR)
}

/// The line of the screen that contains `needle`.
fn line_with<'a>(screen: &'a str, needle: &str) -> &'a str {
    screen
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line with {needle:?} in:\n{screen}"))
}

// --- A fresh home ------------------------------------------------------------------------

#[test]
fn a_fresh_home_is_set_up_step_by_step_and_a_second_run_changes_nothing() {
    let home = Home::fresh();
    home.installer_installs(|| {});
    let mut setup = home.setup(Language::English);

    // 1. Claude Code is missing: Enter offers the installer, and `y` runs it.
    assert_eq!(setup.step(), StepId::ClaudeCode);
    assert_eq!(setup.done(StepId::ClaudeCode), Some(false));
    let text = screen(&setup);
    assert_shows(
        &text,
        &[
            " · 1 Claude Code",
            "Claude Code isn't installed.",
            "curl -fsSL https://claude.ai/install.sh | bash",
            "Enter install",
        ],
    );
    assert_eq!(press(&mut setup, KeyCode::Enter), None);
    assert_shows(
        &screen(&setup),
        &[
            "Run Claude Code's official installer in this terminal?",
            "y/Enter run  n/Esc cancel",
        ],
    );
    // While it asks, the other keys stay with the question.
    assert_eq!(press(&mut setup, KeyCode::Char('q')), None);
    assert_eq!(press(&mut setup, KeyCode::Right), None);
    assert_eq!(press(&mut setup, KeyCode::Char('n')), None);
    assert_shows(&screen(&setup), &["Nothing was run."]);
    assert!(home.env.installer_invocations().is_empty());

    press(&mut setup, KeyCode::Enter);
    assert_eq!(
        press(&mut setup, KeyCode::Char('y')),
        Some(Request::InstallClaudeCode)
    );
    setup.message(Message::Installed(home.core.install_claude_code()));
    assert_eq!(home.env.installer_invocations().len(), 1);
    assert_eq!(setup.done(StepId::ClaudeCode), Some(true));
    let text = screen(&setup);
    assert_shows(
        &text,
        &[
            " ✓ 1 Claude Code",
            "Claude Code is installed.",
            &os("Claude Code is installed: ~/.local/bin/claude, version 2.1.301 (Claude Code)."),
            "isn't on the PATH of this terminal yet",
        ],
    );

    // 2. The primary instance is checked: Enter registers it.
    press(&mut setup, KeyCode::Enter);
    assert_eq!(setup.step(), StepId::Instances);
    let text = screen(&setup);
    let row = line_with(&text, &os("~/.claude "));
    for cell in ["[x]", "primary", "a@example.com", "logged in"] {
        assert!(row.contains(cell), "{text}");
    }
    assert_eq!(press(&mut setup, KeyCode::Enter), None);
    let text = screen(&setup);
    assert_shows(&text, &[&os("Registered ~/.claude."), " ✓ 2 Instances"]);
    assert!(line_with(&text, &os("~/.claude ")).contains("logged in, registered"));

    // 3. A is logged in. `a` adds B: its label, the folder proposed, then the add.
    press(&mut setup, KeyCode::Enter);
    assert_eq!(setup.step(), StepId::Accounts);
    assert_eq!(setup.done(StepId::Accounts), Some(true));
    let text = screen(&setup);
    let row = line_with(&text, &os("~/.claude "));
    for cell in ["A", "a@example.com", "logged in"] {
        assert!(row.contains(cell), "{text}");
    }
    press(&mut setup, KeyCode::Char('a'));
    assert_shows(
        &screen(&setup),
        &["Label of the new account", "Enter next  Esc close"],
    );
    type_text(&mut setup, "B");
    press(&mut setup, KeyCode::Enter);
    assert_shows(
        &screen(&setup),
        &[
            "Folder of the instance of B, which you may change:",
            &os("> ~/.claude-b"),
            "Enter add and log in  Esc back",
        ],
    );
    let request = press(&mut setup, KeyCode::Enter);
    assert_eq!(
        request,
        Some(Request::Add(AddRequest::new(
            "B",
            LoginTerminal::CurrentTerminal
        )))
    );
    // The runner hands the terminal to the add, whose login logs in as B.
    home.env.login_as(UUID_B, "b@example.com");
    let Some(Request::Add(request)) = request else {
        unreachable!()
    };
    setup.message(Message::Added(home.core.add(&request)));
    let text = screen(&setup);
    assert_shows(
        &text,
        &[&os(
            "Added ~/.claude-b. ~/.claude-b is logged in, with account B.",
        )],
    );
    // The next account's label is asked, until the user leaves it empty.
    assert_eq!(
        setup.accounts.form,
        Some(AddForm::Label {
            text: String::new()
        })
    );
    assert_eq!(press(&mut setup, KeyCode::Enter), None);
    assert_eq!(setup.accounts.form, None);
    let text = screen(&setup);
    let row = line_with(&text, &os("~/.claude-b"));
    for cell in ["B", "b@example.com", "logged in"] {
        assert!(row.contains(cell), "{row}");
    }

    // 4. gsd-core is asked about: `l` links it.
    press(&mut setup, KeyCode::Right);
    assert_eq!(setup.step(), StepId::Sharing);
    assert_shows(
        &screen(&setup),
        &[
            "Link the folder gsd-core in the linked instances, or keep it private? (1 of 1)",
            "l link  p keep private  s skip",
        ],
    );
    assert_eq!(press(&mut setup, KeyCode::Char('l')), None);
    let text = screen(&setup);
    assert_shows(
        &text,
        &[
            "gsd-core will be linked from the primary instance in every linked instance.",
            "nothing to ask",
            " ✓ 4 Sharing",
        ],
    );
    assert!(home.config().contains("gsd-core"), "{}", home.config());

    // 5. B now lacks the link to gsd-core: Enter asks, `y` repairs.
    press(&mut setup, KeyCode::Enter);
    assert_eq!(setup.step(), StepId::Drift);
    let text = screen(&setup);
    assert_shows(
        &text,
        &[
            "link:gsd-core",
            &format!(
                "Repair: create gsd-core as a link to {}.",
                os("~/.claude/gsd-core")
            ),
        ],
    );
    assert!(line_with(&text, &os("~/.claude-b")).contains("1 item to fix"));
    assert_eq!(press(&mut setup, KeyCode::Enter), None);
    assert_shows(
        &screen(&setup),
        &[&os("Repair ~/.claude-b?"), "It fixes 1 item:"],
    );
    assert_eq!(
        press(&mut setup, KeyCode::Char('y')),
        Some(Request::Repair(home.b()))
    );
    assert!(setup.busy());
    assert_shows(&screen(&setup), &[&os("Repairing ~/.claude-b…")]);
    // Nothing else happens while the repair runs.
    assert_eq!(press(&mut setup, KeyCode::Char('q')), None);
    let result = home.core.repair(&home.b(), None);
    setup.message(Message::Repaired {
        config_dir: home.b(),
        result,
    });
    let text = screen(&setup);
    assert_shows(
        &text,
        &[&os("Repaired 1 item in ~/.claude-b."), " ✓ 5 Drift"],
    );
    assert!(home.b().join("gsd-core").exists());

    // 6. The add wrote B's launcher: the step shows how to use it in this shell.
    press(&mut setup, KeyCode::Enter);
    assert_eq!(setup.step(), StepId::Launchers);
    let text = screen(&setup);
    assert_shows(
        &text,
        &[
            &format!(
                "Launchers are aliases in mccboard's block of {}.",
                os("~/.bashrc")
            ),
            &format!("source {}", os("~/.bashrc")),
        ],
    );
    let row = line_with(&text, "claude-b ");
    assert!(
        row.contains(&os("~/.claude-b")) && row.contains("ok"),
        "{text}"
    );
    assert!(home.rc().contains("alias claude-b="), "{}", home.rc());

    // 7. Space switches to Italian, then turns the renewal off, both saved at once.
    press(&mut setup, KeyCode::Enter);
    assert_eq!(setup.step(), StepId::Preferences);
    assert_shows(
        &screen(&setup),
        &[
            "Language: English",
            "Claude Code may renew an expired token: on",
        ],
    );
    press(&mut setup, KeyCode::Char(' '));
    assert!(home.config().contains("language = \"it\""));
    press(&mut setup, KeyCode::Down);
    press(&mut setup, KeyCode::Char(' '));
    assert!(!home.core.renew_expired_tokens().unwrap());
    assert_shows(
        &screen(&setup),
        &[
            "Lingua: Italiano",
            "Claude Code può rinnovare un token scaduto: no",
            "A Claude Code non verrà chiesto di rinnovare un token scaduto",
        ],
    );

    // 8. The summary, then Enter opens the dashboard.
    press(&mut setup, KeyCode::Enter);
    assert_eq!(setup.step(), StepId::Summary);
    let text = screen(&setup);
    assert_shows(
        &text,
        &[
            "■ A  a@example.com",
            "■ B  b@example.com",
            "Istanze:",
            "Invio apre la dashboard",
        ],
    );
    assert!(!text.contains("Ancora da fare"), "{text}");
    let row = line_with(&text, &os("~/.claude-b"));
    for cell in [
        "collegata",
        "B",
        "login effettuato",
        "nessuna differenza",
        "claude-b",
    ] {
        assert!(row.contains(cell), "{text}");
    }
    assert_eq!(press(&mut setup, KeyCode::Enter), Some(Request::Dashboard));
    for step in &StepId::ALL[..7] {
        assert_eq!(setup.done(*step), Some(true), "{step:?}");
    }

    // A second run finds everything done, and Enter through every step changes nothing.
    let (config, rc) = (home.config(), home.rc());
    let logins = home.env.login_invocations().len();
    let mut again = home.setup(Language::Italian);
    walk_through(&mut again);
    assert_eq!(home.config(), config);
    assert_eq!(home.rc(), rc);
    assert_eq!(home.env.login_invocations().len(), logins);
    assert_eq!(home.env.installer_invocations().len(), 1);
}

/// Presses Enter on every step from the first, checking that each one is done and that none
/// asks the runner for anything until the summary opens the dashboard.
fn walk_through(setup: &mut Setup) {
    for step in &StepId::ALL[..7] {
        assert_eq!(setup.step(), *step);
        assert_eq!(setup.done(*step), Some(true), "{step:?}");
        assert_eq!(press(setup, KeyCode::Enter), None, "{step:?}");
        assert_eq!(setup.notice, None, "{step:?}");
    }
    assert_eq!(setup.step(), StepId::Summary);
    assert_eq!(press(setup, KeyCode::Enter), Some(Request::Dashboard));
}

// --- A home already set up ---------------------------------------------------------------

#[test]
fn a_home_already_set_up_shows_every_step_done_and_running_it_again_changes_nothing() {
    let home = Home::set_up();
    let (config, rc) = (home.config(), home.rc());
    let (logins, runs) = (
        home.env.login_invocations().len(),
        home.env.claude_invocations().len(),
    );

    let mut setup = home.setup(Language::English);
    let text = screen(&setup);
    assert_shows(
        &text,
        &[
            " ✓ 1 Claude Code",
            " ✓ 2 Instances",
            " ✓ 3 Accounts",
            " ✓ 4 Sharing",
            " ✓ 5 Drift",
            " ✓ 6 Launchers",
            " ✓ 7 Preferences",
            "   8 Summary",
            "Claude Code is installed: /usr/bin/claude, version 2.1.301 (Claude Code).",
        ],
    );
    assert!(!text.contains("isn't on the PATH"), "{text}");
    walk_through(&mut setup);

    // And so on every later run.
    walk_through(&mut home.setup(Language::English));
    assert_eq!(home.config(), config);
    assert_eq!(home.rc(), rc);
    assert_eq!(home.env.login_invocations().len(), logins);
    assert_eq!(home.env.claude_invocations().len(), runs);
    assert!(home.env.installer_invocations().is_empty());
}

#[test]
fn each_step_shows_what_is_done_in_italian() {
    let home = Home::set_up();
    let mut setup = home.setup(Language::Italian);
    let expected: [&[&str]; 8] = [
        &[
            "Claude Code è installato: /usr/bin/claude, versione 2.1.301",
            "Invio continua",
        ],
        &[
            "Istanze trovate nella tua home",
            "login effettuato, registrata",
            "primaria",
            "↑↓ seleziona  Spazio spunta  Invio continua",
        ],
        &["Ogni account accede a una sua istanza", "login effettuato"],
        &["L'istanza principale non ha elementi che mccboard non conosce"],
        &[
            "Ogni istanza confrontata con la sua condivisione",
            "nessuna differenza",
        ],
        &[
            "I comandi di avvio sono alias nel blocco di mccboard",
            "Per usarli in questo terminale, esegui la riga qui sotto.",
        ],
        &[
            "Lingua: Italiano",
            "Claude Code può rinnovare un token scaduto: sì",
        ],
        &["Account:", "Istanze:", "Invio apre la dashboard"],
    ];
    for (step, texts) in StepId::ALL.iter().zip(expected) {
        assert_eq!(setup.step(), *step);
        let text = screen(&setup);
        assert_shows(
            &text,
            &[
                " Passi ",
                "Condivisione",
                "Differenze",
                "Comandi di avvio",
                "Preferenze",
                "Riepilogo",
                "←→ passo  q esci",
            ],
        );
        assert_shows(&text, texts);
        for english in ["Steps", "Sharing", "Launchers", "Enter", "quit"] {
            assert!(!text.contains(english), "{english:?} in:\n{text}");
        }
        press(&mut setup, KeyCode::Right);
    }
}

#[test]
fn on_an_80_column_terminal_the_steps_take_one_line_and_folders_stay_whole() {
    let home = Home::set_up();
    let mut setup = home.setup(Language::Italian);
    press(&mut setup, KeyCode::Char('8'));

    let text = sized_screen(&setup, 80, 24);

    assert_shows(
        &text,
        &[" ✓ 1  ✓ 2  ✓ 3  ✓ 4  ✓ 5  ✓ 6  ✓ 7    8", " 8. Riepilogo "],
    );
    assert!(!text.contains("Passi"), "{text}");
    // The widest table of the setup, in its widest language: the folders stay whole.
    for folder in ["~/.claude ", "~/.claude-b "] {
        assert!(text.contains(&os(folder)), "{folder:?} cut in:\n{text}");
    }
}

// --- Moving between steps ----------------------------------------------------------------

#[test]
fn every_step_can_be_skipped_shown_again_or_reached_by_its_number() {
    let home = Home::fresh();
    let mut setup = home.setup(Language::English);

    // Skipping Claude Code and the instances leaves them as they were.
    press(&mut setup, KeyCode::Right);
    press(&mut setup, KeyCode::Tab);
    assert_eq!(setup.step(), StepId::Accounts);
    assert_eq!(setup.done(StepId::ClaudeCode), Some(false));
    assert!(screen(&setup).contains(" · 2 Instances"));
    press(&mut setup, KeyCode::Left);
    assert_eq!(setup.step(), StepId::Instances);
    press(&mut setup, KeyCode::Char('8'));
    assert_eq!(setup.step(), StepId::Summary);
    let text = screen(&setup);
    assert_shows(
        &text,
        &[
            "No account yet: log in an instance in step 3.",
            "Still to do: Claude Code, Instances, Accounts, Sharing.",
        ],
    );
    // No step after the last, none before the first.
    press(&mut setup, KeyCode::Right);
    assert_eq!(setup.step(), StepId::Summary);
    press(&mut setup, KeyCode::Char('1'));
    press(&mut setup, KeyCode::Left);
    assert_eq!(setup.step(), StepId::ClaudeCode);
    assert_eq!(press(&mut setup, KeyCode::Char('q')), Some(Request::Quit));
    assert!(home.env.installer_invocations().is_empty());
    assert!(!home.path().join(".mccboard").join("state").exists());
}

#[test]
fn space_checks_the_instances_to_register_and_only_those_are_registered() {
    let home = Home::fresh();
    for name in [".claude-old", ".claude-work"] {
        fs::create_dir_all(home.path().join(name)).unwrap();
    }
    let mut setup = home.setup(Language::English);
    press(&mut setup, KeyCode::Char('2'));
    let text = screen(&setup);
    assert!(
        line_with(&text, &os("~/.claude ")).contains("[x]"),
        "{text}"
    );
    assert!(
        line_with(&text, &os("~/.claude-old")).contains("[ ]"),
        "{text}"
    );

    // Unchecks the primary instance, checks ~/.claude-work.
    press(&mut setup, KeyCode::Char(' '));
    press(&mut setup, KeyCode::Down);
    press(&mut setup, KeyCode::Down);
    press(&mut setup, KeyCode::Char(' '));
    assert_eq!(press(&mut setup, KeyCode::Enter), None);

    let registered: Vec<PathBuf> = home
        .core
        .registry()
        .unwrap()
        .instances
        .into_iter()
        .map(|instance| instance.config_dir)
        .collect();
    assert_eq!(registered, [home.path().join(".claude-work")]);
    // The primary instance isn't registered yet: the step isn't done.
    assert_eq!(setup.done(StepId::Instances), Some(false));
    // With nothing checked, Enter moves on.
    assert_eq!(press(&mut setup, KeyCode::Enter), None);
    assert_eq!(setup.step(), StepId::Accounts);
}

#[test]
fn without_the_primary_folder_the_instances_step_says_how_to_get_it() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::new(dir.path(), FakeEnvironment::new(now()));
    let mut setup = Setup::new(core, Language::English, OsKind::Linux);

    press(&mut setup, KeyCode::Right);

    assert_shows(
        &screen(&setup),
        &["the folder of Claude Code itself, isn't there yet"],
    );
    assert_eq!(setup.done(StepId::Instances), Some(false));
}

#[test]
fn keys_typed_in_the_form_stay_in_the_form() {
    let home = Home::fresh();
    home.core.register(&home.primary()).unwrap();
    let mut setup = home.setup(Language::English);
    press(&mut setup, KeyCode::Char('3'));
    press(&mut setup, KeyCode::Char('a'));

    type_text(&mut setup, "q1 Work");
    press(&mut setup, KeyCode::Right);
    press(&mut setup, KeyCode::Backspace);

    assert_eq!(setup.step(), StepId::Accounts);
    assert_eq!(
        setup.accounts.form,
        Some(AddForm::Label {
            text: "q1 Wor".to_owned()
        })
    );
    // The folder proposed follows the label, and another one can be typed instead.
    press(&mut setup, KeyCode::Enter);
    let Some(AddForm::Folder { text, .. }) = &setup.accounts.form else {
        panic!("no folder asked");
    };
    assert_eq!(text, &os("~/.claude-q1-wor"));
    for _ in 0.."q1-wor".len() {
        press(&mut setup, KeyCode::Backspace);
    }
    type_text(&mut setup, "work");
    let Some(Request::Add(request)) = press(&mut setup, KeyCode::Enter) else {
        panic!("no add asked");
    };
    assert_eq!(request.label, "q1 Wor");
    assert_eq!(
        request.config_dir,
        Some(PathBuf::from(os("~/.claude-work")))
    );
    assert!(request.login);
    assert_eq!(request.terminal, LoginTerminal::CurrentTerminal);

    // Esc goes back to the label, then closes the form.
    press(&mut setup, KeyCode::Esc);
    assert!(matches!(setup.accounts.form, Some(AddForm::Label { .. })));
    press(&mut setup, KeyCode::Esc);
    assert_eq!(setup.accounts.form, None);
    assert_eq!(press(&mut setup, KeyCode::Char('q')), Some(Request::Quit));
}

#[test]
fn a_skipped_item_is_asked_again_the_next_time() {
    let home = Home::fresh();
    home.core.register(&home.primary()).unwrap();
    let mut setup = home.setup(Language::English);
    press(&mut setup, KeyCode::Char('4'));

    press(&mut setup, KeyCode::Char('s'));

    let text = screen(&setup);
    assert_shows(&text, &["Skipped for now: gsd-core.", "Enter continue"]);
    assert_eq!(setup.done(StepId::Sharing), Some(false));
    assert!(!home.config().contains("gsd-core"));
    let mut again = home.setup(Language::English);
    press(&mut again, KeyCode::Char('4'));
    assert!(screen(&again).contains("Link the folder gsd-core"));
}

// --- Steps that fail ---------------------------------------------------------------------

#[test]
fn an_installer_that_fails_says_why_in_both_languages() {
    let home = Home::fresh();
    home.env.on_claude_install(|_| {
        Ok(InstallerExit {
            download_exit_code: Some(0),
            exit_code: Some(3),
        })
    });
    let mut setup = home.setup(Language::English);
    press(&mut setup, KeyCode::Enter);
    press(&mut setup, KeyCode::Enter);

    setup.message(Message::Installed(home.core.install_claude_code()));

    assert!(setup.notice.as_ref().unwrap().is_error());
    assert_eq!(setup.done(StepId::ClaudeCode), Some(false));
    assert_shows(
        &screen(&setup),
        &[
            "Claude Code isn't installed: Claude Code's official installer failed (exit code 3)",
            " · 1 Claude Code",
        ],
    );
    setup.language = Language::Italian;
    assert_shows(
        &screen(&setup),
        &["Claude Code non è installato: Claude Code's official installer failed (exit code 3)"],
    );
}

#[test]
fn an_installer_that_cannot_run_without_curl_says_so() {
    let home = Home::fresh();
    home.env.set_program("curl", None);
    let mut setup = home.setup(Language::English);
    press(&mut setup, KeyCode::Enter);
    press(&mut setup, KeyCode::Char('y'));

    setup.message(Message::Installed(home.core.install_claude_code()));

    assert!(home.env.installer_invocations().is_empty());
    assert_shows(&screen(&setup), &["needs curl, which isn't on the PATH"]);
}

#[test]
fn on_windows_the_step_shows_the_powershell_command_and_runs_nothing() {
    let home = Home::fresh();
    let mut setup = Setup::new(home.core.clone(), Language::English, OsKind::Windows);

    assert_shows(
        &screen(&setup),
        &[
            "Install it in PowerShell",
            "irm https://claude.ai/install.ps1 | iex",
        ],
    );
    assert_eq!(press(&mut setup, KeyCode::Enter), None);
    assert_eq!(setup.step(), StepId::Instances);
    assert!(home.env.installer_invocations().is_empty());
}

#[test]
fn a_repair_refused_while_claude_code_runs_says_why() {
    let home = Home::with_drift();
    home.env.add_process(ClaudeProcess {
        pid: 4242,
        name: "claude".to_owned(),
        config_dir: ProcessConfigDir::Dir { path: home.b() },
        cwd: None,
    });
    let mut setup = home.setup(Language::Italian);
    press(&mut setup, KeyCode::Char('5'));
    press(&mut setup, KeyCode::Enter);
    let Some(Request::Repair(dir)) = press(&mut setup, KeyCode::Enter) else {
        panic!("no repair asked");
    };

    let result = home.core.repair(&dir, None);
    setup.message(Message::Repaired {
        config_dir: dir,
        result,
    });

    assert!(!setup.busy());
    assert!(setup.notice.as_ref().unwrap().is_error());
    assert_shows(
        &screen(&setup),
        &[&os(
            "~/.claude-b non riparata: Claude Code è in esecuzione in questa istanza",
        )],
    );
    assert!(!home.b().join("gsd-core").exists());
    assert_eq!(setup.done(StepId::Drift), Some(false));
}

#[test]
fn a_login_that_logs_nothing_in_says_so_and_offers_it_again() {
    let home = Home::fresh();
    let c = home.path().join(".claude-c");
    fs::create_dir_all(&c).unwrap();
    home.core.register(&home.primary()).unwrap();
    home.core.register(&c).unwrap();
    let mut setup = home.setup(Language::English);
    press(&mut setup, KeyCode::Char('3'));
    assert_eq!(setup.done(StepId::Accounts), Some(false));
    let text = screen(&setup);
    assert!(text.contains(&os("│› ~/.claude-c")), "{text}");
    assert_shows(&text, &["Enter log in", " · 3 Accounts"]);

    let request = press(&mut setup, KeyCode::Enter);
    assert_eq!(request, Some(Request::Login(c.clone())));
    // The fake login is closed without logging in.
    let result = home.core.login(&c, LoginTerminal::CurrentTerminal);
    setup.message(Message::LoggedIn {
        config_dir: c.clone(),
        result,
    });

    assert_eq!(setup.notice, Some(Notice::NotLoggedIn(c.clone())));
    assert_shows(
        &screen(&setup),
        &[&os(
            "~/.claude-c is still not logged in: Enter on it opens the login again.",
        )],
    );
    assert_eq!(press(&mut setup, KeyCode::Enter), Some(Request::Login(c)));
}

#[test]
fn an_add_refused_says_why_and_keeps_the_form_to_try_again() {
    let home = Home::fresh();
    home.core.register(&home.primary()).unwrap();
    let mut setup = home.setup(Language::English);
    press(&mut setup, KeyCode::Char('3'));
    press(&mut setup, KeyCode::Char('a'));
    // A label that names no folder.
    type_text(&mut setup, "!!");
    press(&mut setup, KeyCode::Enter);
    assert!(matches!(setup.notice, Some(Notice::LabelRefused(_))));
    assert_eq!(
        setup.accounts.form,
        Some(AddForm::Label {
            text: "!!".to_owned()
        })
    );

    // The primary instance's folder is refused by the core.
    for _ in 0..2 {
        press(&mut setup, KeyCode::Backspace);
    }
    type_text(&mut setup, "B");
    press(&mut setup, KeyCode::Enter);
    for _ in 0.."~/.claude-b".len() {
        press(&mut setup, KeyCode::Backspace);
    }
    type_text(&mut setup, "~/.claude");
    let Some(Request::Add(request)) = press(&mut setup, KeyCode::Enter) else {
        panic!("no add asked");
    };
    setup.message(Message::Added(home.core.add(&request)));

    assert!(matches!(setup.notice, Some(Notice::AddFailed(_))));
    assert!(screen(&setup).contains("Not added: "));
    assert!(matches!(setup.accounts.form, Some(AddForm::Folder { .. })));
    assert!(home.env.login_invocations().is_empty());
}

// --- In the TUI's loop -------------------------------------------------------------------

/// A console that writes down, in the terminal's log, that the setup spoke and waited.
struct RecordedConsole(Recorded);

impl Console for RecordedConsole {
    fn say(&mut self, text: &str) {
        assert_no_token(text);
        self.0.push("say");
    }

    fn wait_for_enter(&mut self, _prompt: &str) {
        self.0.push("wait");
    }
}

fn tui(modes: &Recorded) -> Tui<TestBackend> {
    let guard = TerminalGuard::enter(modes.clone()).unwrap();
    Tui::new(Terminal::new(TestBackend::new(120, 32)).unwrap(), guard)
}

/// Runs the setup of `home` in the loop with `steps`, then `q`, and returns what the last
/// frame showed.
fn run_setup(home: &Home, modes: &Recorded, steps: Vec<Step>) -> String {
    let mut console = RecordedConsole(modes.clone());
    let mut runner = Runner::new(home.setup(Language::English), &mut console);
    let mut events = Script::new(steps);
    let mut tui = tui(modes);
    app::run(&mut tui, &mut events, &mut runner).unwrap();
    assert!(!runner.setup().busy());
    let shown = buffer_text(tui.terminal());
    assert_no_token(&shown);
    shown
}

/// How a scripted login ends.
#[derive(Debug, Clone, Copy)]
enum Outcome {
    LogsIn,
    Closed,
    NotOpened,
}

#[test]
fn the_login_runs_in_the_terminal_and_the_setup_comes_back_whatever_its_outcome() {
    let _serial = serial();
    for outcome in [Outcome::LogsIn, Outcome::Closed, Outcome::NotOpened] {
        let home = Home::fresh();
        let c = home.path().join(".claude-c");
        fs::create_dir_all(&c).unwrap();
        home.core.register(&home.primary()).unwrap();
        home.core.register(&c).unwrap();
        let modes = Recorded::default();
        let log = modes.clone();
        home.env.on_login(move |login| {
            // The login runs while the terminal is the user's.
            log.push("login");
            match outcome {
                Outcome::LogsIn => {
                    log_in(
                        &login.config_dir.join(".claude.json"),
                        &login.config_dir,
                        UUID_C,
                        TOKEN_C,
                    );
                    Ok(LoginExit { exit_code: Some(0) })
                }
                Outcome::Closed => Ok(LoginExit { exit_code: Some(1) }),
                Outcome::NotOpened => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "claude is neither on the PATH nor in ~/.local/bin",
                )),
            }
        });

        let shown = run_setup(
            &home,
            &modes,
            vec![Step::Key(KeyCode::Char('3')), Step::Key(KeyCode::Enter)],
        );

        let (log, notice): (&[&str], String) = match outcome {
            // Logged in: straight back to the setup.
            Outcome::LogsIn => (
                &["enter", "leave", "say", "login", "enter", "leave"],
                os("~/.claude-c is logged in, with account C."),
            ),
            // Not logged in: the user reads what Claude Code said, then comes back.
            Outcome::Closed => (
                &["enter", "leave", "say", "login", "wait", "enter", "leave"],
                os("~/.claude-c is still not logged in"),
            ),
            Outcome::NotOpened => (
                &["enter", "leave", "say", "login", "enter", "leave"],
                os("The login of ~/.claude-c didn't open: Claude Code isn't installed"),
            ),
        };
        assert_eq!(modes.log(), log, "{outcome:?}");
        assert!(shown.contains(&notice), "{outcome:?}:\n{shown}");
    }
}

#[test]
fn the_installer_runs_in_the_terminal_and_the_user_reads_it_before_the_setup_comes_back() {
    let _serial = serial();
    let home = Home::fresh();
    let modes = Recorded::default();
    let log = modes.clone();
    home.installer_installs(move || log.push("installer"));

    let shown = run_setup(
        &home,
        &modes,
        vec![Step::Key(KeyCode::Enter), Step::Key(KeyCode::Char('y'))],
    );

    assert_eq!(
        modes.log(),
        [
            "enter",
            "leave",
            "say",
            "installer",
            "wait",
            "enter",
            "leave"
        ]
    );
    assert!(shown.contains("Claude Code is installed."), "{shown}");
    assert!(shown.contains(" ✓ 1 Claude Code"), "{shown}");
}

#[test]
fn an_add_hands_the_terminal_to_its_login_and_asks_for_the_next_account() {
    let _serial = serial();
    let home = Home::fresh();
    home.core.register(&home.primary()).unwrap();
    let modes = Recorded::default();
    let log = modes.clone();
    home.env.on_login(move |login| {
        log.push("login");
        log_in(
            &login.config_dir.join(".claude.json"),
            &login.config_dir,
            UUID_B,
            TOKEN_C,
        );
        Ok(LoginExit { exit_code: Some(0) })
    });

    let shown = run_setup(
        &home,
        &modes,
        vec![
            Step::Key(KeyCode::Char('3')),
            Step::Key(KeyCode::Char('a')),
            Step::Key(KeyCode::Char('B')),
            Step::Key(KeyCode::Enter),
            Step::Key(KeyCode::Enter),
            // The label of the next account is asked: none.
            Step::Key(KeyCode::Esc),
        ],
    );

    assert_eq!(
        modes.log(),
        ["enter", "leave", "say", "login", "enter", "leave"]
    );
    assert!(
        shown.contains(&os(
            "Added ~/.claude-b. ~/.claude-b is logged in, with account B."
        )),
        "{shown}"
    );
    assert!(home.b().join("agents").exists());
}

fn pause() -> Step {
    Step::Do(Box::new(|| std::thread::sleep(Duration::from_millis(150))))
}

#[test]
fn a_repair_runs_on_its_thread_and_the_setup_waits_for_it() {
    let _serial = serial();
    let home = Home::with_drift();
    let modes = Recorded::default();
    let link = home.b().join("gsd-core");

    let shown = run_setup(
        &home,
        &modes,
        vec![
            Step::Key(KeyCode::Char('5')),
            Step::Key(KeyCode::Enter),
            Step::Key(KeyCode::Char('y')),
            Step::Until("the repair", Box::new(move || link.exists())),
            // Its outcome reaches the screen at the next update.
            pause(),
            Step::Key(KeyCode::Null),
        ],
    );

    assert!(
        shown.contains(&os("Repaired 1 item in ~/.claude-b.")),
        "{shown}"
    );
    assert!(shown.contains(" ✓ 5 Drift"), "{shown}");
    assert_eq!(modes.log(), ["enter", "leave"]);
}

#[test]
fn s_in_the_dashboard_opens_the_setup_and_the_setup_ends_on_the_dashboard() {
    let _serial = serial();
    let home = Home::set_up();
    let modes = Recorded::default();
    let mut console = RecordedConsole(modes.clone());
    let mut events = Script::new(vec![
        Step::Key(KeyCode::Char('s')),
        // In the setup: Italian, then the summary, then Enter.
        Step::Key(KeyCode::Char('7')),
        Step::Key(KeyCode::Char(' ')),
        Step::Key(KeyCode::Char('8')),
        Step::Key(KeyCode::Enter),
        Step::Key(KeyCode::Null),
    ]);

    let mut tui = tui(&modes);
    crate::tui::show(
        &home.core,
        &mut tui,
        &mut events,
        &mut console,
        Show::Dashboard,
    )
    .unwrap();

    // Back in the dashboard, which speaks the language chosen in the setup.
    let shown = buffer_text(tui.terminal());
    assert_no_token(&shown);
    assert!(shown.contains(" Istanze "), "{shown}");
    assert!(shown.contains("s configura"), "{shown}");
    drop(tui);
    // One terminal session throughout.
    assert_eq!(modes.log(), ["enter", "leave"]);
}
