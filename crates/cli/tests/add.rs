//! Smoke tests for `mccboard add` and `mccboard login`. No test opens a real login: either
//! `--no-login`, an unregistered folder, or (on Linux) no terminal stops it first.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

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

fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// A registered primary with settings.json and agents/.
fn fixture_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let primary = home.path().join(".claude");
    fs::create_dir_all(primary.join("agents")).unwrap();
    fs::write(primary.join("settings.json"), "{}").unwrap();
    let out = mccboard(home.path(), &["register", "~/.claude"]);
    assert!(out.status.success(), "{}", text(&out));
    home
}

#[test]
fn add_without_login_sets_up_and_registers_the_instance() {
    let home = fixture_home();

    let out = mccboard(home.path(), &["add", "B", "--no-login", "--json"]);

    assert!(out.status.success(), "{}", text(&out));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let dir = home.path().join(".claude-b");
    assert_eq!(json["config_dir"], serde_json::json!(dir));
    assert_eq!(json["created"], true);
    assert_eq!(json["login"]["run"]["run"], "not_requested");
    assert!(is_link(&dir.join("settings.json")));
    assert!(is_link(&dir.join("agents")));

    let list = mccboard(home.path(), &["list", "--json"]);
    let list: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let dirs: Vec<&str> = list["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["config_dir"].as_str().unwrap())
        .collect();
    assert!(dirs.contains(&dir.to_str().unwrap()), "{dirs:?}");

    let again = mccboard(home.path(), &["add", "B", "--no-login"]);
    assert!(again.status.success(), "{}", text(&again));
    assert!(text(&again).contains("exists already"), "{}", text(&again));
}

#[test]
fn add_refuses_the_primary_instance() {
    let home = fixture_home();

    let out = mccboard(home.path(), &["add", "B", "--dir", "~/.claude"]);

    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("primary instance"), "{}", text(&out));
}

#[test]
fn login_refuses_an_unregistered_folder() {
    let home = fixture_home();
    fs::create_dir(home.path().join(".claude-x")).unwrap();

    let out = mccboard(home.path(), &["login", "~/.claude-x"]);

    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        text(&out).contains("not a registered instance"),
        "{}",
        text(&out)
    );
}

/// On Linux the login runs in the current terminal: without one, it says how to log in, and
/// everything before the login is done.
#[cfg(not(windows))]
#[test]
fn without_a_terminal_add_sets_up_and_says_how_to_log_in() {
    let home = fixture_home();

    let out = mccboard(home.path(), &["add", "B"]);

    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("mccboard login"), "{}", text(&out));
    assert!(is_link(&home.path().join(".claude-b/settings.json")));

    let login = mccboard(home.path(), &["login", "~/.claude-b"]);
    assert_eq!(login.status.code(), Some(1), "{}", text(&login));
    assert!(
        text(&login).contains("needs a terminal"),
        "{}",
        text(&login)
    );
}
