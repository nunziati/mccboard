//! Smoke tests for aligned items in `mccboard doctor` and `mccboard repair`: a Global config
//! key shown with both values, then repaired. Plugins aren't exercised here: repairing them
//! runs the real `claude`.

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

/// A primary whose `~/.claude.json` sets `diffTool`, and a registered linked instance `b`
/// that sets it differently.
fn fixture_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".claude")).unwrap();
    fs::write(
        home.path().join(".claude.json"),
        r#"{"diffTool": "auto", "theme": "dark"}"#,
    )
    .unwrap();
    let b = home.path().join(".claude-b");
    fs::create_dir_all(&b).unwrap();
    fs::write(
        b.join(".claude.json"),
        r#"{"theme": "light", "diffTool": "terminal"}"#,
    )
    .unwrap();
    let out = mccboard(home.path(), &["register", "~/.claude", "~/.claude-b"]);
    assert!(out.status.success(), "{}", text(&out));
    home
}

#[test]
fn doctor_shows_both_values_of_a_misaligned_key_and_repair_aligns_it() {
    let home = fixture_home();
    let b = home.path().join(".claude-b");

    let doctor = mccboard(home.path(), &["doctor", "--no-ask"]);
    assert_eq!(doctor.status.code(), Some(2), "{}", text(&doctor));
    let printed = text(&doctor);
    assert!(printed.contains("[config:diffTool]"), "{printed}");
    assert!(printed.contains("\"terminal\""), "{printed}");
    assert!(printed.contains("\"auto\""), "{printed}");

    let json = mccboard(home.path(), &["doctor", "--json"]);
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let item = &report["instances"][1]["items"][0];
    assert_eq!(item["problem"]["kind"], "misaligned_key");
    assert_eq!(item["problem"]["primary"], "auto");
    assert_eq!(item["problem"]["instance"], "terminal");
    assert_eq!(item["repair"]["action"], "align_key");

    let repair = mccboard(
        home.path(),
        &["repair", "~/.claude-b", "--item", "config:diffTool"],
    );
    assert!(repair.status.success(), "{}", text(&repair));
    let global: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(b.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(
        global,
        serde_json::json!({"theme": "light", "diffTool": "auto"})
    );
    let again = mccboard(home.path(), &["doctor", "--no-ask"]);
    assert_eq!(again.status.code(), Some(0), "{}", text(&again));
}
