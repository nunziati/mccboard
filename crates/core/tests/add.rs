//! Adding a linked instance, with login (spec stories 13–16, 21–24 and 66; ticket #11): one
//! action makes the folder, links and aligns it, creates its launcher, opens Claude Code's
//! official login through the environment port, then observes the account and registers the
//! instance under it. Rerunning it after a failure at any step completes what's missing.
//! "Log in" also works on its own for any registered instance.

mod common;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use common::{TempHome, UUID_A, UUID_B, UUID_C};
use mccboard_core::testing::{FAKE_LOGIN_TOKEN, FakeEnvironment};
use mccboard_core::{
    AddRequest, ClaudeInvocation, ClaudeProcess, ClaudeRun, Core, Error, InstanceKind, LoginExit,
    LoginRun, LoginState, LoginTerminal, LoginWarning, OsKind, ProcessConfigDir, RepairOutcome,
    SharingAnswer,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------------
// Fixtures

fn write(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// Creates a folder link at `at` to `target`, the way a user would by hand.
fn link_dir(target: &Path, at: &Path) {
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, at).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(target, at).unwrap();
}

fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// A link's first hop, as written (Windows may add the verbatim prefix).
fn first_hop(path: &Path) -> PathBuf {
    let text = fs::read_link(path).unwrap().to_string_lossy().into_owned();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

struct Fixture {
    home: TempHome,
    core: Core,
    env: FakeEnvironment,
    primary: PathBuf,
}

/// A home whose primary instance is logged in as A and registered, with a settings file, a
/// `CLAUDE.md`, an `agents/` folder, a personal skill and some Global config keys.
fn fixture() -> Fixture {
    let home = TempHome::new();
    let primary = home.primary();
    primary.global_config(json!({
        "oauthAccount": {"accountUuid": UUID_A, "emailAddress": "a@example.com"},
        "diffTool": "auto",
        "defaultToAgentsView": true,
        "projects": {"/home/me/a": {}},
    }));
    write(primary.dir.join("settings.json"), r#"{"model": "opus"}"#);
    write(primary.dir.join("CLAUDE.md"), "# Me\n");
    write(primary.dir.join("agents/reviewer.md"), "reviewer\n");
    write(primary.dir.join("skills/notes/SKILL.md"), "notes\n");
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();
    Fixture {
        primary: primary.dir.clone(),
        home,
        core,
        env,
    }
}

impl Fixture {
    fn path(&self, relative: &str) -> PathBuf {
        self.home.path().join(relative)
    }

    fn rc_file(&self) -> String {
        fs::read_to_string(self.path(".bashrc")).unwrap_or_default()
    }

    /// The registered instance in `dir`, if any.
    fn registered(&self, dir: &Path) -> Option<mccboard_core::InstanceStatus> {
        self.core
            .registry()
            .unwrap()
            .instances
            .into_iter()
            .find(|i| i.config_dir == dir)
    }

    fn label_of(&self, uuid: &str) -> Option<String> {
        self.core
            .registry()
            .unwrap()
            .accounts
            .into_iter()
            .find(|a| a.uuid.as_str() == uuid)
            .map(|a| a.label)
    }
}

/// The primary also has one plugin from a GitHub marketplace.
fn primary_plugin(primary: &Path) {
    write(
        primary.join("plugins/known_marketplaces.json"),
        &json!({"tools": {
            "source": {"source": "github", "repo": "owner/tools"},
            "installLocation": primary.join("plugins/marketplaces/tools"),
        }})
        .to_string(),
    );
    write(
        primary.join("plugins/installed_plugins.json"),
        &json!({"version": 2, "plugins": {"review@tools": [{"scope": "user", "version": "1.0.0"}]}})
            .to_string(),
    );
}

/// A `claude plugin` that records what it adds, like Claude Code's; with `fail`, installing
/// a plugin fails.
fn plugin_commands(env: &FakeEnvironment, fail: bool) {
    env.on_claude(move |invocation: &ClaudeInvocation| {
        let args: Vec<&str> = invocation.args.iter().map(String::as_str).collect();
        let dir = &invocation.config_dir;
        match args.as_slice() {
            ["plugin", "marketplace", "add", ..] => {
                write(
                    dir.join("plugins/known_marketplaces.json"),
                    &json!({"tools": {"source": {"source": "github", "repo": "owner/tools"}}})
                        .to_string(),
                );
                Ok(ClaudeRun::ok(
                    r#"{"command":"marketplace-add","outcome":"ok"}"#,
                ))
            }
            ["plugin", "install", id, ..] if !fail => {
                write(
                    dir.join("plugins/installed_plugins.json"),
                    &json!({"version": 2, "plugins": {*id: [{"scope": "user"}]}}).to_string(),
                );
                Ok(ClaudeRun::ok(r#"{"command":"install","outcome":"ok"}"#))
            }
            ["plugin", "install", ..] => Ok(ClaudeRun::exit(
                1,
                r#"{"command":"install","outcome":"failed","message":"network down"}"#,
                "",
            )),
            _ => Ok(ClaudeRun::exit(1, "", "unexpected command")),
        }
    });
}

/// An add from the CLI: the login runs in the current terminal.
fn add(label: &str) -> AddRequest {
    AddRequest::new(label, LoginTerminal::CurrentTerminal)
}

/// A login from the CLI.
fn login(f: &Fixture, dir: &Path) -> Result<mccboard_core::LoginReport, Error> {
    f.core.login(dir, LoginTerminal::CurrentTerminal)
}

// ---------------------------------------------------------------------------------------------
// The whole flow

#[test]
fn adding_makes_the_folder_links_aligns_creates_the_launcher_logs_in_and_registers() {
    let f = fixture();
    f.env.login_as(UUID_B, "b@example.com");

    let report = f.core.add(&add("B")).unwrap();

    let dir = f.path(".claude-b");
    assert_eq!(report.config_dir, dir);
    assert!(report.created);
    // Linked items: one link each, to the primary's real item.
    assert!(is_link(&dir.join("settings.json")));
    assert_eq!(
        first_hop(&dir.join("settings.json")),
        f.primary.join("settings.json")
    );
    assert!(is_link(&dir.join("CLAUDE.md")));
    assert!(is_link(&dir.join("agents")));
    assert!(is_link(&dir.join("skills/notes")));
    assert!(!is_link(&dir.join("skills")));
    // Aligned items: the Global config keys, next to what the login wrote.
    let global = read_json(dir.join(".claude.json"));
    assert_eq!(global["diffTool"], json!("auto"));
    assert_eq!(global["defaultToAgentsView"], json!(true));
    assert_eq!(global["oauthAccount"]["accountUuid"], json!(UUID_B));
    assert!(global.get("projects").is_none());
    // The launcher.
    assert!(f.rc_file().contains("alias claude-b="), "{}", f.rc_file());
    // The login ran once, in this instance, in the terminal the front end chose.
    let logins = f.env.login_invocations();
    assert_eq!(logins.len(), 1);
    assert_eq!(logins[0].config_dir, dir);
    assert_eq!(logins[0].kind, InstanceKind::Linked);
    assert_eq!(logins[0].terminal, LoginTerminal::CurrentTerminal);
    // Registered under the account it logged in with, labelled as asked.
    let login = &report.login;
    assert_eq!(login.run, LoginRun::Ran { exit_code: Some(0) });
    let account = login.account.as_ref().expect("an account");
    assert_eq!(account.uuid.as_str(), UUID_B);
    assert_eq!(account.label, "B");
    assert!(login.new_account);
    assert!(login.warnings.is_empty(), "{:?}", login.warnings);
    let instance = f.registered(&dir).expect("registered");
    assert_eq!(instance.registered_account.unwrap().as_str(), UUID_B);
    assert!(matches!(
        instance.observation.login,
        LoginState::LoggedIn { .. }
    ));
    // Nothing left to fix.
    assert_eq!(f.core.drift(&dir).unwrap().drift_count, 0);
    assert_eq!(report.setup.failed(), 0);
}

#[test]
fn the_front_end_chooses_where_the_login_opens() {
    // The app opens it in a new window on every OS; the CLI in its own terminal.
    let f = fixture();
    f.env.set_os(OsKind::Windows);
    f.env.on_login(|_| Ok(LoginExit { exit_code: Some(1) }));

    f.core
        .add(&AddRequest::new("B", LoginTerminal::NewWindow))
        .unwrap();
    f.core
        .login(&f.path(".claude-b"), LoginTerminal::CurrentTerminal)
        .unwrap();

    let terminals: Vec<LoginTerminal> = f
        .env
        .login_invocations()
        .iter()
        .map(|l| l.terminal)
        .collect();
    assert_eq!(
        terminals,
        vec![LoginTerminal::NewWindow, LoginTerminal::CurrentTerminal]
    );
    assert!(f.path(".mccboard/bin/claude-b.exe").is_file());
    assert!(f.path(".mccboard/bin/claude-b.shim").is_file());
}

#[test]
fn adding_aligns_plugins_with_claude_codes_own_commands() {
    let f = fixture();
    primary_plugin(&f.primary);
    plugin_commands(&f.env, false);
    f.env.login_as(UUID_B, "b@example.com");

    let report = f.core.add(&add("B")).unwrap();

    assert_eq!(report.setup.failed(), 0, "{:?}", report.setup.items);
    let commands: Vec<String> = f
        .env
        .claude_invocations()
        .iter()
        .map(|i| i.args[..2].join(" "))
        .collect();
    assert!(
        commands.contains(&"plugin install".to_owned()),
        "{commands:?}"
    );
    assert!(
        f.path(".claude-b/plugins/installed_plugins.json").is_file(),
        "plugins are installed, never copied or linked"
    );
    assert!(!is_link(&f.path(".claude-b/plugins")));
}

#[test]
fn the_folder_and_label_can_be_chosen() {
    let f = fixture();
    f.env.login_as(UUID_B, "b@example.com");
    let request = AddRequest {
        config_dir: Some(PathBuf::from("~/claude-work")),
        ..add("Work")
    };

    let report = f.core.add(&request).unwrap();

    let dir = f.path("claude-work");
    assert_eq!(report.config_dir, dir);
    assert!(is_link(&dir.join("settings.json")));
    assert_eq!(f.label_of(UUID_B).as_deref(), Some("Work"));
    assert!(
        f.rc_file().contains("alias claude-work="),
        "{}",
        f.rc_file()
    );
    assert!(f.registered(&dir).is_some());
}

#[test]
fn the_default_folder_follows_the_label() {
    let f = fixture();

    assert_eq!(f.core.add_folder("B").unwrap(), f.path(".claude-b"));
    assert_eq!(
        f.core.add_folder("Personal Max").unwrap(),
        f.path(".claude-personal-max")
    );
    let error = f.core.add_folder("??").unwrap_err();
    assert!(matches!(error, Error::NoDefaultFolder { .. }), "{error}");
    assert_eq!(error.code(), "no_default_folder");
}

#[test]
fn a_known_account_keeps_its_label() {
    let f = fixture();
    f.env.login_as(UUID_B, "b@example.com");
    f.core.add(&add("B")).unwrap();
    f.env.login_as(UUID_C, "c@example.com");

    // "B" is taken by another account: the new one gets a suggested label instead.
    let request = AddRequest {
        config_dir: Some(f.path(".claude-x")),
        ..add("B")
    };
    let report = f.core.add(&request).unwrap();

    let account = report.login.account.unwrap();
    assert_eq!(account.uuid.as_str(), UUID_C);
    assert_eq!(account.label, "X");
    assert_eq!(f.label_of(UUID_B).as_deref(), Some("B"));
}

// ---------------------------------------------------------------------------------------------
// Warnings

#[test]
fn logging_in_with_an_account_that_has_another_instance_here_warns() {
    let f = fixture();
    // The user logs in with A, whose primary instance is already here.
    f.env.login_as(UUID_A, "a@example.com");

    let report = f.core.add(&add("B")).unwrap();

    let dir = f.path(".claude-b");
    assert_eq!(
        report.login.warnings,
        vec![LoginWarning::AccountHasOtherInstances {
            account: mccboard_core::AccountUuid::new(UUID_A),
            label: Some("A".to_owned()),
            instances: vec![f.primary.clone()],
        }]
    );
    assert!(!report.login.new_account);
    // A's label is untouched; the instance is registered all the same.
    assert_eq!(f.label_of(UUID_A).as_deref(), Some("A"));
    assert_eq!(
        f.registered(&dir)
            .unwrap()
            .registered_account
            .unwrap()
            .as_str(),
        UUID_A
    );
}

#[test]
fn unknown_items_of_the_primary_are_asked_about_and_linked_once_answered() {
    let f = fixture();
    write(f.primary.join("gsd-core/index.md"), "gsd\n");
    f.env.login_as(UUID_B, "b@example.com");

    let report = f.core.add(&add("B")).unwrap();

    let dir = f.path(".claude-b");
    let asked: Vec<&str> = report
        .sharing_questions
        .iter()
        .map(|q| q.item.as_str())
        .collect();
    assert_eq!(asked, vec!["gsd-core"]);
    assert!(!dir.join("gsd-core").exists());

    f.core
        .answer_sharing("gsd-core", SharingAnswer::Link)
        .unwrap();
    let again = f.core.add(&add("B")).unwrap();

    assert!(again.sharing_questions.is_empty());
    assert!(is_link(&dir.join("gsd-core")));
}

// ---------------------------------------------------------------------------------------------
// Idempotent and resumable

#[test]
fn adding_again_changes_nothing_and_doesnt_log_in_again() {
    let f = fixture();
    f.env.login_as(UUID_B, "b@example.com");
    f.core.add(&add("B")).unwrap();
    let rc = f.rc_file();
    let config = f.home.config_text();

    let again = f.core.add(&add("B")).unwrap();

    assert!(!again.created);
    assert!(again.setup.items.is_empty(), "{:?}", again.setup.items);
    assert_eq!(again.login.run, LoginRun::AlreadyLoggedIn);
    assert_eq!(f.env.login_invocations().len(), 1);
    assert!(again.login.warnings.is_empty());
    assert_eq!(f.rc_file(), rc);
    assert_eq!(f.home.config_text(), config);
    assert_eq!(f.core.registry().unwrap().instances.len(), 2);
}

#[test]
fn adding_again_opens_the_login_when_the_credentials_say_it_is_needed() {
    let f = fixture();
    f.env.login_as(UUID_B, "b@example.com");
    f.core.add(&add("B")).unwrap();

    // An expired token is no logout: Claude Code renews it the next time it runs, so, like
    // the app's "Log in", adding again doesn't open the login.
    f.env.advance(jiff::SignedDuration::from_hours(9));
    let again = f.core.add(&add("B")).unwrap();
    assert_eq!(again.login.run, LoginRun::AlreadyLoggedIn);
    assert_eq!(f.env.login_invocations().len(), 1);

    // Tokens Claude Code emptied after a dead refresh token are a logout.
    fs::write(
        f.path(".claude-b/.credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"","refreshToken":"","expiresAt":0}}"#,
    )
    .unwrap();
    let again = f.core.add(&add("B")).unwrap();
    assert_eq!(again.login.run, LoginRun::Ran { exit_code: Some(0) });

    // `oauthAccount` without credentials is no login either.
    fs::remove_file(f.path(".claude-b/.credentials.json")).unwrap();
    let again = f.core.add(&add("B")).unwrap();
    assert_eq!(again.login.run, LoginRun::Ran { exit_code: Some(0) });
    assert_eq!(f.env.login_invocations().len(), 3);
}

#[test]
fn on_windows_an_account_without_a_credentials_file_counts_as_logged_in() {
    // Claude Code may keep the credentials in the Windows Credential Manager, where mccboard
    // doesn't look: the account in `.claude.json` is all there is to see.
    let f = fixture();
    f.env.set_os(OsKind::Windows);
    f.home
        .linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");

    let report = f.core.add(&add("B")).unwrap();

    assert_eq!(report.login.run, LoginRun::AlreadyLoggedIn);
    assert!(f.env.login_invocations().is_empty());
    assert_eq!(report.login.account.unwrap().label, "B");
}

#[test]
fn a_login_that_cant_start_leaves_the_instance_ready_and_a_rerun_finishes() {
    let f = fixture();
    f.env
        .on_login(|_| Err(io::Error::new(io::ErrorKind::NotFound, "program not found")));

    let error = f.core.add(&add("B")).unwrap_err();

    assert!(matches!(error, Error::ClaudeNotInstalled), "{error}");
    assert_eq!(error.code(), "claude_not_installed");
    let dir = f.path(".claude-b");
    // Everything before the login is done, and the instance is registered, not logged in.
    assert!(is_link(&dir.join("settings.json")));
    assert!(f.rc_file().contains("alias claude-b="));
    let instance = f.registered(&dir).expect("registered before the login");
    assert!(instance.registered_account.is_none());

    f.env.login_as(UUID_B, "b@example.com");
    let report = f.core.add(&add("B")).unwrap();

    assert!(report.setup.items.is_empty(), "{:?}", report.setup.items);
    assert_eq!(report.login.account.unwrap().label, "B");
    assert_eq!(f.env.login_invocations().len(), 2);
    assert_eq!(
        f.registered(&dir)
            .unwrap()
            .registered_account
            .unwrap()
            .as_str(),
        UUID_B
    );
}

#[test]
fn a_login_closed_without_logging_in_is_reported_and_a_rerun_logs_in() {
    let f = fixture();
    // The terminal was closed: Claude Code exits without writing anything.
    f.env.on_login(|_| Ok(LoginExit { exit_code: Some(1) }));

    let report = f.core.add(&add("B")).unwrap();

    assert_eq!(report.login.run, LoginRun::Ran { exit_code: Some(1) });
    assert!(report.login.account.is_none());
    let dir = f.path(".claude-b");
    assert!(f.registered(&dir).unwrap().registered_account.is_none());

    f.env.login_as(UUID_B, "b@example.com");
    let report = f.core.add(&add("B")).unwrap();

    assert_eq!(report.login.account.unwrap().uuid.as_str(), UUID_B);
}

#[test]
fn without_developer_mode_nothing_is_made_and_a_rerun_completes() {
    let f = fixture();
    f.env.set_developer_mode(false);
    f.env.login_as(UUID_B, "b@example.com");

    let error = f.core.add(&add("B")).unwrap_err();

    assert!(matches!(error, Error::DeveloperModeOff), "{error}");
    let dir = f.path(".claude-b");
    assert!(!dir.exists());
    assert!(f.registered(&dir).is_none());
    assert!(f.env.login_invocations().is_empty());

    f.env.set_developer_mode(true);
    let report = f.core.add(&add("B")).unwrap();

    assert!(is_link(&dir.join("settings.json")));
    assert_eq!(report.login.account.unwrap().uuid.as_str(), UUID_B);
}

#[test]
fn developer_mode_is_needed_only_while_a_file_link_is_missing() {
    let f = fixture();
    f.env.login_as(UUID_B, "b@example.com");
    f.core.add(&add("B")).unwrap();
    f.env.set_developer_mode(false);

    // Every file link is in place: adding again needs none.
    f.core.add(&add("B")).unwrap();

    // One is missing again: it needs Developer Mode.
    fs::remove_file(f.path(".claude-b/settings.json")).unwrap();
    let error = f.core.add(&add("B")).unwrap_err();
    assert!(matches!(error, Error::DeveloperModeOff), "{error}");
}

#[test]
fn a_running_session_in_an_existing_folder_stops_the_add_before_anything_changes() {
    let f = fixture();
    let dir = f.path(".claude-b");
    fs::create_dir_all(&dir).unwrap();
    f.env.add_process(ClaudeProcess {
        pid: 42,
        name: "claude".into(),
        config_dir: ProcessConfigDir::Dir { path: dir.clone() },
        cwd: None,
    });

    let error = f.core.add(&add("B")).unwrap_err();

    assert!(matches!(error, Error::InstanceInUse { .. }), "{error}");
    assert!(!dir.join("settings.json").exists());
    assert!(f.registered(&dir).is_none());
    assert!(f.env.login_invocations().is_empty());
}

#[test]
fn a_launcher_that_cant_be_written_stops_the_add_and_a_rerun_completes() {
    let f = fixture();
    write(
        f.path(".bashrc"),
        "export X=1\n# >>> mccboard launchers >>>\nalias claude-old='claude'\n",
    );
    f.env.login_as(UUID_B, "b@example.com");

    let error = f.core.add(&add("B")).unwrap_err();

    assert!(
        matches!(error, Error::LauncherBlockCorrupt { .. }),
        "{error}"
    );
    assert!(f.env.login_invocations().is_empty());

    write(f.path(".bashrc"), "export X=1\n");
    let report = f.core.add(&add("B")).unwrap();

    let dir = f.path(".claude-b");
    assert!(f.rc_file().contains("alias claude-b="));
    assert!(is_link(&dir.join("settings.json")));
    assert_eq!(report.login.account.unwrap().uuid.as_str(), UUID_B);
}

#[test]
fn a_failed_plugin_install_is_reported_the_login_still_happens_and_a_rerun_installs_it() {
    let f = fixture();
    primary_plugin(&f.primary);
    plugin_commands(&f.env, true);
    f.env.login_as(UUID_B, "b@example.com");

    let report = f.core.add(&add("B")).unwrap();

    let failed = report
        .setup
        .items
        .iter()
        .find(|i| i.id == "plugin:review@tools")
        .expect("the plugin item");
    assert!(
        matches!(&failed.outcome, RepairOutcome::Failed { reason } if reason.contains("network down")),
        "{:?}",
        failed.outcome
    );
    assert!(report.login.account.is_some());

    plugin_commands(&f.env, false);
    let again = f.core.add(&add("B")).unwrap();

    assert_eq!(again.setup.failed(), 0);
    assert_eq!(
        again
            .setup
            .items
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        vec!["plugin:review@tools"]
    );
    assert_eq!(f.env.login_invocations().len(), 1);
}

#[test]
fn adding_without_login_stops_after_the_launcher() {
    let f = fixture();
    f.env.login_as(UUID_B, "b@example.com");
    let request = AddRequest {
        login: false,
        ..add("B")
    };

    let report = f.core.add(&request).unwrap();

    assert_eq!(report.login.run, LoginRun::NotRequested);
    assert!(report.login.account.is_none());
    assert!(f.env.login_invocations().is_empty());
    let dir = f.path(".claude-b");
    assert!(is_link(&dir.join("settings.json")));
    assert!(f.rc_file().contains("alias claude-b="));
    assert!(f.registered(&dir).is_some());
}

#[test]
fn the_label_asked_for_waits_in_the_config_for_a_later_login() {
    let f = fixture();
    let request = AddRequest {
        login: false,
        ..add("Personal Max")
    };
    f.core.add(&request).unwrap();
    let dir = f.path(".claude-personal-max");
    let config = f.home.config_text().unwrap();
    assert!(config.contains(r#"label = "Personal Max""#), "{config}");

    f.env.login_as(UUID_B, "b@example.com");
    let report = login(&f, &dir).unwrap();

    let account = report.account.unwrap();
    assert_eq!(account.uuid.as_str(), UUID_B);
    assert_eq!(account.label, "Personal Max");
    assert!(report.new_account);
    // Used once the account is observed, then forgotten.
    let config = f.home.config_text().unwrap();
    assert_eq!(config.matches("label = ").count(), 2, "{config}");
}

#[test]
fn the_label_asked_for_survives_a_login_that_couldnt_start() {
    let f = fixture();
    f.env
        .on_login(|_| Err(io::Error::new(io::ErrorKind::NotFound, "program not found")));
    f.core.add(&add("Work")).unwrap_err();

    f.env.login_as(UUID_B, "b@example.com");
    let report = login(&f, &f.path(".claude-work")).unwrap();

    assert_eq!(report.account.unwrap().label, "Work");
}

#[test]
fn an_instance_registered_by_hand_later_gets_the_label_asked_for_too() {
    let f = fixture();
    let request = AddRequest {
        login: false,
        ..add("Work")
    };
    f.core.add(&request).unwrap();
    let dir = f.path(".claude-work");
    // Logged in outside mccboard (the launcher), then registered again.
    f.home
        .linked(".claude-work")
        .logged_in_as(UUID_B, "b@example.com");

    let registration = f.core.register(&dir).unwrap();

    assert_eq!(registration.new_account.unwrap().label, "Work");
    assert_eq!(f.label_of(UUID_B).as_deref(), Some("Work"));
}

// ---------------------------------------------------------------------------------------------
// Refusals

#[test]
fn the_primary_instance_and_odd_paths_are_refused() {
    let f = fixture();
    let primary = AddRequest {
        config_dir: Some(PathBuf::from("~/.claude")),
        ..add("B")
    };
    assert!(matches!(
        f.core.add(&primary).unwrap_err(),
        Error::PrimaryInstance { .. }
    ));

    write(f.path("a-file"), "x");
    let file = AddRequest {
        config_dir: Some(f.path("a-file")),
        ..add("B")
    };
    assert!(matches!(
        f.core.add(&file).unwrap_err(),
        Error::NotADirectory { .. }
    ));

    let relative = AddRequest {
        config_dir: Some(PathBuf::from("relative/dir")),
        ..add("B")
    };
    assert!(matches!(
        f.core.add(&relative).unwrap_err(),
        Error::RelativePath { .. }
    ));

    assert!(matches!(
        f.core.add(&add("  ")).unwrap_err(),
        Error::InvalidLabel { .. }
    ));

    let desktop = AddRequest {
        config_dir: Some(f.path("AppData/Roaming/Claude/code")),
        ..add("B")
    };
    assert!(matches!(
        f.core.add(&desktop).unwrap_err(),
        Error::DesktopAppFolder { .. }
    ));
    assert!(!f.path("AppData").exists());
    assert!(f.env.login_invocations().is_empty());
}

#[test]
fn protected_folders_are_refused_before_anything_changes() {
    let f = fixture();
    let data = f.path("data/b");
    fs::create_dir_all(&data).unwrap();
    link_dir(&data, &f.path(".claude-b"));
    link_dir(&f.primary, &f.path("via"));
    let outside_home = f.home.path().parent().unwrap().to_path_buf();
    let config = f.home.config_text();

    let refused = [
        // The home itself, and a folder holding it.
        (PathBuf::from("~"), "folder_not_addable"),
        (outside_home, "folder_not_addable"),
        // mccboard's own files, and inside them.
        (PathBuf::from("~/.mccboard"), "folder_not_addable"),
        (PathBuf::from("~/.mccboard/b"), "folder_not_addable"),
        // Inside the primary instance, spelled directly or reached through a link.
        (PathBuf::from("~/.claude/b"), "folder_not_addable"),
        (PathBuf::from("~/via/b"), "folder_not_addable"),
        // A link elsewhere: never followed.
        (PathBuf::from("~/.claude-b"), "folder_not_addable"),
        // A link to the primary instance is the primary instance.
        (PathBuf::from("~/via"), "primary_instance"),
    ];
    for (dir, code) in refused {
        let request = AddRequest {
            config_dir: Some(dir.clone()),
            ..add("B")
        };
        let error = f.core.add(&request).unwrap_err();
        assert_eq!(error.code(), code, "{}: {error}", dir.display());
    }

    assert!(!f.path(".mccboard/b").exists());
    assert!(!f.primary.join("b").exists());
    assert_eq!(fs::read_dir(&data).unwrap().count(), 0);
    assert!(!is_link(&f.primary.join("settings.json")));
    assert_eq!(f.home.config_text(), config);
    assert!(f.env.login_invocations().is_empty());
}

#[test]
fn a_folder_that_belongs_to_another_account_is_refused() {
    let f = fixture();
    f.env.login_as(UUID_B, "b@example.com");
    f.core.add(&add("Work")).unwrap();
    let config = f.home.config_text();

    // Another label for the same folder, chosen or derived ("work!" also gives .claude-work).
    let chosen = AddRequest {
        config_dir: Some(PathBuf::from("~/.claude-work")),
        ..add("C")
    };
    for request in [chosen, add("work!")] {
        let error = f.core.add(&request).unwrap_err();
        assert!(
            matches!(&error, Error::FolderHasAnotherAccount { label, .. } if label == "Work"),
            "{error}"
        );
        assert_eq!(error.code(), "folder_has_another_account");
        assert!(error.to_string().contains("\"Work\""), "{error}");
        assert_eq!(error.detail(), Some("Work"));
    }
    assert_eq!(f.home.config_text(), config);
    assert_eq!(f.env.login_invocations().len(), 1);

    // The same label in another spelling is the same account: adding again is fine.
    f.core.add(&add("work")).unwrap();
}

#[test]
fn a_folder_logged_in_with_a_known_account_is_refused_for_another_label() {
    let f = fixture();
    f.home
        .linked(".claude-x")
        .logged_in_as(UUID_A, "a@example.com");
    let request = AddRequest {
        config_dir: Some(f.path(".claude-x")),
        ..add("B")
    };

    let error = f.core.add(&request).unwrap_err();

    assert!(
        matches!(&error, Error::FolderHasAnotherAccount { label, .. } if label == "A"),
        "{error}"
    );
    assert!(f.registered(&f.path(".claude-x")).is_none());
}

#[test]
fn a_link_to_the_primary_folder_is_the_primary_instance_and_never_relinked() {
    let f = fixture();
    let alias = f.path(".claude-alias");
    link_dir(&f.primary, &alias);

    assert_eq!(f.core.kind_of(&alias), InstanceKind::Primary);
    assert!(
        f.core.scan().unwrap().iter().all(|c| c.config_dir != alias),
        "an alias of the primary isn't proposed"
    );
    let registration = f.core.register(&alias).unwrap();
    assert_eq!(registration.instance.kind, InstanceKind::Primary);
    let report = f.core.repair(&alias, None).unwrap();

    assert!(report.items.is_empty(), "{:?}", report.items);
    assert_eq!(f.core.drift(&alias).unwrap().drift_count, 0);
    // The primary's own items stay real: nothing was linked to itself.
    assert!(!is_link(&f.primary.join("settings.json")));
    assert!(!is_link(&f.primary.join("agents")));
    assert!(!f.rc_file().contains(".claude-alias"), "{}", f.rc_file());
    // Its login is the primary's own: Claude Code's default folder.
    login(&f, &alias).unwrap();
    let logins = f.env.login_invocations();
    assert_eq!(logins[0].kind, InstanceKind::Primary);
    assert_eq!(logins[0].config_dir, f.primary);
}

// ---------------------------------------------------------------------------------------------
// Log in on its own

#[test]
fn logging_in_a_registered_instance_records_its_account() {
    let f = fixture();
    let linked = f.home.linked(".claude-b");
    f.core.register(&linked.dir).unwrap();
    f.env.login_as(UUID_B, "b@example.com");

    let report = login(&f, &linked.dir).unwrap();

    assert_eq!(report.config_dir, linked.dir);
    assert_eq!(report.run, LoginRun::Ran { exit_code: Some(0) });
    assert_eq!(report.account.unwrap().uuid.as_str(), UUID_B);
    assert!(report.new_account);
    assert!(matches!(
        report.instance.observation.login,
        LoginState::LoggedIn { .. }
    ));
    assert_eq!(
        f.registered(&linked.dir)
            .unwrap()
            .registered_account
            .unwrap()
            .as_str(),
        UUID_B
    );
    // The launcher follows the account.
    assert!(f.rc_file().contains("alias claude-b="));
}

#[test]
fn logging_in_again_an_expired_instance_runs_the_login_even_with_an_account() {
    let f = fixture();
    let linked = f.home.linked(".claude-b");
    linked
        .logged_in_as(UUID_B, "b@example.com")
        .credentials_expiring(common::now() - jiff::SignedDuration::from_hours(1));
    f.core.register(&linked.dir).unwrap();
    f.env.login_as(UUID_B, "b@example.com");

    let report = login(&f, &linked.dir).unwrap();

    assert_eq!(f.env.login_invocations().len(), 1);
    assert!(!report.new_account);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(matches!(
        report.instance.observation.login,
        LoginState::LoggedIn { .. }
    ));
}

#[test]
fn logging_in_with_an_account_that_has_another_instance_warns() {
    let f = fixture();
    let linked = f.home.linked(".claude-b");
    f.core.register(&linked.dir).unwrap();
    f.env.login_as(UUID_A, "a@example.com");

    let report = login(&f, &linked.dir).unwrap();

    assert!(matches!(
        &report.warnings[..],
        [LoginWarning::AccountHasOtherInstances { instances, .. }] if instances == &vec![f.primary.clone()]
    ));
}

#[test]
fn logging_in_an_unregistered_folder_is_refused() {
    let f = fixture();
    let linked = f.home.linked(".claude-b");

    let error = login(&f, &linked.dir).unwrap_err();

    assert!(matches!(error, Error::NotRegistered { .. }), "{error}");
    assert!(f.env.login_invocations().is_empty());
}

#[test]
fn the_primary_instance_logs_in_as_itself() {
    let f = fixture();
    f.env.login_as(UUID_A, "a@example.com");

    let report = login(&f, &f.primary).unwrap();

    let logins = f.env.login_invocations();
    assert_eq!(logins[0].kind, InstanceKind::Primary);
    assert_eq!(logins[0].config_dir, f.primary);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    // The primary's global config lives next to its folder, and keeps its other keys.
    let global = read_json(f.path(".claude.json"));
    assert_eq!(global["diffTool"], json!("auto"));
    assert_eq!(global["oauthAccount"]["accountUuid"], json!(UUID_A));
}

#[test]
fn logging_in_an_instance_whose_folder_is_gone_says_so() {
    let f = fixture();
    let linked = f.home.linked(".claude-b");
    f.core.register(&linked.dir).unwrap();
    fs::remove_dir(&linked.dir).unwrap();

    let error = login(&f, &linked.dir).unwrap_err();

    assert!(
        matches!(error, Error::InstanceFolderMissing { .. }),
        "{error}"
    );
    assert_eq!(error.code(), "instance_folder_missing");
    assert_eq!(error.path(), Some(linked.dir.as_path()));
    assert!(f.env.login_invocations().is_empty());
}

#[test]
fn a_login_that_needs_a_terminal_says_so() {
    let f = fixture();
    let linked = f.home.linked(".claude-b");
    f.core.register(&linked.dir).unwrap();
    f.env
        .on_login(|_| Err(io::Error::new(io::ErrorKind::Unsupported, "no terminal")));

    let error = login(&f, &linked.dir).unwrap_err();

    assert!(matches!(error, Error::LoginNeedsTerminal { .. }), "{error}");
    assert_eq!(error.code(), "login_needs_terminal");
    assert!(error.to_string().contains("mccboard login"), "{error}");
}

// ---------------------------------------------------------------------------------------------
// The first login on a machine where Claude Code never ran

/// A home where Claude Code is installed but never ran: no `~/.claude`, no `~/.claude.json`.
fn bare() -> (TempHome, Core, FakeEnvironment) {
    let home = TempHome::new();
    let (core, env) = home.core();
    (home, core, env)
}

#[test]
fn the_primary_instance_logs_in_before_its_folder_exists_and_is_registered() {
    let (home, core, env) = bare();
    let primary = home.path().join(".claude");
    // Claude Code makes its folder on its first login.
    env.login_as(UUID_A, "a@example.com");

    let report = core.login_primary(LoginTerminal::CurrentTerminal).unwrap();

    let logins = env.login_invocations();
    assert_eq!(logins.len(), 1);
    assert_eq!(logins[0].kind, InstanceKind::Primary);
    assert_eq!(logins[0].config_dir, primary);
    assert_eq!(logins[0].home, home.path());
    assert_eq!(report.run, LoginRun::Ran { exit_code: Some(0) });
    assert_eq!(report.config_dir, primary);
    assert_eq!(report.account.as_ref().unwrap().label, "A");
    assert!(report.new_account);
    let registry = core.registry().unwrap();
    assert_eq!(registry.instances.len(), 1);
    assert_eq!(registry.instances[0].config_dir, primary);
    assert_eq!(registry.instances[0].kind, InstanceKind::Primary);
    assert_eq!(registry.accounts[0].uuid.as_str(), UUID_A);
}

#[test]
fn a_first_login_that_logs_nothing_in_registers_nothing_and_mccboard_makes_no_folder() {
    let (home, core, env) = bare();

    // The fake login is closed without logging in: Claude Code makes nothing.
    let report = core.login_primary(LoginTerminal::CurrentTerminal).unwrap();

    assert_eq!(env.login_invocations().len(), 1);
    assert_eq!(report.run, LoginRun::Ran { exit_code: Some(1) });
    assert_eq!(report.account, None);
    assert!(!home.path().join(".claude").exists());
    assert!(core.registry().unwrap().instances.is_empty());
}

#[test]
fn a_first_login_that_makes_the_folder_without_an_account_registers_nothing() {
    let (home, core, env) = bare();
    env.on_login(|login| {
        fs::create_dir(&login.config_dir)?;
        Ok(LoginExit { exit_code: Some(1) })
    });

    let report = core.login_primary(LoginTerminal::CurrentTerminal).unwrap();

    assert_eq!(report.account, None);
    assert!(home.path().join(".claude").is_dir());
    assert!(core.registry().unwrap().instances.is_empty());
}

#[test]
fn the_primary_instance_registered_already_logs_in_as_login_does() {
    let f = fixture();
    f.env.login_as(UUID_A, "a@example.com");

    let report = f
        .core
        .login_primary(LoginTerminal::CurrentTerminal)
        .unwrap();

    assert_eq!(report.config_dir, f.primary);
    assert_eq!(report.account.as_ref().unwrap().label, "A");
    assert!(!report.new_account);
    assert_eq!(f.core.registry().unwrap().instances.len(), 1);
}

#[test]
fn the_primary_instance_whose_folder_is_gone_logs_in_and_reports_no_account_until_it_is_back() {
    let f = fixture();
    fs::remove_dir_all(&f.primary).unwrap();

    let report = login(&f, &f.primary).unwrap();

    assert_eq!(f.env.login_invocations().len(), 1);
    assert_eq!(report.account, None);
    assert!(!f.primary.exists());
    // Still registered, with its folder missing.
    assert!(
        report
            .instance
            .observation
            .warnings
            .contains(&mccboard_core::Warning::ConfigDirMissing)
    );
    assert!(f.registered(&f.primary).is_some());
}

#[test]
fn a_primary_folder_that_is_a_file_is_refused_before_the_login() {
    let (home, core, env) = bare();
    fs::write(home.path().join(".claude"), "not a folder").unwrap();

    let error = core
        .login_primary(LoginTerminal::CurrentTerminal)
        .unwrap_err();

    assert!(matches!(error, Error::NotADirectory { .. }), "{error}");
    assert!(env.login_invocations().is_empty());
}

#[test]
fn login_reaches_the_first_login_of_the_primary_instance_when_it_is_not_registered() {
    for spelling in ["~/.claude", "absolute"] {
        let (home, core, env) = bare();
        let primary = home.path().join(".claude");
        let path = match spelling {
            "~/.claude" => PathBuf::from("~/.claude"),
            _ => primary.clone(),
        };
        env.login_as(UUID_A, "a@example.com");

        let report = login_with(&core, &path).unwrap();

        assert_eq!(report.config_dir, primary, "{spelling}");
        assert_eq!(report.account.as_ref().unwrap().label, "A", "{spelling}");
        assert_eq!(env.login_invocations()[0].kind, InstanceKind::Primary);
        assert_eq!(core.registry().unwrap().instances[0].config_dir, primary);
    }
}

#[test]
fn login_of_the_missing_primary_instance_that_logs_nothing_in_reports_no_account() {
    let (home, core, env) = bare();

    let report = login_with(&core, &home.path().join(".claude")).unwrap();

    assert_eq!(env.login_invocations().len(), 1);
    assert_eq!(report.account, None);
    assert!(!home.path().join(".claude").exists());
    assert!(core.registry().unwrap().instances.is_empty());
}

#[test]
fn a_linked_instance_whose_folder_goes_away_during_its_login_is_still_an_error() {
    let f = fixture();
    let linked = f.home.linked(".claude-b");
    f.core.register(&linked.dir).unwrap();
    f.env.on_login(|login| {
        fs::remove_dir_all(&login.config_dir)?;
        Ok(LoginExit { exit_code: Some(0) })
    });

    let error = login(&f, &linked.dir).unwrap_err();

    assert!(matches!(error, Error::NotADirectory { .. }), "{error}");
}

#[test]
fn the_os_the_core_works_on_is_the_environments() {
    let (_home, core, env) = bare();
    assert_eq!(core.os(), OsKind::Linux);

    env.set_os(OsKind::Windows);

    assert_eq!(core.os(), OsKind::Windows);
}

/// A login from the CLI, in a core with no fixture.
fn login_with(core: &Core, dir: &Path) -> Result<mccboard_core::LoginReport, Error> {
    core.login(dir, LoginTerminal::CurrentTerminal)
}

// ---------------------------------------------------------------------------------------------
// Token rules

#[test]
fn mccboard_never_handles_the_credentials_the_login_writes() {
    let f = fixture();
    f.env.login_as(UUID_B, "b@example.com");

    let report = f.core.add(&add("B")).unwrap();

    let out = serde_json::to_string(&report).unwrap();
    assert!(!out.contains(FAKE_LOGIN_TOKEN));
    for (path, text) in f.home.mccboard_files() {
        assert!(!text.contains(FAKE_LOGIN_TOKEN), "{}", path.display());
    }
    // Only the login (the fake playing Claude Code) wrote the credentials file.
    let credentials = f.path(".claude-b/.credentials.json");
    assert!(
        fs::read_to_string(credentials)
            .unwrap()
            .contains(FAKE_LOGIN_TOKEN)
    );
}
