//! A desktop profile's own taskbar identity (spec #34, ticket #37): its Start-menu shortcut
//! (name, target, arguments, AppUserModelID, icon), kept in step with the profile's label,
//! color and removal; and which running processes get which window identity. Through the
//! fake environment port, on temporary homes; the fake writes a placeholder file for each
//! shortcut.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{TempHome, UUID_A, UUID_B, UUID_C, desktop_process, user_data_dir};
use mccboard_core::testing::{FAKE_MCCBOARD_APP, FakeEnvironment};
use mccboard_core::{
    AccountUpdate, AccountUuid, Color, Core, DesktopPackage, Error, IconCache, Image, OsKind,
    RemoveLevel, ShortcutWarning, app_user_model_id,
};

const FULL_NAME: &str = "Claude_1.0.2342.0_x64__pzs8sxrjxfjjc";

const MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Applications>
    <Application Id="Claude" Executable="app\Claude.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="Claude" Square44x44Logo="Assets\Square44x44Logo.png" />
    </Application>
  </Applications>
</Package>
"#;

fn uuid(s: &str) -> AccountUuid {
    AccountUuid::new(s)
}

fn color(hex: &str) -> Color {
    hex.parse().unwrap()
}

/// A 16×16 logo: Claude's orange with a white middle, on a transparent margin.
fn logo() -> Image {
    let side = 16;
    let mut rgba = vec![0u8; side * side * 4];
    for y in 2..14 {
        for x in 2..14 {
            let white = (5..11).contains(&x) && (5..11).contains(&y);
            let px = if white {
                [255, 255, 255, 255]
            } else {
                [0xd9, 0x77, 0x57, 255]
            };
            let at = (y * side + x) * 4;
            rgba[at..at + 4].copy_from_slice(&px);
        }
    }
    Image::new(side as u32, side as u32, rgba)
}

/// The desktop app installed with its manifest and, when `with_logo`, a plated logo.
fn install(home: &TempHome, with_logo: bool) -> DesktopPackage {
    let install_dir = home.path().join("WindowsApps").join(FULL_NAME);
    fs::create_dir_all(install_dir.join("app")).unwrap();
    fs::write(install_dir.join("app").join("Claude.exe"), "MZ fake Claude").unwrap();
    fs::write(install_dir.join("AppxManifest.xml"), MANIFEST).unwrap();
    if with_logo {
        let assets = install_dir.join("Assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(
            assets.join("Square44x44Logo.targetsize-48.png"),
            logo().to_png(),
        )
        .unwrap();
    }
    DesktopPackage {
        full_name: FULL_NAME.to_owned(),
        install_dir,
    }
}

/// A Windows home with the app installed (its logo included), the primary instance as A,
/// `~/.claude-b` as B and `~/.claude-c` as C; no desktop profile yet.
fn setup() -> (TempHome, Core, FakeEnvironment) {
    let home = TempHome::new();
    let (core, env) = home.core();
    env.set_os(OsKind::Windows);
    env.set_desktop_app_packages(vec![install(&home, true)]);
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    core.register(&primary.dir).unwrap();
    for (dir, account) in [(".claude-b", UUID_B), (".claude-c", UUID_C)] {
        let instance = home.linked(dir);
        instance.logged_in_as(account, "x@example.com");
        core.register(&instance.dir).unwrap();
    }
    (home, core, env)
}

fn folder(home: &TempHome, name: &str) -> PathBuf {
    home.path().join("profiles").join(name)
}

fn add_b(home: &TempHome, core: &Core) {
    core.add_desktop_profile(&uuid(UUID_B), Some(&folder(home, "b")))
        .unwrap();
}

fn color_of(core: &Core, account: &str) -> Color {
    core.registry()
        .unwrap()
        .accounts
        .into_iter()
        .find(|a| a.uuid.as_str() == account)
        .unwrap()
        .color
}

fn app_icon(core: &Core, account: &str, color: &Color) -> PathBuf {
    IconCache::new(&core.paths())
        .icons(&uuid(account), color)
        .app
}

fn lnk_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn adding_a_profile_writes_its_shortcut_with_its_identity_and_colored_icon() {
    let (home, core, env) = setup();

    add_b(&home, &core);

    let written = env.shortcuts_written();
    assert_eq!(written.len(), 1);
    let shortcut = &written[0];
    assert_eq!(shortcut.path, home.start_menu().join("Claude B.lnk"));
    assert_eq!(shortcut.target, PathBuf::from(FAKE_MCCBOARD_APP));
    assert_eq!(shortcut.arguments, format!("--open-profile {UUID_B}"));
    assert_eq!(
        shortcut.app_user_model_id,
        format!("io.github.nunziati.mccboard.profile.{UUID_B}")
    );
    assert_eq!(shortcut.app_user_model_id, app_user_model_id(&uuid(UUID_B)));
    assert_eq!(shortcut.description, "Claude B");
    let icon = app_icon(&core, UUID_B, &color_of(&core, UUID_B));
    assert_eq!(shortcut.icon.as_deref(), Some(icon.as_path()));
    assert!(icon.is_file(), "the icon is drawn and cached");
    assert_eq!(lnk_files(&home.start_menu()), ["Claude B.lnk"]);
}

#[test]
fn syncing_again_changes_nothing() {
    let (home, core, env) = setup();
    add_b(&home, &core);

    let sync = core.sync_desktop_shortcuts().unwrap();

    assert!(sync.written.is_empty() && sync.removed.is_empty());
    assert_eq!(sync.unchanged, [home.start_menu().join("Claude B.lnk")]);
    assert_eq!(env.shortcuts_written().len(), 1);
}

#[test]
fn a_shortcut_deleted_by_hand_is_written_again() {
    let (home, core, env) = setup();
    add_b(&home, &core);
    fs::remove_file(home.start_menu().join("Claude B.lnk")).unwrap();

    let sync = core.sync_desktop_shortcuts().unwrap();

    assert_eq!(sync.written, [home.start_menu().join("Claude B.lnk")]);
    assert_eq!(env.shortcuts_written().len(), 2);
}

#[test]
fn a_new_label_renames_the_shortcut_and_keeps_its_identity() {
    let (home, core, env) = setup();
    add_b(&home, &core);

    core.update_account(
        &uuid(UUID_B),
        AccountUpdate {
            label: Some("Work".into()),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(lnk_files(&home.start_menu()), ["Claude Work.lnk"]);
    let last = env.shortcuts_written().pop().unwrap();
    assert_eq!(last.path, home.start_menu().join("Claude Work.lnk"));
    assert_eq!(last.description, "Claude Work");
    assert_eq!(last.app_user_model_id, app_user_model_id(&uuid(UUID_B)));
    assert_eq!(last.arguments, format!("--open-profile {UUID_B}"));
}

#[test]
fn a_new_color_redraws_the_icon_and_updates_the_shortcut() {
    let (home, core, env) = setup();
    add_b(&home, &core);
    let old_icon = app_icon(&core, UUID_B, &color_of(&core, UUID_B));

    core.update_account(
        &uuid(UUID_B),
        AccountUpdate {
            color: Some(color("#2563eb")),
            ..Default::default()
        },
    )
    .unwrap();

    let new_icon = app_icon(&core, UUID_B, &color("#2563eb"));
    assert!(new_icon.is_file());
    assert!(!old_icon.exists(), "the old color's icon is gone");
    let last = env.shortcuts_written().pop().unwrap();
    assert_eq!(last.path, home.start_menu().join("Claude B.lnk"));
    assert_eq!(last.icon.as_deref(), Some(new_icon.as_path()));
    assert_eq!(env.shortcuts_written().len(), 2);
}

#[test]
fn removing_the_profile_removes_its_shortcut_and_its_icons() {
    let (home, core, _env) = setup();
    add_b(&home, &core);
    core.add_desktop_profile(&uuid(UUID_C), Some(&folder(&home, "c")))
        .unwrap();
    let icons = home.path().join(".mccboard").join("state").join("icons");
    assert!(!lnk_files(&icons).is_empty());

    core.remove_desktop_profile(&uuid(UUID_B), RemoveLevel::FromMccboard)
        .unwrap();

    assert_eq!(lnk_files(&home.start_menu()), ["Claude C.lnk"]);
    assert!(
        lnk_files(&icons)
            .iter()
            .all(|name| !name.starts_with(UUID_B)),
        "{:?}",
        lnk_files(&icons)
    );
}

#[test]
fn off_windows_no_shortcut_is_written() {
    let (home, core, env) = setup();
    env.set_os(OsKind::Linux);

    add_b(&home, &core);
    let sync = core.sync_desktop_shortcuts().unwrap();

    assert_eq!(sync, Default::default());
    assert!(env.shortcuts_written().is_empty());
    assert!(!home.start_menu().exists());
}

#[test]
fn with_no_profile_nothing_is_asked_of_the_system() {
    let (_home, core, env) = setup();
    env.set_start_menu(None);
    env.set_mccboard_app(None);

    let sync = core.sync_desktop_shortcuts().unwrap();

    assert_eq!(sync, Default::default());
}

#[test]
fn a_file_mccboard_didnt_make_is_left_alone() {
    let (home, core, env) = setup();
    fs::create_dir_all(home.start_menu()).unwrap();
    fs::write(home.start_menu().join("Claude B.lnk"), "the user's own").unwrap();

    add_b(&home, &core);
    let sync = core.sync_desktop_shortcuts().unwrap();

    assert!(env.shortcuts_written().is_empty());
    assert_eq!(
        sync.warnings,
        [ShortcutWarning::NameTaken {
            path: home.start_menu().join("Claude B.lnk")
        }]
    );
    assert_eq!(
        fs::read_to_string(home.start_menu().join("Claude B.lnk")).unwrap(),
        "the user's own"
    );
    core.remove_desktop_profile(&uuid(UUID_B), RemoveLevel::FromMccboard)
        .unwrap();
    assert!(home.start_menu().join("Claude B.lnk").is_file());
}

#[test]
fn without_a_logo_the_programs_icon_is_used_and_without_either_mccboards() {
    let (home, core, env) = setup();
    let package = install(&home, false);
    fs::remove_dir_all(package.install_dir.join("Assets")).unwrap();
    env.set_desktop_app_packages(vec![package]);
    env.set_program_icon(Some(logo()));
    add_b(&home, &core);
    let icon = app_icon(&core, UUID_B, &color_of(&core, UUID_B));
    assert_eq!(
        env.shortcuts_written()[0].icon.as_deref(),
        Some(icon.as_path())
    );

    env.set_program_icon(None);
    core.add_desktop_profile(&uuid(UUID_C), Some(&folder(&home, "c")))
        .unwrap();
    let sync = core.sync_desktop_shortcuts().unwrap();

    let c = env
        .shortcuts_written()
        .into_iter()
        .find(|s| s.description == "Claude C")
        .unwrap();
    assert_eq!(c.icon, None);
    assert!(sync.warnings.contains(&ShortcutWarning::NoIcon {
        account: uuid(UUID_C),
        error: "no_icon_source".into()
    }));
}

#[test]
fn a_shortcut_that_cant_be_written_fails_the_call_but_the_profile_is_added() {
    let (home, core, env) = setup();
    env.set_shortcut_error(Some("access denied"));

    let error = core
        .add_desktop_profile(&uuid(UUID_B), Some(&folder(&home, "b")))
        .unwrap_err();

    assert!(
        matches!(&error, Error::ShortcutNotWritten { path, .. } if path == &home.start_menu().join("Claude B.lnk")),
        "{error:?}"
    );
    assert_eq!(error.code(), "shortcut_not_written");
    assert_eq!(
        core.desktop_profiles().unwrap().len(),
        2,
        "added all the same"
    );

    env.set_shortcut_error(None);
    let sync = core.sync_desktop_shortcuts().unwrap();
    assert_eq!(sync.written, [home.start_menu().join("Claude B.lnk")]);
}

#[test]
fn without_the_start_menu_or_the_app_the_error_says_which() {
    let (home, core, env) = setup();
    env.set_start_menu(None);
    let error = core
        .add_desktop_profile(&uuid(UUID_B), Some(&folder(&home, "b")))
        .unwrap_err();
    assert_eq!(error.code(), "start_menu_not_found");

    env.set_start_menu(Some(&home.start_menu()));
    env.set_mccboard_app(None);
    let error = core.sync_desktop_shortcuts().unwrap_err();
    assert_eq!(error.code(), "app_program_not_found");
    assert!(env.shortcuts_written().is_empty());
}

#[test]
fn labels_that_make_the_same_file_name_get_distinct_shortcuts() {
    let (home, core, _env) = setup();
    for (account, label) in [(UUID_B, "x?"), (UUID_C, "x*")] {
        core.update_account(
            &uuid(account),
            AccountUpdate {
                label: Some(label.into()),
                ..Default::default()
            },
        )
        .unwrap();
    }
    add_b(&home, &core);
    core.add_desktop_profile(&uuid(UUID_C), Some(&folder(&home, "c")))
        .unwrap();

    assert_eq!(
        lnk_files(&home.start_menu()),
        ["Claude x_ (2).lnk", "Claude x_.lnk"]
    );
}

#[test]
fn running_profiles_are_the_non_default_ones_with_all_their_processes() {
    let (home, core, env) = setup();
    add_b(&home, &core);
    core.add_desktop_profile(&uuid(UUID_C), Some(&folder(&home, "c")))
        .unwrap();
    let b = user_data_dir(&folder(&home, "b"));
    env.set_desktop_processes(vec![
        desktop_process(10, &[]), // the default profile: never tagged
        desktop_process(11, &["--type=renderer"]),
        desktop_process(20, &[&b]),
        desktop_process(21, &["--type=gpu-process", &b]),
    ]);

    let running = core.running_desktop_profiles().unwrap();

    assert_eq!(
        running.len(),
        1,
        "C isn't running, A is the default profile"
    );
    let profile = &running[0];
    assert_eq!(profile.account, uuid(UUID_B));
    assert_eq!(profile.label, "B");
    assert_eq!(profile.pids, [20, 21]);
    let identity = &profile.identity;
    assert_eq!(identity.app_user_model_id, app_user_model_id(&uuid(UUID_B)));
    assert_eq!(identity.display_name, "Claude B");
    assert_eq!(
        identity.relaunch_command.as_deref(),
        Some(format!(r#""{FAKE_MCCBOARD_APP}" --open-profile {UUID_B}"#).as_str())
    );
    let icon = app_icon(&core, UUID_B, &color_of(&core, UUID_B));
    assert_eq!(identity.icon.as_deref(), Some(icon.as_path()));
    assert_eq!(profile.icons.as_ref().map(|i| &i.app), Some(&icon));
}

#[test]
fn a_running_profile_is_tagged_even_without_the_app_or_an_icon() {
    let (home, core, env) = setup();
    add_b(&home, &core);
    env.set_mccboard_app(None);
    env.set_desktop_app_packages(Vec::new());
    fs::remove_dir_all(home.path().join(".mccboard").join("state").join("icons")).unwrap();
    env.set_desktop_processes(vec![desktop_process(
        20,
        &[&user_data_dir(&folder(&home, "b"))],
    )]);

    let running = core.running_desktop_profiles().unwrap();

    assert_eq!(running[0].identity.relaunch_command, None);
    assert_eq!(running[0].identity.icon, None);
    assert_eq!(
        running[0].identity.app_user_model_id,
        app_user_model_id(&uuid(UUID_B))
    );
}

#[test]
fn nothing_runs_nothing_is_drawn() {
    let (home, core, env) = setup();
    env.set_os(OsKind::Linux);
    add_b(&home, &core);

    assert!(core.running_desktop_profiles().unwrap().is_empty());
    assert!(
        !home
            .path()
            .join(".mccboard")
            .join("state")
            .join("icons")
            .exists()
    );
}

#[test]
fn the_shortcut_errors_have_their_message_in_both_ui_languages() {
    let locales = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/lib/locales");
    for language in ["en", "it"] {
        let text = fs::read_to_string(locales.join(format!("{language}.json"))).unwrap();
        let messages: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&text).unwrap();
        for code in [
            "start_menu_not_found",
            "app_program_not_found",
            "shortcut_not_written",
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
