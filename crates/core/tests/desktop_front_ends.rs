//! What the front ends need for desktop profiles (spec #34, ticket #39): the accounts that can
//! get a profile, the tray's "Open profile" menu, and the profiles opened when mccboard starts.
//! Through the fake environment port, on temporary homes.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{TempHome, UUID_A, UUID_B, UUID_C, desktop_process, user_data_dir};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    AccountUuid, CLAUDE_ORANGE, Core, DesktopPackage, DesktopProfileUpdate, Language, OsKind,
    RemoveLevel,
};

const UUID_D: &str = "44444444-4444-4444-8444-444444444444";
const FULL_NAME: &str = "Claude_1.0.2342.0_x64__pzs8sxrjxfjjc";

fn uuid(s: &str) -> AccountUuid {
    AccountUuid::new(s)
}

/// A Windows home with the app installed and four accounts: A on the primary instance (the
/// default desktop profile), B with a local instance and a profile in `~/profiles/b`, C with a
/// local instance and no profile, D whose instance was removed.
fn setup() -> (TempHome, Core, FakeEnvironment) {
    let home = TempHome::new();
    let (core, env) = home.core();
    env.set_os(OsKind::Windows);
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    core.register(&primary.dir).unwrap();
    for (dir, account) in [
        (".claude-b", UUID_B),
        (".claude-c", UUID_C),
        (".claude-d", UUID_D),
    ] {
        let instance = home.linked(dir);
        instance.logged_in_as(account, "x@example.com");
        core.register(&instance.dir).unwrap();
    }
    core.remove(&home.path().join(".claude-d"), RemoveLevel::FromMccboard)
        .unwrap();
    core.add_desktop_profile(&uuid(UUID_B), Some(&folder_b(&home)))
        .unwrap();
    env.set_desktop_app_packages(vec![install(&home)]);
    (home, core, env)
}

fn folder_b(home: &TempHome) -> PathBuf {
    home.path().join("profiles").join("b")
}

fn install(home: &TempHome) -> DesktopPackage {
    let install_dir = home.path().join("WindowsApps").join(FULL_NAME);
    fs::create_dir_all(install_dir.join("app")).unwrap();
    fs::write(install_dir.join("app").join("Claude.exe"), "MZ fake Claude").unwrap();
    DesktopPackage {
        full_name: FULL_NAME.to_owned(),
        install_dir,
    }
}

// ---------------------------------------------------------------------------------------------
// Accounts that can get a profile

#[test]
fn only_accounts_with_a_local_instance_and_no_profile_can_get_one() {
    let (_home, core, _env) = setup();

    let candidates = core.desktop_profile_candidates().unwrap();

    let labels: Vec<&str> = candidates.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(
        labels,
        ["C"],
        "not A (default), B (has one), D (no instance)"
    );
}

#[test]
fn a_removed_profile_makes_its_account_a_candidate_again() {
    let (_home, core, _env) = setup();
    core.remove_desktop_profile(&uuid(UUID_B), RemoveLevel::FromMccboard)
        .unwrap();

    let candidates = core.desktop_profile_candidates().unwrap();

    let labels: Vec<&str> = candidates.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, ["B", "C"]);
}

// ---------------------------------------------------------------------------------------------
// The tray's "Open profile" menu

#[test]
fn the_tray_lists_every_profile_default_first_with_its_color() {
    let (_home, core, _env) = setup();

    let menu = core.tray_profiles(Language::English).unwrap();

    assert_eq!(menu.title, "Open profile");
    assert_eq!(menu.profiles.len(), 2);
    let default = &menu.profiles[0];
    assert!(default.default);
    assert_eq!(default.account, Some(uuid(UUID_A)));
    assert_eq!(default.title, "A (default)");
    assert_eq!(default.color, CLAUDE_ORANGE.parse().unwrap());
    let b = &menu.profiles[1];
    assert!(!b.default);
    assert_eq!(b.account, Some(uuid(UUID_B)));
    assert_eq!(b.title, "B");
    let registry = core.registry().unwrap();
    let b_color = &registry
        .accounts
        .iter()
        .find(|a| a.uuid == uuid(UUID_B))
        .unwrap()
        .color;
    assert_eq!(&b.color, b_color);
}

#[test]
fn the_tray_menu_speaks_italian_too() {
    let home = TempHome::new();
    let (core, env) = home.core();
    env.set_os(OsKind::Windows);

    let menu = core.tray_profiles(Language::Italian).unwrap();

    assert_eq!(menu.title, "Apri profilo");
    assert_eq!(menu.profiles.len(), 1);
    assert_eq!(menu.profiles[0].title, "Profilo predefinito");
}

#[test]
fn off_windows_the_tray_offers_no_profile() {
    let (_home, core, env) = setup();
    env.set_os(OsKind::Linux);

    let menu = core.tray_profiles(Language::English).unwrap();

    assert!(menu.profiles.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Opening at mccboard's start

#[test]
fn profiles_set_to_open_at_start_open_and_the_others_dont() {
    let (home, core, env) = setup();
    let c = home.path().join("profiles").join("c");
    core.add_desktop_profile(&uuid(UUID_C), Some(&c)).unwrap();
    core.update_desktop_profile(
        &uuid(UUID_B),
        DesktopProfileUpdate {
            open_at_start: Some(true),
        },
    )
    .unwrap();

    let openings = core.open_desktop_profiles_at_start().unwrap();

    assert_eq!(openings.len(), 1);
    assert_eq!(openings[0].account, uuid(UUID_B));
    assert_eq!(openings[0].label.as_deref(), Some("B"));
    let opening = openings[0].result.as_ref().unwrap();
    assert_eq!(
        opening.data_folder.as_deref(),
        Some(folder_b(&home).as_path())
    );
    let launches = env.desktop_launches();
    assert_eq!(launches.len(), 1);
    assert!(launches[0].args.contains(&user_data_dir(&folder_b(&home))));
    assert!(!c.exists(), "C doesn't open at start");
}

#[test]
fn a_profile_already_running_is_not_opened_again_at_start() {
    let (home, core, env) = setup();
    core.update_desktop_profile(
        &uuid(UUID_B),
        DesktopProfileUpdate {
            open_at_start: Some(true),
        },
    )
    .unwrap();
    env.set_desktop_processes(vec![desktop_process(
        10,
        &[&user_data_dir(&folder_b(&home))],
    )]);

    let openings = core.open_desktop_profiles_at_start().unwrap();

    assert!(openings.is_empty());
    assert!(env.desktop_launches().is_empty());
}

#[test]
fn a_profile_that_cant_open_at_start_is_reported_and_the_others_still_open() {
    let (home, core, env) = setup();
    let c = home.path().join("profiles").join("c");
    core.add_desktop_profile(&uuid(UUID_C), Some(&c)).unwrap();
    for account in [UUID_B, UUID_C] {
        core.update_desktop_profile(
            &uuid(account),
            DesktopProfileUpdate {
                open_at_start: Some(true),
            },
        )
        .unwrap();
    }
    // B's folder is taken by a file: B can't open.
    fs::create_dir_all(folder_b(&home).parent().unwrap()).unwrap();
    fs::write(folder_b(&home), "not a folder").unwrap();

    let openings = core.open_desktop_profiles_at_start().unwrap();

    assert_eq!(openings.len(), 2);
    let b = openings.iter().find(|o| o.account == uuid(UUID_B)).unwrap();
    assert_eq!(b.result.as_ref().unwrap_err().code(), "not_a_directory");
    let c_opening = openings.iter().find(|o| o.account == uuid(UUID_C)).unwrap();
    assert!(c_opening.result.is_ok());
    assert_eq!(env.desktop_launches().len(), 1);
}

#[test]
fn off_windows_nothing_opens_at_start() {
    let (_home, core, env) = setup();
    core.update_desktop_profile(
        &uuid(UUID_B),
        DesktopProfileUpdate {
            open_at_start: Some(true),
        },
    )
    .unwrap();
    env.set_os(OsKind::Linux);

    assert!(core.open_desktop_profiles_at_start().unwrap().is_empty());
    assert!(env.desktop_launches().is_empty());
}
