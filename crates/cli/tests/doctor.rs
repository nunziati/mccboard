//! Smoke tests for `mccboard doctor` and `mccboard share`: exit codes, `--json`, answers.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const FIXTURE_TOKEN: &str = "sk-ant-oat01-FIXTURE-TOKEN-must-never-leak-0123456789";

fn mccboard(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mccboard"))
        .arg("--home")
        .arg(home)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A primary with settings.json and a GSD folder, and a registered linked instance `b`
/// without any link.
fn fixture_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let primary = home.path().join(".claude");
    fs::create_dir_all(primary.join("gsd-core")).unwrap();
    fs::write(primary.join("settings.json"), "{}").unwrap();
    fs::write(
        primary.join(".credentials.json"),
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"{FIXTURE_TOKEN}","expiresAt":4102444800000}}}}"#
        ),
    )
    .unwrap();
    fs::create_dir(home.path().join(".claude-b")).unwrap();
    let out = mccboard(home.path(), &["register", "~/.claude", "~/.claude-b"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    home
}

fn link_settings(home: &Path) {
    let target = home.join(".claude").join("settings.json");
    let at = home.join(".claude-b").join("settings.json");
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, at).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(target, at).unwrap();
}

#[test]
fn drift_exits_with_2_and_explains_each_item() {
    let home = fixture_home();

    let out = mccboard(home.path(), &["doctor"]);

    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = stdout(&out);
    assert!(text.contains("link:settings.json"), "{text}");
    assert!(text.contains("Repair:"), "{text}");
    assert!(text.contains("mccboard share gsd-core"), "{text}");
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn no_drift_exits_with_0() {
    let home = fixture_home();
    link_settings(home.path());

    let out = mccboard(home.path(), &["doctor"]);

    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(stdout(&out).contains("No drift"));
}

#[test]
fn json_output_is_machine_readable() {
    let home = fixture_home();

    let out = mccboard(home.path(), &["doctor", "--json"]);

    assert_eq!(out.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["drift_count"], 1);
    assert_eq!(json["instances"][0]["kind"], "primary");
    let item = &json["instances"][1]["items"][0];
    assert_eq!(item["id"], "link:settings.json");
    assert_eq!(item["problem"]["kind"], "missing_link");
    assert_eq!(item["repair"]["action"], "create_link");
    assert_eq!(json["questions"][0]["item"], "gsd-core");
    assert!(!stdout(&out).contains("FIXTURE"));
}

#[test]
fn share_answers_once_for_every_instance() {
    let home = fixture_home();
    link_settings(home.path());

    let out = mccboard(home.path(), &["share", "gsd-core", "link"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = mccboard(home.path(), &["doctor", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["questions"], serde_json::json!([]));
    assert_eq!(json["instances"][1]["items"][0]["id"], "link:gsd-core");

    let out = mccboard(home.path(), &["share", "gsd-core", "private"]);
    assert!(out.status.success());
    assert_eq!(mccboard(home.path(), &["doctor"]).status.code(), Some(0));
}

#[test]
fn share_refuses_items_in_the_sharing_table() {
    let home = fixture_home();

    let out = mccboard(home.path(), &["share", "settings.json", "private"]);

    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("settings.json"));
}

#[test]
fn doctor_checks_a_given_path_registered_or_not() {
    let home = fixture_home();
    fs::create_dir(home.path().join(".claude-c")).unwrap();

    let out = mccboard(home.path(), &["doctor", "~/.claude-c", "--json"]);

    assert_eq!(out.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["instances"].as_array().unwrap().len(), 1);
    assert_eq!(json["instances"][0]["kind"], "linked");
}
