//! Opening a desktop profile (spec #34, ticket #36): the app found from its MSIX package at
//! every launch, the data folder created first, `--user-data-dir` and `CLAUDE_CONFIG_DIR`
//! passed, the other `CLAUDE*`/`ANTHROPIC*` variables stripped, no console; the default
//! desktop profile opened with neither; the typed errors; which profiles run. Through the fake
//! environment port, on temporary homes.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use common::{FIXTURE_TOKEN, TempHome, UUID_A, UUID_B, UUID_C, desktop_process, user_data_dir};
use mccboard_core::testing::{FIRST_FAKE_DESKTOP_PID, FakeEnvironment};
use mccboard_core::{
    AccountUuid, Core, DesktopPackage, DesktopProfile, Error, OsKind, PACKAGE_FAMILY,
    ProcessListing,
};

fn uuid(s: &str) -> AccountUuid {
    AccountUuid::new(s)
}

const FULL_NAME: &str = "Claude_1.0.2342.0_x64__pzs8sxrjxfjjc";

/// A Windows home with the primary instance registered as A, `~/.claude-b` registered as B
/// with a desktop profile in `~/profiles/b` (not created yet), and the app installed.
fn setup() -> (TempHome, Core, FakeEnvironment) {
    let home = TempHome::new();
    let (core, env) = home.core();
    env.set_os(OsKind::Windows);
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    core.register(&primary.dir).unwrap();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    core.register(&b.dir).unwrap();
    core.add_desktop_profile(&uuid(UUID_B), Some(&folder_b(&home)))
        .unwrap();
    let package = install(&home, FULL_NAME);
    env.set_desktop_app_packages(vec![package]);
    (home, core, env)
}

fn folder_b(home: &TempHome) -> PathBuf {
    home.path().join("profiles").join("b")
}

/// A package install folder with `app\Claude.exe` in it.
fn install(home: &TempHome, full_name: &str) -> DesktopPackage {
    let install_dir = home.path().join("WindowsApps").join(full_name);
    fs::create_dir_all(install_dir.join("app")).unwrap();
    fs::write(install_dir.join("app").join("Claude.exe"), "MZ fake Claude").unwrap();
    DesktopPackage {
        full_name: full_name.to_owned(),
        install_dir,
    }
}

fn program(package: &DesktopPackage) -> PathBuf {
    package.install_dir.join("app").join("Claude.exe")
}

fn profile<'a>(profiles: &'a [DesktopProfile], account: &str) -> &'a DesktopProfile {
    profiles
        .iter()
        .find(|p| !p.default && p.account.as_ref().map(AccountUuid::as_str) == Some(account))
        .unwrap()
}

// ---------------------------------------------------------------------------------------------
// Opening a profile

#[test]
fn a_profile_opens_with_its_folder_its_instance_and_no_console() {
    let (home, core, env) = setup();
    let folder = folder_b(&home);
    let existed_at_launch = Arc::new(Mutex::new(Vec::new()));
    let seen = existed_at_launch.clone();
    let checked = folder.clone();
    env.on_desktop_launch(move |_| {
        seen.lock().unwrap().push(checked.is_dir());
        Ok(77)
    });

    let opening = core.open_desktop_profile(&uuid(UUID_B)).unwrap();

    let launches = env.desktop_launches();
    assert_eq!(launches.len(), 1);
    let launch = &launches[0];
    let package = install(&home, FULL_NAME);
    assert_eq!(launch.program, program(&package));
    assert_eq!(
        launch.args,
        [format!("--user-data-dir={}", folder.display())]
    );
    assert_eq!(
        launch.set_env,
        [(
            "CLAUDE_CONFIG_DIR".to_owned(),
            home.path().join(".claude-b")
        )]
    );
    assert_eq!(launch.remove_env_prefixes, ["CLAUDE", "ANTHROPIC"]);
    assert!(!launch.console_window);
    assert_eq!(
        *existed_at_launch.lock().unwrap(),
        [true],
        "the folder exists before the app starts"
    );

    assert!(!opening.default);
    assert_eq!(opening.account, Some(uuid(UUID_B)));
    assert_eq!(opening.label.as_deref(), Some("B"));
    assert_eq!(opening.program, program(&package));
    assert_eq!(opening.data_folder, Some(folder.clone()));
    assert_eq!(opening.instance, Some(home.path().join(".claude-b")));
    assert!(opening.created_folder);
    assert_eq!(opening.pid, 77);
    assert!(folder.is_dir());
    assert_eq!(
        fs::read_dir(&folder).unwrap().count(),
        0,
        "mccboard writes nothing inside"
    );
}

#[test]
fn an_opening_tells_whether_the_profile_still_needs_its_first_sign_in() {
    let (home, core, _env) = setup();

    let first = core.open_desktop_profile(&uuid(UUID_B)).unwrap();
    assert!(
        first.needs_sign_in,
        "a new data folder: nobody signed in yet"
    );

    fs::create_dir_all(
        folder_b(&home)
            .join("claude-code-sessions")
            .join(UUID_B)
            .join("99999999-9999-4999-8999-999999999999"),
    )
    .unwrap();
    let signed_in = core.open_desktop_profile(&uuid(UUID_B)).unwrap();
    assert!(!signed_in.needs_sign_in);

    let default = core.open_default_desktop_profile().unwrap();
    assert!(!default.needs_sign_in, "the default profile has no guide");
}

#[test]
fn opening_again_launches_again_without_waiting_and_keeps_the_folder() {
    let (home, core, env) = setup();
    let folder = folder_b(&home);
    fs::create_dir_all(folder.join("Local Storage")).unwrap();

    let first = core.open_desktop_profile(&uuid(UUID_B)).unwrap();
    let second = core.open_desktop_profile(&uuid(UUID_B)).unwrap();

    assert!(!first.created_folder);
    assert!(!second.created_folder);
    assert_eq!(first.pid, FIRST_FAKE_DESKTOP_PID);
    assert_eq!(second.pid, FIRST_FAKE_DESKTOP_PID + 1);
    assert_eq!(env.desktop_launches().len(), 2);
    assert!(folder.join("Local Storage").is_dir());
}

#[test]
fn the_app_is_looked_up_at_every_launch_and_the_newest_package_wins() {
    let (home, core, env) = setup();
    // Adding the profile looked the app up for its shortcut's icon.
    let before = env.desktop_app_lookups().len();
    core.open_desktop_profile(&uuid(UUID_B)).unwrap();

    // An update installs a newer version beside the old one, then the old one goes.
    let old = install(&home, FULL_NAME);
    let new = install(&home, "Claude_1.0.10000.0_x64__pzs8sxrjxfjjc");
    env.set_desktop_app_packages(vec![old.clone(), new.clone()]);
    let during = core.open_desktop_profile(&uuid(UUID_B)).unwrap();
    env.set_desktop_app_packages(vec![new.clone()]);
    fs::remove_dir_all(&old.install_dir).unwrap();
    let after = core.open_desktop_profile(&uuid(UUID_B)).unwrap();

    assert_eq!(during.program, program(&new));
    assert_eq!(after.program, program(&new));
    let lookups = env.desktop_app_lookups().split_off(before);
    assert_eq!(lookups.len(), 3, "one lookup per launch");
    assert!(lookups.iter().all(|families| families[0] == PACKAGE_FAMILY));
}

#[test]
fn a_package_without_its_program_is_skipped() {
    let (home, core, env) = setup();
    let good = install(&home, FULL_NAME);
    let broken = install(&home, "Claude_9.0.0.0_x64__pzs8sxrjxfjjc");
    fs::remove_file(program(&broken)).unwrap();
    env.set_desktop_app_packages(vec![broken, good.clone()]);

    let opening = core.open_desktop_profile(&uuid(UUID_B)).unwrap();

    assert_eq!(opening.program, program(&good));
}

#[test]
fn other_claude_package_families_are_looked_up_from_their_data_folders() {
    let (home, core, env) = setup();
    let packages = home.path().join("AppData").join("Local").join("Packages");
    for name in [
        "Claude_pzs8sxrjxfjjc",
        "Claude_otherpublisher1",
        "Microsoft.WindowsTerminal_8wekyb3d8bbwe",
    ] {
        fs::create_dir_all(packages.join(name)).unwrap();
    }
    let before = env.desktop_app_lookups().len();

    core.open_desktop_profile(&uuid(UUID_B)).unwrap();

    assert_eq!(
        env.desktop_app_lookups().split_off(before),
        [vec![
            PACKAGE_FAMILY.to_owned(),
            "Claude_otherpublisher1".to_owned()
        ]]
    );
}

// ---------------------------------------------------------------------------------------------
// The default desktop profile

#[test]
fn the_default_profile_opens_with_neither_folder_nor_variable() {
    let (home, core, env) = setup();

    let by_label = core.open_desktop_profile(&uuid(UUID_A)).unwrap();
    let direct = core.open_default_desktop_profile().unwrap();

    for opening in [&by_label, &direct] {
        assert!(opening.default);
        assert_eq!(opening.account, Some(uuid(UUID_A)));
        assert_eq!(opening.label.as_deref(), Some("A"));
        assert_eq!(opening.data_folder, None);
        assert_eq!(opening.instance, None);
        assert!(!opening.created_folder);
    }
    let launches = env.desktop_launches();
    assert_eq!(launches.len(), 2);
    for launch in launches {
        assert_eq!(launch.program, program(&install(&home, FULL_NAME)));
        assert!(launch.args.is_empty());
        assert!(launch.set_env.is_empty());
        // Inherited CLAUDE_CONFIG_DIR and tokens are still removed.
        assert_eq!(launch.remove_env_prefixes, ["CLAUDE", "ANTHROPIC"]);
        assert!(!launch.console_window);
    }
    assert!(
        !home
            .path()
            .join("AppData")
            .join("Roaming")
            .join("Claude")
            .exists()
    );
}

#[test]
fn the_default_profile_opens_with_nothing_registered() {
    let home = TempHome::new();
    let (core, env) = home.core();
    env.set_os(OsKind::Windows);
    env.set_desktop_app_packages(vec![install(&home, FULL_NAME)]);

    let opening = core.open_default_desktop_profile().unwrap();

    assert!(opening.default);
    assert_eq!(opening.account, None);
    assert_eq!(home.config_text(), None, "opening writes nothing");
}

// ---------------------------------------------------------------------------------------------
// Typed errors

/// Nothing was launched and the profile's folder wasn't created.
fn assert_nothing_done(home: &TempHome, env: &FakeEnvironment) {
    assert!(env.desktop_launches().is_empty());
    assert!(!folder_b(home).exists());
}

#[test]
fn off_windows_nothing_opens() {
    let (home, core, env) = setup();
    env.set_os(OsKind::Linux);
    let before = env.desktop_app_lookups().len();

    for result in [
        core.open_desktop_profile(&uuid(UUID_B)),
        core.open_default_desktop_profile(),
    ] {
        let error = result.unwrap_err();
        assert!(matches!(error, Error::DesktopAppNeedsWindows), "{error:?}");
        assert_eq!(error.code(), "desktop_app_needs_windows");
    }
    assert_eq!(env.desktop_app_lookups().len(), before);
    assert_nothing_done(&home, &env);
}

#[test]
fn an_app_not_installed_is_reported() {
    let (home, core, env) = setup();
    env.set_desktop_app_packages(Vec::new());

    let error = core.open_desktop_profile(&uuid(UUID_B)).unwrap_err();

    assert!(matches!(error, Error::DesktopAppNotInstalled), "{error:?}");
    assert_eq!(error.code(), "desktop_app_not_installed");
    assert_nothing_done(&home, &env);
}

#[test]
fn the_old_installer_is_not_the_msix_app() {
    let (home, core, env) = setup();
    env.set_desktop_app_packages(Vec::new());
    let squirrel = home
        .path()
        .join("AppData")
        .join("Local")
        .join("AnthropicClaude");
    fs::create_dir_all(squirrel.join("app-0.9.0")).unwrap();
    fs::write(squirrel.join("claude.exe"), "MZ old").unwrap();

    let error = core.open_desktop_profile(&uuid(UUID_B)).unwrap_err();

    assert!(
        matches!(error, Error::DesktopAppNotMsix { .. }),
        "{error:?}"
    );
    assert_eq!(error.code(), "desktop_app_not_msix");
    assert_eq!(error.path(), Some(squirrel.as_path()));
    assert_nothing_done(&home, &env);
}

#[test]
fn a_failing_lookup_or_launch_is_reported_with_its_reason() {
    let (home, core, env) = setup();
    env.set_desktop_app_lookup_error(Some("the packaging API failed"));

    let error = core.open_desktop_profile(&uuid(UUID_B)).unwrap_err();

    assert_eq!(error.code(), "desktop_app_not_opened");
    assert!(error.detail().unwrap().contains("the packaging API failed"));
    assert_nothing_done(&home, &env);

    env.set_desktop_app_lookup_error(None);
    env.on_desktop_launch(|_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "fake: Claude.exe went away",
        ))
    });
    let error = core.open_desktop_profile(&uuid(UUID_B)).unwrap_err();
    assert!(
        matches!(error, Error::DesktopAppNotOpened { .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("Claude.exe went away"));
}

#[test]
fn an_account_without_a_profile_or_unknown_is_refused() {
    let (home, core, env) = setup();
    let c = home.linked(".claude-c");
    c.logged_in_as(UUID_C, "c@example.com");
    core.register(&c.dir).unwrap();

    let error = core.open_desktop_profile(&uuid(UUID_C)).unwrap_err();
    assert_eq!(error.code(), "desktop_profile_not_found");
    let error = core
        .open_desktop_profile(&uuid("44444444-4444-4444-8444-444444444444"))
        .unwrap_err();
    assert_eq!(error.code(), "account_not_found");
    assert!(env.desktop_launches().is_empty());
}

#[test]
fn a_profile_whose_instance_went_away_is_not_opened_on_the_primary_one() {
    let (home, core, env) = setup();
    core.unregister(&home.path().join(".claude-b")).unwrap();

    let error = core.open_desktop_profile(&uuid(UUID_B)).unwrap_err();

    // Its Code tab would fall back to ~/.claude and write B into ~/.claude.json.
    assert!(matches!(error, Error::NoLocalInstance { .. }), "{error:?}");
    assert_nothing_done(&home, &env);
}

#[test]
fn a_hand_edited_data_folder_on_an_instance_is_never_used() {
    let (home, core, env) = setup();
    let instance = home.path().join(".claude-b");
    // Edited as TOML: the core may write a Windows path as a literal string ('C:\…').
    let mut config: toml_edit::DocumentMut = home.config_text().unwrap().parse().unwrap();
    config["desktop_profiles"][0]["data_folder"] = toml_edit::value(instance.to_str().unwrap());
    home.write_config(&config.to_string());
    let profile = profile(&core.desktop_profiles().unwrap(), UUID_B).clone();
    assert_eq!(profile.data_folder, instance, "the edit took");

    let error = core.open_desktop_profile(&uuid(UUID_B)).unwrap_err();

    assert_eq!(error.code(), "desktop_profile_folder_not_allowed");
    assert!(env.desktop_launches().is_empty());
}

#[test]
fn every_new_error_has_its_message_in_both_ui_languages() {
    let locales = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/lib/locales");
    for language in ["en", "it"] {
        let text = fs::read_to_string(locales.join(format!("{language}.json"))).unwrap();
        let messages: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&text).unwrap();
        for code in [
            "desktop_app_needs_windows",
            "desktop_app_not_installed",
            "desktop_app_not_msix",
            "desktop_app_not_opened",
        ] {
            let key = format!("error.{code}");
            assert!(
                messages
                    .get(&key)
                    .and_then(|v| v.as_str())
                    .is_some_and(|m| !m.is_empty()),
                "{key} missing in {language}.json"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Running state

#[test]
fn a_profile_runs_while_any_of_its_processes_does() {
    let (home, core, env) = setup();
    let folder = folder_b(&home);

    let profiles = core.desktop_profiles().unwrap();
    assert!(!profiles[0].running);
    assert!(!profile(&profiles, UUID_B).running);

    // B's main process and a renderer; the default profile's main process.
    env.set_desktop_processes(vec![
        desktop_process(10, &[&user_data_dir(&folder)]),
        desktop_process(11, &["--type=renderer", &user_data_dir(&folder)]),
        desktop_process(20, &[]),
    ]);
    let profiles = core.desktop_profiles().unwrap();
    assert!(profiles[0].running);
    assert!(profile(&profiles, UUID_B).running);

    // Only a utility helper of B left, and a default-profile helper naming the app's folder.
    let own = home.path().join("AppData").join("Roaming").join("Claude");
    env.set_desktop_processes(vec![
        desktop_process(12, &["--type=utility", &user_data_dir(&folder)]),
        desktop_process(21, &["--type=gpu-process", &user_data_dir(&own)]),
    ]);
    let profiles = core.desktop_profiles().unwrap();
    assert!(profiles[0].running);
    assert!(profile(&profiles, UUID_B).running);

    let using = core.desktop_processes_using(&folder).unwrap();
    assert_eq!(using.len(), 1);
    assert_eq!(using[0].pid, 12);
    assert_eq!(using[0].helper.as_deref(), Some("utility"));
    assert!(!using[0].is_main());
}

#[test]
fn claude_code_and_unreadable_processes_run_no_profile() {
    let (home, core, env) = setup();
    let code = ProcessListing {
        pid: 30,
        parent_pid: None,
        name: "claude.exe".into(),
        exe: Some(PathBuf::from(r"C:\Users\me\.local\bin\claude.exe")),
        args: vec!["claude".into(), "--resume".into()],
    };
    env.set_desktop_processes(vec![code, desktop_process(31, &[]).without_args()]);

    let profiles = core.desktop_profiles().unwrap();

    assert!(!profiles[0].running);
    assert!(!profile(&profiles, UUID_B).running);
    assert!(
        core.desktop_processes_using(&folder_b(&home))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn nothing_opened_shows_a_token() {
    let (home, core, _env) = setup();
    let folder = folder_b(&home);
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("config.json"),
        format!(r#"{{"oauth:tokenCache":"{FIXTURE_TOKEN}"}}"#),
    )
    .unwrap();

    let opening = core.open_desktop_profile(&uuid(UUID_B)).unwrap();

    let output = format!("{opening:?}{}", serde_json::to_string(&opening).unwrap());
    assert!(!output.contains("FIXTURE"), "{output}");
}

trait WithoutArgs {
    fn without_args(self) -> Self;
}

impl WithoutArgs for ProcessListing {
    /// The same process, its command line unreadable.
    fn without_args(mut self) -> Self {
        self.args.clear();
        self
    }
}
