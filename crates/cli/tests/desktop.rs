//! Smoke tests for `mccboard desktop`: list (default desktop profile first, Claude's orange),
//! add, set, remove at both levels, exit codes, `--json`, and no token from a profile's data
//! folder in any output.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const FIXTURE_TOKEN: &str = "sk-ant-oat01-FIXTURE-TOKEN-must-never-leak-0123456789";
const UUID_A: &str = "11111111-1111-4111-8111-111111111111";
const UUID_B: &str = "22222222-2222-4222-8222-222222222222";
const ORG: &str = "99999999-9999-4999-8999-999999999999";

fn mccboard(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mccboard"))
        .arg("--home")
        .arg(home)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn json(out: &Output) -> serde_json::Value {
    assert!(out.status.success(), "{}", text(out));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// A registered primary instance logged in as A, and a registered linked instance `b` logged
/// in as B.
fn fixture_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".claude")).unwrap();
    fs::write(
        home.path().join(".claude.json"),
        format!(
            r#"{{"oauthAccount":{{"accountUuid":"{UUID_A}","emailAddress":"a@example.com"}}}}"#
        ),
    )
    .unwrap();
    let b = home.path().join(".claude-b");
    fs::create_dir_all(&b).unwrap();
    fs::write(
        b.join(".claude.json"),
        format!(
            r#"{{"oauthAccount":{{"accountUuid":"{UUID_B}","emailAddress":"b@example.com"}}}}"#
        ),
    )
    .unwrap();
    let out = mccboard(home.path(), &["register", "~/.claude", "~/.claude-b"]);
    assert!(out.status.success(), "{}", text(&out));
    home
}

#[test]
fn list_shows_the_default_profile_first_in_claude_orange() {
    let home = fixture_home();

    let out = mccboard(home.path(), &["desktop", "list"]);
    assert!(out.status.success(), "{}", text(&out));
    let shown = text(&out);
    assert!(
        shown.contains("Default desktop profile: account A"),
        "{shown}"
    );
    assert!(shown.contains("#d97757"), "{shown}");
    assert!(shown.contains("not signed in yet"), "{shown}");

    let list = json(&mccboard(home.path(), &["desktop", "list", "--json"]));
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["default"], true);
    assert_eq!(list[0]["account"], UUID_A);
    assert_eq!(list[0]["color"], "#d97757");
}

#[test]
fn add_set_and_remove_a_profile() {
    let home = fixture_home();
    let folder = home.path().join("profiles").join("b");

    let added = json(&mccboard(
        home.path(),
        &[
            "desktop",
            "add",
            "B",
            "--folder",
            folder.to_str().unwrap(),
            "--json",
        ],
    ));
    assert_eq!(added["account"], UUID_B);
    assert_eq!(added["open_at_start"], false);
    assert_eq!(added["data_folder"], folder.to_str().unwrap());

    // The app signs in, and keeps its token cache next to it.
    fs::create_dir_all(folder.join("claude-code-sessions").join(UUID_B).join(ORG)).unwrap();
    fs::write(
        folder.join("config.json"),
        format!(r#"{{"oauth:tokenCache":"{FIXTURE_TOKEN}"}}"#),
    )
    .unwrap();

    let list = json(&mccboard(home.path(), &["desktop", "list", "--json"]));
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[0]["default"], true);
    assert_eq!(list[1]["account"], UUID_B);
    assert_eq!(list[1]["observed"]["account"], UUID_B);
    let shown = text(&mccboard(home.path(), &["desktop", "list"]));
    assert!(shown.contains("Desktop profile B"), "{shown}");
    assert!(shown.contains("signed in as   B"), "{shown}");

    let set = json(&mccboard(
        home.path(),
        &["desktop", "set", "B", "--open-at-start", "on", "--json"],
    ));
    assert_eq!(set["open_at_start"], true);

    // Deleting the folder needs --yes; nothing changes without it.
    let out = mccboard(home.path(), &["desktop", "remove", "B", "--with-folder"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("--with-folder --yes"), "{}", text(&out));
    assert!(folder.join("config.json").exists());

    let out = mccboard(
        home.path(),
        &["desktop", "remove", "B", "--with-folder", "--yes"],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains("deleted its data folder"),
        "{}",
        text(&out)
    );
    assert!(!folder.exists());
    let list = json(&mccboard(home.path(), &["desktop", "list", "--json"]));
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[test]
fn refusals_exit_with_one_and_say_why() {
    let home = fixture_home();

    let out = mccboard(home.path(), &["desktop", "remove", "A"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out).contains("default desktop profile"),
        "{}",
        text(&out)
    );

    let out = mccboard(home.path(), &["desktop", "add", "A"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out).contains("default desktop profile"),
        "{}",
        text(&out)
    );

    let out = mccboard(home.path(), &["desktop", "add", "Nobody"]);
    assert_eq!(out.status.code(), Some(1));

    let out = mccboard(home.path(), &["desktop", "remove", "B"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out).contains("has no desktop profile"),
        "{}",
        text(&out)
    );

    let out = mccboard(
        home.path(),
        &["desktop", "set", "B", "--open-at-start", "maybe"],
    );
    assert_eq!(out.status.code(), Some(2), "clap's usage error");

    let out = mccboard(home.path(), &["account", "set", "A", "--color", "#00ff00"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out).contains("orange"), "{}", text(&out));
}

#[test]
fn no_token_from_a_data_folder_in_any_output() {
    let home = fixture_home();
    let out = mccboard(home.path(), &["desktop", "add", "B"]);
    assert!(out.status.success(), "{}", text(&out));
    let list = json(&mccboard(home.path(), &["desktop", "list", "--json"]));
    let folder = Path::new(list[1]["data_folder"].as_str().unwrap()).to_path_buf();
    fs::create_dir_all(folder.join("claude-code-sessions").join(UUID_A).join(ORG)).unwrap();
    fs::write(
        folder.join("config.json"),
        format!(r#"{{"oauth:tokenCache":"{FIXTURE_TOKEN}"}}"#),
    )
    .unwrap();

    for args in [
        &["desktop", "list"][..],
        &["desktop", "list", "--json"][..],
        &["desktop", "set", "B", "--open-at-start", "off"][..],
        &["desktop", "remove", "B"][..],
    ] {
        let out = mccboard(home.path(), args);
        assert!(out.status.success(), "{args:?}: {}", text(&out));
        assert!(!text(&out).contains("FIXTURE"), "{args:?}");
    }
}

#[test]
fn open_needs_an_account_or_default_and_a_known_account() {
    let home = fixture_home();

    let out = mccboard(home.path(), &["desktop", "open"]);
    assert_eq!(out.status.code(), Some(2), "clap's usage error");
    let out = mccboard(home.path(), &["desktop", "open", "B", "--default"]);
    assert_eq!(out.status.code(), Some(2), "clap's usage error");

    let out = mccboard(home.path(), &["desktop", "open", "Nobody"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
}

/// Off Windows nothing is launched. (On Windows this would start the real app, so it isn't
/// run there: the core's tests cover opening through the fake environment.)
#[cfg(not(windows))]
#[test]
fn open_off_windows_says_profiles_open_on_windows_only() {
    let home = fixture_home();
    let folder = home.path().join("profiles").join("b");
    let out = mccboard(
        home.path(),
        &["desktop", "add", "B", "--folder", folder.to_str().unwrap()],
    );
    assert!(out.status.success(), "{}", text(&out));

    for args in [
        &["desktop", "open", "B"][..],
        &["desktop", "open", "--default", "--json"][..],
        &["desktop", "open", "A"][..],
    ] {
        let out = mccboard(home.path(), args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", text(&out));
        assert!(
            text(&out).contains("Windows only"),
            "{args:?}: {}",
            text(&out)
        );
    }
    assert!(!folder.exists(), "no folder is created when nothing opens");
}
