//! Smoke tests for `mccboard repair`: exit codes, `--item`, `--json`, the session guard.

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

/// A primary with settings.json and agents/, and a registered linked instance `b` with a
/// copy of settings.json of its own.
fn fixture_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let primary = home.path().join(".claude");
    fs::create_dir_all(primary.join("agents")).unwrap();
    fs::write(primary.join("settings.json"), "{}").unwrap();
    let b = home.path().join(".claude-b");
    fs::create_dir_all(&b).unwrap();
    fs::write(b.join("settings.json"), r#"{"old": true}"#).unwrap();
    let out = mccboard(home.path(), &["register", "~/.claude", "~/.claude-b"]);
    assert!(out.status.success(), "{}", text(&out));
    home
}

#[test]
fn repair_fixes_every_registered_instance_and_doctor_then_finds_no_drift() {
    let home = fixture_home();
    let b = home.path().join(".claude-b");

    let out = mccboard(home.path(), &["repair"]);

    assert!(out.status.success(), "{}", text(&out));
    let printed = text(&out);
    assert!(printed.contains("link:settings.json"), "{printed}");
    assert!(printed.contains("backups"), "{printed}");
    assert!(
        fs::symlink_metadata(b.join("agents"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let doctor = mccboard(home.path(), &["doctor", "--no-ask"]);
    assert_eq!(doctor.status.code(), Some(0), "{}", text(&doctor));

    let again = mccboard(home.path(), &["repair"]);
    assert!(again.status.success());
    assert!(
        text(&again).contains("Nothing to repair"),
        "{}",
        text(&again)
    );
}

#[test]
fn one_item_by_id_with_json_output() {
    let home = fixture_home();

    let out = mccboard(
        home.path(),
        &["repair", "~/.claude-b", "--item", "link:agents", "--json"],
    );

    assert!(out.status.success(), "{}", text(&out));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let items = json[0]["report"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], "link:agents");
    assert_eq!(items[0]["outcome"]["outcome"], "done");
    // settings.json is still b's own copy.
    let settings = fs::symlink_metadata(home.path().join(".claude-b/settings.json")).unwrap();
    assert!(!settings.file_type().is_symlink());
}

#[test]
fn a_private_item_still_linked_to_the_primary_is_reported_then_unlinked() {
    // `downloads` answered "link" before it became private, and linked back then.
    let home = fixture_home();
    assert!(mccboard(home.path(), &["repair"]).status.success());
    let primary_downloads = home.path().join(".claude").join("downloads");
    fs::create_dir(&primary_downloads).unwrap();
    fs::write(primary_downloads.join("file.zip"), "kept").unwrap();
    let at = home.path().join(".claude-b").join("downloads");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&primary_downloads, &at).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&primary_downloads, &at).unwrap();
    let config = home.path().join(".mccboard").join("config.toml");
    let text_before = fs::read_to_string(&config).unwrap();
    let with_answer = format!("{text_before}\n[sharing.items]\ndownloads = \"link\"\n");
    fs::write(&config, &with_answer).unwrap();

    let doctor = mccboard(home.path(), &["doctor", "--no-ask"]);
    assert_eq!(doctor.status.code(), Some(2), "{}", text(&doctor));
    let printed = text(&doctor);
    assert!(printed.contains("[private:downloads]"), "{printed}");
    assert!(printed.contains("is now ignored"), "{printed}");

    let out = mccboard(
        home.path(),
        &["repair", "~/.claude-b", "--item", "private:downloads"],
    );

    assert!(out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains("done    [private:downloads]"),
        "{}",
        text(&out)
    );
    assert!(fs::symlink_metadata(&at).is_err());
    assert_eq!(
        fs::read_to_string(primary_downloads.join("file.zip")).unwrap(),
        "kept"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), with_answer);
    let doctor = mccboard(home.path(), &["doctor", "--no-ask"]);
    assert_eq!(doctor.status.code(), Some(0), "{}", text(&doctor));
}

#[test]
fn unlinking_credentials_shared_with_the_primary_says_to_log_in_again() {
    let home = fixture_home();
    assert!(mccboard(home.path(), &["repair"]).status.success());
    let primary_credentials = home.path().join(".claude").join(".credentials.json");
    fs::write(&primary_credentials, "{}").unwrap();
    let at = home.path().join(".claude-b").join(".credentials.json");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&primary_credentials, &at).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&primary_credentials, &at).unwrap();

    let doctor = mccboard(home.path(), &["doctor", "--no-ask"]);
    assert_eq!(doctor.status.code(), Some(2), "{}", text(&doctor));
    let printed = text(&doctor);
    assert!(printed.contains("[private:.credentials.json]"), "{printed}");
    assert!(printed.contains("logged out"), "{printed}");
    assert!(printed.contains("mccboard login"), "{printed}");

    let out = mccboard(home.path(), &["repair", "~/.claude-b"]);

    assert!(out.status.success(), "{}", text(&out));
    let printed = text(&out);
    assert!(printed.contains("now logged out"), "{printed}");
    // The folder as it was given, quoted when it has a space.
    assert!(
        printed.contains("`mccboard login ~/.claude-b`"),
        "{printed}"
    );
    assert!(!printed.contains("logout"), "{printed}");
    assert!(fs::symlink_metadata(&at).is_err());
    assert_eq!(fs::read_to_string(&primary_credentials).unwrap(), "{}");
}

#[test]
fn an_unknown_item_or_a_missing_path_fails_with_1() {
    let home = fixture_home();

    let unknown = mccboard(
        home.path(),
        &["repair", "~/.claude-b", "--item", "link:nope"],
    );
    assert_eq!(unknown.status.code(), Some(1), "{}", text(&unknown));
    assert!(text(&unknown).contains("link:nope"));

    let no_path = mccboard(home.path(), &["repair", "--item", "link:agents"]);
    assert_eq!(no_path.status.code(), Some(1), "{}", text(&no_path));
}

/// Copies the executable `from` to `to` in a child `cp`, never in this process. While this
/// process holds `to` open for writing, a test on another thread that spawns a process forks
/// a copy of that descriptor, which lives until the child execs; running `to` in that window
/// fails with ETXTBSY ("Text file busy"). `cp`'s descriptor never enters this process.
#[cfg(target_os = "linux")]
fn copy_executable(from: &Path, to: &Path) {
    let status = Command::new("cp").arg(from).arg(to).status().unwrap();
    assert!(status.success(), "cp {} {}", from.display(), to.display());
}

#[cfg(target_os = "linux")]
#[test]
fn repair_refuses_while_claude_code_runs_in_the_instance() {
    let home = fixture_home();
    let b = home.path().join(".claude-b");
    let bin = home.path().join("bin").join("claude");
    fs::create_dir_all(bin.parent().unwrap()).unwrap();
    copy_executable(Path::new("/bin/sleep"), &bin);
    let mut claude = Command::new(&bin)
        .arg("30")
        .env("CLAUDE_CONFIG_DIR", &b)
        .spawn()
        .unwrap();
    for _ in 0..50 {
        if fs::read_link(format!("/proc/{}/exe", claude.id())).is_ok_and(|e| e == bin) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let out = mccboard(home.path(), &["repair", "~/.claude-b"]);
    let _ = claude.kill();
    let _ = claude.wait();

    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        text(&out).contains(&claude.id().to_string()),
        "{}",
        text(&out)
    );
    assert!(
        !fs::symlink_metadata(b.join("settings.json"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!b.join("agents").exists());
}
