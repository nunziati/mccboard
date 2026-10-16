//! Desktop profiles in the model (spec #34, ticket #35): stored in `config.toml`, the default
//! desktop profile implicit with Claude's orange, the rules with their typed errors, the
//! account observed from `claude-code-sessions` folder names only, the mismatch warnings,
//! "never link here", and removal at both levels behind the process guard. Runs on temporary
//! homes.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::{
    FIXTURE_TOKEN, TempHome, UUID_A, UUID_B, UUID_C, desktop_process, set_folder_modified,
    user_data_dir,
};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    AccountUpdate, AccountUuid, AddRequest, CLAUDE_ORANGE, Color, Core, DesktopDataDir,
    DesktopProcess, DesktopProfile, DesktopProfileUpdate, DesktopWarning, Error, FolderOutcome,
    Language, LoginTerminal, OsKind, RemoveLevel,
};

const ORG: &str = "99999999-9999-4999-8999-999999999999";

fn uuid(s: &str) -> AccountUuid {
    AccountUuid::new(s)
}

/// A home with the primary instance registered as A and `~/.claude-b` registered as B.
fn setup() -> (TempHome, Core, FakeEnvironment) {
    let home = TempHome::new();
    let (core, env) = home.core();
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    core.register(&primary.dir).unwrap();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    core.register(&b.dir).unwrap();
    (home, core, env)
}

/// Makes `<data folder>/claude-code-sessions/<account>/<org>/` as the app does at sign-in,
/// modified at `modified`.
fn signed_in(data_folder: &Path, account: &str, modified: SystemTime) {
    let account_dir = data_folder.join("claude-code-sessions").join(account);
    let org_dir = account_dir.join(ORG);
    fs::create_dir_all(&org_dir).unwrap();
    for dir in [&org_dir, &account_dir] {
        set_folder_modified(dir, modified);
    }
}

fn ago(minutes: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(minutes * 60)
}

fn profile<'a>(profiles: &'a [DesktopProfile], account: &str) -> &'a DesktopProfile {
    profiles
        .iter()
        .find(|p| !p.default && p.account.as_ref().map(AccountUuid::as_str) == Some(account))
        .unwrap()
}

fn default_profile(profiles: &[DesktopProfile]) -> &DesktopProfile {
    assert!(profiles[0].default, "the default profile comes first");
    &profiles[0]
}

fn orange() -> Color {
    CLAUDE_ORANGE.parse().unwrap()
}

// ---------------------------------------------------------------------------------------------
// The default desktop profile

#[test]
fn the_default_profile_is_implicit_with_claude_orange_and_the_primary_instance() {
    let (home, core, _env) = setup();

    let profiles = core.desktop_profiles().unwrap();

    assert_eq!(profiles.len(), 1);
    let default = default_profile(&profiles);
    assert_eq!(default.account, Some(uuid(UUID_A)));
    assert_eq!(default.label.as_deref(), Some("A"));
    assert_eq!(default.color, orange());
    assert_eq!(CLAUDE_ORANGE, "#d97757");
    assert_eq!(default.instance, Some(home.path().join(".claude")));
    assert!(!default.open_at_start);
    assert!(!home.config_text().unwrap().contains("desktop_profiles"));
}

#[test]
fn the_default_profile_is_listed_with_nothing_registered() {
    let home = TempHome::new();
    let (core, _env) = home.core();

    let profiles = core.desktop_profiles().unwrap();

    assert_eq!(profiles.len(), 1);
    let default = default_profile(&profiles);
    assert_eq!(default.account, None);
    assert_eq!(default.color, orange());
    assert_eq!(default.instance, None);
    assert_eq!(home.config_text(), None, "listing writes nothing");
}

#[test]
fn the_default_profiles_account_is_orange_in_every_view_model() {
    let (_home, core, _env) = setup();

    let registry = core.registry().unwrap();
    let a = registry.accounts.iter().find(|a| a.label == "A").unwrap();
    assert_eq!(a.color, orange());
    let b = registry.accounts.iter().find(|a| a.label == "B").unwrap();
    assert_ne!(b.color, orange());

    let usage = core.usage().unwrap();
    let a = usage
        .accounts
        .iter()
        .find(|u| u.account.uuid == uuid(UUID_A))
        .unwrap();
    assert_eq!(a.account.color, orange());

    let tray = core
        .tray_view(&usage, Language::English, &jiff::tz::TimeZone::UTC)
        .unwrap();
    let a = tray
        .accounts
        .iter()
        .find(|t| t.account.uuid == uuid(UUID_A))
        .unwrap();
    assert_eq!(a.account.color, orange());
}

#[test]
fn the_default_profiles_account_color_cannot_be_edited_but_its_label_can() {
    let (home, core, _env) = setup();
    let before = home.config_text().unwrap();

    let refused = core.update_account(
        &uuid(UUID_A),
        AccountUpdate {
            color: Some("#00ff00".parse().unwrap()),
            ..Default::default()
        },
    );

    assert!(
        matches!(&refused, Err(Error::DefaultProfileColor { label }) if label == "A"),
        "{refused:?}"
    );
    assert_eq!(refused.unwrap_err().code(), "default_profile_color");
    assert_eq!(home.config_text().unwrap(), before);

    let renamed = core
        .update_account(
            &uuid(UUID_A),
            AccountUpdate {
                label: Some("Main".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(renamed.label, "Main");
    assert_eq!(renamed.color, orange());
}

#[test]
fn the_default_profile_is_never_removed_nor_changed() {
    let (home, core, _env) = setup();
    let before = home.config_text().unwrap();

    for level in [RemoveLevel::FromMccboard, RemoveLevel::WithFolder] {
        let refused = core.remove_desktop_profile(&uuid(UUID_A), level);
        assert!(
            matches!(refused, Err(Error::DefaultDesktopProfile)),
            "{refused:?}"
        );
    }
    let refused = core.update_desktop_profile(
        &uuid(UUID_A),
        DesktopProfileUpdate {
            open_at_start: Some(true),
        },
    );
    assert!(
        matches!(refused, Err(Error::DefaultDesktopProfile)),
        "{refused:?}"
    );
    assert_eq!(home.config_text().unwrap(), before);
}

// ---------------------------------------------------------------------------------------------
// Adding

#[test]
fn a_profile_takes_its_accounts_local_instance_and_a_default_folder() {
    let (home, core, _env) = setup();

    let added = core.add_desktop_profile(&uuid(UUID_B), None).unwrap();

    // Elsewhere than Windows the folder is never used, but it is a real path all the same.
    let folder = home.path().join(".config").join("Claude-B");
    assert!(!added.default);
    assert_eq!(added.account, Some(uuid(UUID_B)));
    assert_eq!(added.label.as_deref(), Some("B"));
    assert_eq!(added.data_folder, folder);
    assert_eq!(added.instance, Some(home.path().join(".claude-b")));
    assert!(!added.open_at_start);
    assert!(
        !folder.exists(),
        "nothing is created before the first launch"
    );

    let profiles = core.desktop_profiles().unwrap();
    assert_eq!(profiles.len(), 2);
    assert!(profiles[0].default);
    let b = profile(&profiles, UUID_B);
    assert_eq!(b, &added);
    let registry = core.registry().unwrap();
    let account_b = registry.accounts.iter().find(|a| a.label == "B").unwrap();
    assert_eq!(b.color, account_b.color);

    let config: toml::Table = home.config_text().unwrap().parse().unwrap();
    let stored = config["desktop_profiles"].as_array().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["account"].as_str(), Some(UUID_B));
    assert_eq!(stored[0]["data_folder"].as_str(), folder.to_str());
    assert_eq!(stored[0]["open_at_start"].as_bool(), Some(false));
}

#[test]
fn on_windows_the_default_folder_is_in_local_app_data() {
    let (home, core, env) = setup();
    env.set_os(OsKind::Windows);

    let folder = core.desktop_profile_folder(&uuid(UUID_B)).unwrap();

    assert_eq!(
        folder,
        home.path().join("AppData").join("Local").join("Claude-B")
    );
}

#[test]
fn the_default_folder_keeps_the_labels_case_and_drops_unsafe_characters() {
    let (_home, core, _env) = setup();
    core.update_account(
        &uuid(UUID_B),
        AccountUpdate {
            label: Some("Personal Max/2".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let folder = core.desktop_profile_folder(&uuid(UUID_B)).unwrap();

    assert_eq!(folder.file_name().unwrap(), "Claude-Personal-Max-2");
}

#[test]
fn a_chosen_folder_is_taken_as_given_and_tilde_means_the_home() {
    let (home, core, _env) = setup();

    let added = core
        .add_desktop_profile(&uuid(UUID_B), Some(Path::new("~/profiles/b")))
        .unwrap();

    assert_eq!(added.data_folder, home.path().join("profiles").join("b"));
}

// ---------------------------------------------------------------------------------------------
// The rules

#[test]
fn an_account_without_a_local_instance_gets_no_profile() {
    let (home, core, _env) = setup();
    let text = home.config_text().unwrap();
    home.write_config(&format!(
        "{text}\n[accounts.{UUID_C}]\nlabel = \"C\"\ncolor = \"#16a34a\"\n"
    ));

    let refused = core.add_desktop_profile(&uuid(UUID_C), None);

    assert!(
        matches!(&refused, Err(Error::NoLocalInstance { label }) if label == "C"),
        "{refused:?}"
    );
    assert_eq!(refused.unwrap_err().code(), "no_local_instance");
    assert!(!home.config_text().unwrap().contains("desktop_profiles"));
}

#[test]
fn an_unknown_account_gets_no_profile() {
    let (_home, core, _env) = setup();

    let refused = core.add_desktop_profile(&uuid(UUID_C), None);

    assert!(
        matches!(refused, Err(Error::AccountNotFound { .. })),
        "{refused:?}"
    );
}

#[test]
fn one_profile_per_account() {
    let (home, core, _env) = setup();
    core.add_desktop_profile(&uuid(UUID_B), None).unwrap();
    let before = home.config_text().unwrap();

    let refused = core.add_desktop_profile(&uuid(UUID_B), Some(Path::new("~/elsewhere")));

    assert!(
        matches!(&refused, Err(Error::DesktopProfileExists { label }) if label == "B"),
        "{refused:?}"
    );
    assert_eq!(home.config_text().unwrap(), before);
}

#[test]
fn the_primary_instances_account_already_has_the_default_profile() {
    let (_home, core, _env) = setup();

    let refused = core.add_desktop_profile(&uuid(UUID_A), None);

    assert!(
        matches!(&refused, Err(Error::DefaultDesktopProfileAccount { label }) if label == "A"),
        "{refused:?}"
    );
}

#[test]
fn the_data_folder_stays_out_of_instances_mccboard_the_app_and_other_profiles() {
    let (home, core, _env) = setup();
    let c = home.linked(".claude-c");
    c.logged_in_as(UUID_C, "c@example.com");
    core.register(&c.dir).unwrap();
    core.add_desktop_profile(&uuid(UUID_B), Some(Path::new("~/profiles/b")))
        .unwrap();
    let before = home.config_text().unwrap();

    for folder in [
        "~/.claude-c/app",
        "~/.claude-b",
        "~/.claude/app",
        "~/.claude",
        "~/.mccboard/app",
        "~/profiles/b",
        "~/profiles/b/inner",
        "~/profiles",
        "~",
        "~/AppData/Roaming/Claude",
        "~/AppData/Local/Packages/Claude_pzs8sxrjxfjjc/LocalCache",
        "~/AppData/Local",
    ] {
        let refused = core.add_desktop_profile(&uuid(UUID_C), Some(Path::new(folder)));
        assert!(
            matches!(refused, Err(Error::DesktopProfileFolderNotAllowed { .. })),
            "{folder}: {refused:?}"
        );
    }
    let refused = core.add_desktop_profile(&uuid(UUID_C), Some(Path::new("relative/c")));
    assert!(
        matches!(refused, Err(Error::RelativePath { .. })),
        "{refused:?}"
    );
    home.write("a-file", "not a folder");
    let refused = core.add_desktop_profile(&uuid(UUID_C), Some(&home.path().join("a-file")));
    assert!(
        matches!(refused, Err(Error::NotADirectory { .. })),
        "{refused:?}"
    );
    assert_eq!(home.config_text().unwrap(), before);
}

#[test]
fn a_data_folder_behind_a_link_is_refused() {
    let (home, core, _env) = setup();
    let target = home.path().join("real");
    fs::create_dir_all(&target).unwrap();
    let link = home.path().join("linked-profile");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&target, &link).unwrap();

    let refused = core.add_desktop_profile(&uuid(UUID_B), Some(&link));

    assert!(
        matches!(refused, Err(Error::DesktopProfileFolderNotAllowed { .. })),
        "{refused:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// config.toml

#[test]
fn hand_written_profiles_are_understood_and_unknown_keys_survive_every_rewrite() {
    let (home, core, _env) = setup();
    let c = home.linked(".claude-c");
    c.logged_in_as(UUID_C, "c@example.com");
    core.register(&c.dir).unwrap();
    let b_folder = home.path().join("profiles").join("b");
    let text = home.config_text().unwrap();
    home.write_config(&format!(
        "{text}\n# My profiles\n[[desktop_profiles]]\naccount = \"{UUID_B}\"\n\
         data_folder = {b:?}  # where B lives\nopen_at_start = true\nnote = \"keep me\"\n",
        b = b_folder.to_str().unwrap(),
    ));

    let b = profile(&core.desktop_profiles().unwrap(), UUID_B).clone();
    assert_eq!(b.data_folder, b_folder);
    assert!(b.open_at_start);

    let set = core
        .update_desktop_profile(
            &uuid(UUID_B),
            DesktopProfileUpdate {
                open_at_start: Some(false),
            },
        )
        .unwrap();
    assert!(!set.open_at_start);
    core.add_desktop_profile(&uuid(UUID_C), None).unwrap();
    core.remove_desktop_profile(&uuid(UUID_C), RemoveLevel::FromMccboard)
        .unwrap();

    let text = home.config_text().unwrap();
    assert!(text.contains("note = \"keep me\""), "{text}");
    assert!(text.contains("# My profiles"), "{text}");
    assert!(text.contains("# where B lives"), "{text}");
    assert!(text.contains("open_at_start = false"), "{text}");
    assert!(!text.contains("Claude-C"), "{text}");
    let profiles = core.desktop_profiles().unwrap();
    assert_eq!(profiles.len(), 2);
    assert!(!profile(&profiles, UUID_B).open_at_start);
}

#[test]
fn profiles_written_as_an_inline_array_are_edited_in_place() {
    let (home, core, _env) = setup();
    let b_folder = home.path().join("profiles").join("b");
    let text = home.config_text().unwrap();
    home.write_config(&format!(
        "desktop_profiles = [{{ account = \"{UUID_B}\", data_folder = {b:?}, note = \"keep\" }}]\n{text}",
        b = b_folder.to_str().unwrap(),
    ));

    core.update_desktop_profile(
        &uuid(UUID_B),
        DesktopProfileUpdate {
            open_at_start: Some(true),
        },
    )
    .unwrap();

    assert!(profile(&core.desktop_profiles().unwrap(), UUID_B).open_at_start);
    assert!(home.config_text().unwrap().contains("note = \"keep\""));
    core.remove_desktop_profile(&uuid(UUID_B), RemoveLevel::FromMccboard)
        .unwrap();
    assert_eq!(core.desktop_profiles().unwrap().len(), 1);
}

#[test]
fn a_wrong_profile_entry_is_reported_and_the_file_never_written() {
    let (home, core, _env) = setup();
    let text = home.config_text().unwrap();
    for (entry, needle) in [
        (
            format!("[[desktop_profiles]]\naccount = \"{UUID_B}\"\ndata_folder = \"relative\"\n"),
            "desktop_profiles[0].data_folder",
        ),
        (
            format!(
                "[[desktop_profiles]]\naccount = \"{UUID_B}\"\ndata_folder = {f:?}\n\
                 [[desktop_profiles]]\naccount = \"{UUID_B}\"\ndata_folder = {g:?}\n",
                f = home.path().join("f").to_str().unwrap(),
                g = home.path().join("g").to_str().unwrap(),
            ),
            "desktop_profiles[1].account",
        ),
    ] {
        let written = format!("{text}\n{entry}");
        home.write_config(&written);

        let error = core.desktop_profiles().unwrap_err();

        assert!(
            matches!(&error, Error::ConfigCorrupt { detail, .. } if detail.contains(needle)),
            "{error:?}"
        );
        let refused = core.add_desktop_profile(&uuid(UUID_B), None);
        assert!(matches!(refused, Err(Error::ConfigCorrupt { .. })));
        assert_eq!(home.config_text().unwrap(), written);
    }
}

// ---------------------------------------------------------------------------------------------
// Observation and warnings

#[test]
fn a_profile_without_sessions_is_not_signed_in_yet() {
    let (home, core, _env) = setup();
    let folder = home.path().join("profiles").join("b");
    core.add_desktop_profile(&uuid(UUID_B), Some(&folder))
        .unwrap();

    let b = profile(&core.desktop_profiles().unwrap(), UUID_B).clone();
    assert_eq!(b.observed.account, None);
    assert!(b.observed.others.is_empty());
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);

    // The folder exists, with an empty sessions folder and names that aren't accounts.
    fs::create_dir_all(folder.join("claude-code-sessions").join("not-a-uuid")).unwrap();
    fs::write(folder.join("claude-code-sessions").join(UUID_C), "a file").unwrap();
    let b = profile(&core.desktop_profiles().unwrap(), UUID_B).clone();
    assert_eq!(b.observed.account, None);
}

#[test]
fn a_profile_signed_in_with_its_account_has_no_warning() {
    let (home, core, _env) = setup();
    let folder = home.path().join("profiles").join("b");
    core.add_desktop_profile(&uuid(UUID_B), Some(&folder))
        .unwrap();
    signed_in(&folder, UUID_B, ago(5));

    let b = profile(&core.desktop_profiles().unwrap(), UUID_B).clone();

    assert_eq!(b.observed.account, Some(uuid(UUID_B)));
    assert!(b.observed.others.is_empty());
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);
}

#[test]
fn with_several_accounts_the_most_recent_wins_and_the_others_are_reported() {
    let (home, core, _env) = setup();
    let folder = home.path().join("profiles").join("b");
    core.add_desktop_profile(&uuid(UUID_B), Some(&folder))
        .unwrap();
    signed_in(&folder, UUID_B, ago(60));
    signed_in(&folder, UUID_C, ago(1));
    signed_in(&folder, UUID_A, ago(30));

    let b = profile(&core.desktop_profiles().unwrap(), UUID_B).clone();

    assert_eq!(b.observed.account, Some(uuid(UUID_C)));
    assert_eq!(b.observed.others, vec![uuid(UUID_A), uuid(UUID_B)]);
    assert_eq!(
        b.warnings,
        vec![DesktopWarning::ObservedAccountDiffers {
            observed: uuid(UUID_C),
            observed_label: None,
        }]
    );
}

#[test]
fn a_profile_signed_in_with_another_account_warns() {
    let (home, core, _env) = setup();
    let folder = home.path().join("profiles").join("b");
    core.add_desktop_profile(&uuid(UUID_B), Some(&folder))
        .unwrap();
    signed_in(&folder, UUID_A, ago(5));

    let b = profile(&core.desktop_profiles().unwrap(), UUID_B).clone();

    assert_eq!(
        b.warnings,
        vec![DesktopWarning::ObservedAccountDiffers {
            observed: uuid(UUID_A),
            observed_label: Some("A".into()),
        }]
    );
}

#[test]
fn the_default_profile_signed_in_with_another_account_than_the_primary_instances_warns() {
    let (home, core, _env) = setup();
    // The MSIX app's virtualised view of %APPDATA%\Claude.
    let app = home
        .path()
        .join("AppData/Local/Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude");
    signed_in(&app, UUID_A, ago(10));

    let default = default_profile(&core.desktop_profiles().unwrap()).clone();
    assert_eq!(default.observed.account, Some(uuid(UUID_A)));
    assert!(default.warnings.is_empty(), "{:?}", default.warnings);

    // A later sign-in with B, in the real %APPDATA%\Claude.
    signed_in(&home.path().join("AppData/Roaming/Claude"), UUID_B, ago(1));
    let default = default_profile(&core.desktop_profiles().unwrap()).clone();
    assert_eq!(default.observed.account, Some(uuid(UUID_B)));
    assert_eq!(default.observed.others, vec![uuid(UUID_A)]);
    assert_eq!(
        default.warnings,
        vec![DesktopWarning::DefaultProfileNotPrimaryAccount {
            observed: uuid(UUID_B),
            observed_label: Some("B".into()),
        }]
    );
}

#[test]
fn a_profile_whose_instance_went_away_warns() {
    let (home, core, _env) = setup();
    core.add_desktop_profile(&uuid(UUID_B), None).unwrap();
    core.remove(&home.path().join(".claude-b"), RemoveLevel::FromMccboard)
        .unwrap();

    let b = profile(&core.desktop_profiles().unwrap(), UUID_B).clone();

    assert_eq!(b.instance, None);
    assert_eq!(b.warnings, vec![DesktopWarning::NoLocalInstance]);
}

#[test]
fn nothing_in_a_profiles_data_folder_is_read_beyond_the_sessions_folder_names() {
    let (home, core, env) = setup();
    let folder = home.path().join("profiles").join("b");
    core.add_desktop_profile(&uuid(UUID_B), Some(&folder))
        .unwrap();
    signed_in(&folder, UUID_B, ago(5));
    // The app's token cache, and a session file, both holding a token-shaped value.
    let secret = format!(r#"{{"oauth:tokenCache":"{FIXTURE_TOKEN}"}}"#);
    fs::write(folder.join("config.json"), &secret).unwrap();
    fs::write(
        folder
            .join("claude-code-sessions")
            .join(UUID_B)
            .join(ORG)
            .join("session.json"),
        &secret,
    )
    .unwrap();
    let app = home.path().join("AppData/Roaming/Claude");
    signed_in(&app, UUID_A, ago(5));
    fs::write(app.join("config.json"), &secret).unwrap();
    env.set_desktop_processes(vec![desktop_process(7, &[&user_data_dir(&folder)])]);

    let profiles = core.desktop_profiles().unwrap();
    let removal = core.remove_desktop_profile(&uuid(UUID_B), RemoveLevel::WithFolder);

    let outputs = [
        serde_json::to_string(&profiles).unwrap(),
        format!("{profiles:?}"),
        format!("{removal:?}"),
        removal.as_ref().map_err(ToString::to_string).unwrap_err(),
    ];
    for output in outputs {
        assert!(!output.contains("FIXTURE"), "{output}");
        assert!(!output.contains("tokenCache"), "{output}");
    }
    for (path, text) in home.mccboard_files() {
        assert!(!text.contains("FIXTURE"), "{}", path.display());
    }
}

// ---------------------------------------------------------------------------------------------
// Never link here

#[test]
fn mccboard_never_makes_an_instance_inside_a_profiles_data_folder() {
    let (home, core, _env) = setup();
    let folder = home.path().join("profiles").join("b");
    core.add_desktop_profile(&uuid(UUID_B), Some(&folder))
        .unwrap();

    let mut request = AddRequest::new("D", LoginTerminal::CurrentTerminal);
    request.config_dir = Some(folder.join(".claude-d"));
    request.login = false;
    let refused = core.add(&request);

    assert!(
        matches!(refused, Err(Error::DesktopAppFolder { .. })),
        "{refused:?}"
    );
    assert!(!folder.join(".claude-d").exists());
}

#[test]
fn mccboard_never_repairs_an_instance_inside_a_profiles_data_folder() {
    let (home, core, _env) = setup();
    // An instance made by hand inside what later becomes a profile's folder.
    let folder = home.path().join("profiles").join("b");
    let inside = folder.join("instance");
    fs::create_dir_all(&inside).unwrap();
    core.register(&inside).unwrap();
    // The folder rule refuses it now; a hand-edited config.toml can still say so.
    let text = home.config_text().unwrap();
    home.write_config(&format!(
        "{text}\n[[desktop_profiles]]\naccount = \"{UUID_B}\"\ndata_folder = {f:?}\n",
        f = folder.to_str().unwrap()
    ));

    let refused = core.repair(&inside, None);

    assert!(
        matches!(refused, Err(Error::DesktopAppFolder { .. })),
        "{refused:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// Removing

fn add_with_folder(home: &TempHome, core: &Core) -> PathBuf {
    let folder = home.path().join("profiles").join("b");
    core.add_desktop_profile(&uuid(UUID_B), Some(&folder))
        .unwrap();
    signed_in(&folder, UUID_B, ago(5));
    fs::write(folder.join("config.json"), "{}").unwrap();
    folder
}

#[test]
fn removing_from_mccboard_forgets_the_profile_and_keeps_its_folder() {
    let (home, core, _env) = setup();
    let folder = add_with_folder(&home, &core);

    let removal = core
        .remove_desktop_profile(&uuid(UUID_B), RemoveLevel::FromMccboard)
        .unwrap();

    assert_eq!(removal.account, uuid(UUID_B));
    assert_eq!(removal.data_folder, folder);
    assert_eq!(removal.folder, FolderOutcome::Kept);
    assert!(folder.join("config.json").exists());
    assert_eq!(core.desktop_profiles().unwrap().len(), 1);
    assert!(!home.config_text().unwrap().contains("desktop_profiles"));
}

#[test]
fn removing_with_the_folder_deletes_it_without_following_links() {
    let (home, core, _env) = setup();
    let folder = add_with_folder(&home, &core);
    let outside = home.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep.txt"), "keep").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, folder.join("link")).unwrap();
    #[cfg(windows)]
    junction::create(&outside, folder.join("link")).unwrap();

    let removal = core
        .remove_desktop_profile(&uuid(UUID_B), RemoveLevel::WithFolder)
        .unwrap();

    assert_eq!(removal.folder, FolderOutcome::Deleted);
    assert!(!folder.exists());
    assert!(outside.join("keep.txt").exists());
    assert_eq!(core.desktop_profiles().unwrap().len(), 1);
}

#[test]
fn removing_with_a_folder_already_gone_just_forgets_it() {
    let (_home, core, _env) = setup();
    core.add_desktop_profile(&uuid(UUID_B), None).unwrap();

    let removal = core
        .remove_desktop_profile(&uuid(UUID_B), RemoveLevel::WithFolder)
        .unwrap();

    assert_eq!(removal.folder, FolderOutcome::AlreadyGone);
    assert_eq!(core.desktop_profiles().unwrap().len(), 1);
}

#[test]
fn removing_with_the_folder_is_refused_while_the_app_uses_it() {
    let (home, core, env) = setup();
    let folder = add_with_folder(&home, &core);
    let before = home.config_text().unwrap();
    let process = DesktopProcess {
        pid: 4242,
        parent_pid: Some(1),
        name: "Claude.exe".into(),
        helper: None,
        data_dir: DesktopDataDir::Dir {
            path: folder.clone(),
        },
    };
    env.set_desktop_processes(vec![
        desktop_process(1, &[]),
        desktop_process(
            2,
            &[&user_data_dir(&home.path().join("profiles").join("c"))],
        ),
        desktop_process(4242, &[&user_data_dir(&folder.join("."))]),
    ]);

    assert_eq!(
        core.desktop_processes_using(&folder).unwrap(),
        vec![process.clone()]
    );
    let refused = core.remove_desktop_profile(&uuid(UUID_B), RemoveLevel::WithFolder);

    match refused {
        Err(error @ Error::DesktopProfileInUse { .. }) => {
            assert_eq!(error.code(), "desktop_profile_in_use");
            assert_eq!(error.desktop_processes(), Some(&[process][..]));
            assert_eq!(error.path(), Some(folder.as_path()));
        }
        other => panic!("{other:?}"),
    }
    assert!(folder.join("config.json").exists());
    assert_eq!(home.config_text().unwrap(), before);

    // Forgetting it only is fine while it runs.
    env.set_desktop_processes(Vec::new());
    core.remove_desktop_profile(&uuid(UUID_B), RemoveLevel::WithFolder)
        .unwrap();
    assert!(!folder.exists());
}

#[test]
fn a_hand_edited_profile_folder_on_a_protected_folder_is_never_deleted() {
    let (home, core, _env) = setup();
    let text = home.config_text().unwrap();
    let instance = home.path().join(".claude-b");
    home.write_config(&format!(
        "{text}\n[[desktop_profiles]]\naccount = \"{UUID_B}\"\ndata_folder = {f:?}\n",
        f = instance.to_str().unwrap()
    ));

    let refused = core.remove_desktop_profile(&uuid(UUID_B), RemoveLevel::WithFolder);

    assert!(
        matches!(refused, Err(Error::FolderNotDeletable { .. })),
        "{refused:?}"
    );
    assert!(instance.join(".claude.json").exists());
}

#[test]
fn removing_or_changing_a_profile_that_isnt_there_says_so() {
    let (_home, core, _env) = setup();

    let refused = core.remove_desktop_profile(&uuid(UUID_B), RemoveLevel::FromMccboard);
    assert!(
        matches!(&refused, Err(Error::DesktopProfileNotFound { label }) if label == "B"),
        "{refused:?}"
    );
    let refused = core.update_desktop_profile(
        &uuid(UUID_B),
        DesktopProfileUpdate {
            open_at_start: Some(true),
        },
    );
    assert!(
        matches!(refused, Err(Error::DesktopProfileNotFound { .. })),
        "{refused:?}"
    );
    let refused = core.remove_desktop_profile(&uuid(UUID_C), RemoveLevel::FromMccboard);
    assert!(
        matches!(refused, Err(Error::AccountNotFound { .. })),
        "{refused:?}"
    );
}

#[test]
fn open_at_start_is_off_by_default_and_can_be_turned_on_and_off() {
    let (_home, core, _env) = setup();
    core.add_desktop_profile(&uuid(UUID_B), None).unwrap();

    let on = core
        .update_desktop_profile(
            &uuid(UUID_B),
            DesktopProfileUpdate {
                open_at_start: Some(true),
            },
        )
        .unwrap();
    assert!(on.open_at_start);
    assert!(profile(&core.desktop_profiles().unwrap(), UUID_B).open_at_start);

    let off = core
        .update_desktop_profile(
            &uuid(UUID_B),
            DesktopProfileUpdate {
                open_at_start: Some(false),
            },
        )
        .unwrap();
    assert!(!off.open_at_start);
}
