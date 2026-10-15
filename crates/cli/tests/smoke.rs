//! Smoke tests: the binary runs, `--json` parses, exit codes, no token in output.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const FIXTURE_TOKEN: &str = "sk-ant-oat01-FIXTURE-TOKEN-must-never-leak-0123456789";
const UUID_A: &str = "11111111-1111-4111-8111-111111111111";

fn mccboard(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mccboard"))
        .arg("--home")
        .arg(home)
        .args(args)
        .output()
        .unwrap()
}

fn fixture_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let primary = home.path().join(".claude");
    fs::create_dir(&primary).unwrap();
    fs::write(
        home.path().join(".claude.json"),
        format!(
            r#"{{"oauthAccount":{{"accountUuid":"{UUID_A}","emailAddress":"a@example.com"}}}}"#
        ),
    )
    .unwrap();
    // Far in the future, so the test doesn't depend on the real clock.
    fs::write(
        primary.join(".credentials.json"),
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"{FIXTURE_TOKEN}","refreshToken":"{FIXTURE_TOKEN}","expiresAt":4102444800000}}}}"#
        ),
    )
    .unwrap();
    fs::create_dir(home.path().join(".claude-b")).unwrap();
    home
}

#[test]
fn scan_lists_candidates() {
    let home = fixture_home();

    let out = mccboard(home.path(), &["scan"]);

    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains(".claude-b"), "{stdout}");
    assert!(stdout.contains("primary"), "{stdout}");
    assert!(stdout.contains("a@example.com"), "{stdout}");
    assert!(stdout.contains("not logged in"), "{stdout}");
}

#[test]
fn scan_json_is_machine_readable() {
    let home = fixture_home();

    let out = mccboard(home.path(), &["scan", "--json"]);

    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json[0]["kind"], "primary");
    assert_eq!(json[0]["account"]["uuid"], UUID_A);
    assert_eq!(json[0]["login"]["state"], "logged_in");
    assert_eq!(json[1]["kind"], "linked");
    assert_eq!(json[1]["login"]["state"], "not_logged_in");
}

#[test]
fn no_token_in_any_output() {
    let home = fixture_home();

    for args in [&["scan"][..], &["scan", "--json"][..]] {
        let out = mccboard(home.path(), args);
        for stream in [&out.stdout, &out.stderr] {
            assert!(!String::from_utf8_lossy(stream).contains("FIXTURE"));
        }
    }
}

#[test]
fn a_missing_home_fails_with_a_non_zero_exit() {
    let home = tempfile::tempdir().unwrap();

    let out = mccboard(&home.path().join("nobody"), &["scan"]);

    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("nobody"));
}
