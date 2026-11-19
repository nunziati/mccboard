//! `mccboard claude-code status` and `install`, run as the binary with `--home` and a PATH of
//! the test's own. Fake `claude`, `curl` and `bash` stand in for the real ones: the real
//! installer never runs, and the real home is never touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const FIXTURE_TOKEN: &str = "sk-ant-oat01-FIXTURE-TOKEN-must-never-leak-0123456789";

/// A temporary home with a logged-in primary instance, and an empty `bin` folder, the PATH.
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let primary = home.path().join(".claude");
    fs::create_dir(&primary).unwrap();
    fs::write(
        primary.join(".credentials.json"),
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"{FIXTURE_TOKEN}","refreshToken":"{FIXTURE_TOKEN}","expiresAt":4102444800000}}}}"#
        ),
    )
    .unwrap();
    let bin = home.path().join("bin");
    fs::create_dir(&bin).unwrap();
    (home, bin)
}

/// Runs mccboard in `home` with `bin` as the whole PATH and no terminal.
fn mccboard(home: &Path, bin: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mccboard"))
        .arg("--home")
        .arg(home)
        .args(args)
        .env("PATH", bin)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn assert_no_token(out: &Output) {
    assert!(!stdout(out).contains("FIXTURE"), "{}", stdout(out));
    assert!(!stderr(out).contains("FIXTURE"), "{}", stderr(out));
}

/// Writes a fake program from a shell script. It sets its own PATH, since mccboard runs with
/// only `bin` on it. Copied in a child `cp`: this process never holds it open for writing
/// while another test thread starts a program (ETXTBSY).
#[cfg(unix)]
fn write_program(at: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let source = at.with_extension("txt");
    fs::write(&source, format!("#!/bin/sh\nPATH=/usr/bin:/bin\n{body}")).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    let status = Command::new("cp").arg(&source).arg(at).status().unwrap();
    assert!(status.success());
}

#[cfg(unix)]
const VERSION_SCRIPT: &str = "echo '2.1.301 (Claude Code)'\n";

/// Fake `curl` and `bash`: `curl` records its arguments in `out/curl` and prints `script`, and
/// `bash` runs what it reads, as the real ones do with the official installer.
#[cfg(unix)]
fn installer(bin: &Path, out: &Path, script: &str, bash_exit: i32) {
    fs::create_dir_all(out).unwrap();
    write_program(
        &bin.join("curl"),
        &format!(
            "printf '%s\\n' \"$*\" > '{}'\ncat <<'SCRIPT'\n{script}SCRIPT\n",
            out.join("curl").display()
        ),
    );
    write_program(&bin.join("bash"), &format!("sh\nexit {bash_exit}\n"));
}

/// The installer script's stand-in: it talks, then puts Claude Code in `$HOME/.local/bin`.
#[cfg(unix)]
const INSTALL_SCRIPT: &str = r#"echo 'Installing Claude Code...'
mkdir -p "$HOME/.local/bin"
printf '#!/bin/sh\necho "2.1.301 (Claude Code)"\n' > "$HOME/.local/bin/claude"
chmod 755 "$HOME/.local/bin/claude"
"#;

#[cfg(unix)]
#[test]
fn status_finds_claude_on_the_path_with_its_version() {
    let (home, bin) = fixture();
    write_program(&bin.join("claude"), VERSION_SCRIPT);

    let out = mccboard(home.path(), &bin, &["claude-code", "status", "--json"]);

    assert!(out.status.success(), "{}", stderr(&out));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["state"], "found");
    assert_eq!(json["program"], serde_json::json!(bin.join("claude")));
    assert_eq!(json["on_path"], true);
    assert_eq!(json["version"], "2.1.301 (Claude Code)");

    let out = mccboard(home.path(), &bin, &["claude-code", "status"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("version 2.1.301 (Claude Code)"),
        "{}",
        stdout(&out)
    );
    assert_no_token(&out);
}

#[cfg(unix)]
#[test]
fn status_finds_claude_in_local_bin_off_the_path() {
    let (home, bin) = fixture();
    let local = home.path().join(".local").join("bin");
    fs::create_dir_all(&local).unwrap();
    write_program(&local.join("claude"), VERSION_SCRIPT);

    let out = mccboard(home.path(), &bin, &["claude-code", "status", "--json"]);

    assert!(out.status.success(), "{}", stderr(&out));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["state"], "found");
    assert_eq!(json["program"], serde_json::json!(local.join("claude")));
    assert_eq!(json["on_path"], false);

    let out = mccboard(home.path(), &bin, &["claude-code", "status"]);
    let report = stdout(&out);
    assert!(report.contains("isn't on this terminal's PATH"), "{report}");
    // The login finds it in ~/.local/bin: only the launchers and the shell need the PATH.
    assert!(
        report.contains("mccboard's login and background runs find claude there"),
        "{report}"
    );
    assert!(
        report.contains("the launchers and your shell look for it on the PATH"),
        "{report}"
    );
}

#[cfg(unix)]
#[test]
fn status_without_a_readable_version_still_finds_claude() {
    let (home, bin) = fixture();
    write_program(&bin.join("claude"), "echo 'not now' >&2\nexit 1\n");

    let out = mccboard(home.path(), &bin, &["claude-code", "status", "--json"]);

    assert!(out.status.success(), "{}", stderr(&out));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["state"], "found");
    assert_eq!(json["version"], serde_json::Value::Null);

    let out = mccboard(home.path(), &bin, &["claude-code", "status"]);
    assert!(
        stdout(&out).contains("couldn't be read"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn status_without_claude_code_says_it_is_missing() {
    let (home, bin) = fixture();

    let out = mccboard(home.path(), &bin, &["claude-code", "status", "--json"]);

    assert!(out.status.success(), "{}", stderr(&out));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json, serde_json::json!({"state": "missing"}));

    let out = mccboard(home.path(), &bin, &["claude-code", "status"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("isn't installed"), "{}", stdout(&out));
    assert_no_token(&out);
}

#[cfg(unix)]
#[test]
fn install_without_yes_and_without_a_terminal_runs_nothing() {
    let (home, bin) = fixture();
    let out_dir = home.path().join("out");
    installer(&bin, &out_dir, INSTALL_SCRIPT, 0);

    let out = mccboard(home.path(), &bin, &["claude-code", "install"]);

    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(stderr(&out).contains("--yes"), "{}", stderr(&out));
    assert!(!out_dir.join("curl").exists());
    assert!(!home.path().join(".local").exists());
}

#[cfg(unix)]
#[test]
fn install_with_yes_runs_the_official_installer_in_the_home_then_finds_claude_code() {
    let (home, bin) = fixture();
    let out_dir = home.path().join("out");
    installer(&bin, &out_dir, INSTALL_SCRIPT, 0);

    let out = mccboard(home.path(), &bin, &["claude-code", "install", "--yes"]);

    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    let curl = fs::read_to_string(out_dir.join("curl")).unwrap();
    assert_eq!(curl, "-fsSL https://claude.ai/install.sh\n");
    // The installer talks on standard error: standard output is mccboard's own report.
    assert!(
        stderr(&out).contains("Installing Claude Code..."),
        "{}",
        stderr(&out)
    );
    assert!(
        !stdout(&out).contains("Installing Claude Code..."),
        "{}",
        stdout(&out)
    );
    let report = stdout(&out);
    assert!(report.contains("version 2.1.301 (Claude Code)"), "{report}");
    assert!(report.contains("isn't on this terminal's PATH"), "{report}");
    assert_no_token(&out);

    // `--home` is the installer's HOME: Claude Code landed in the temporary home.
    let out = mccboard(home.path(), &bin, &["claude-code", "status", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json["program"],
        serde_json::json!(home.path().join(".local").join("bin").join("claude"))
    );
}

#[cfg(unix)]
#[test]
fn install_with_a_failing_installer_fails_with_its_exit_code() {
    let (home, bin) = fixture();
    installer(
        &bin,
        &home.path().join("out"),
        "echo 'unsupported platform' >&2\n",
        3,
    );

    let out = mccboard(home.path(), &bin, &["claude-code", "install", "--yes"]);

    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("unsupported platform"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("exit code 3"), "{}", stderr(&out));
}

#[cfg(unix)]
#[test]
fn install_without_curl_on_the_path_runs_nothing() {
    let (home, bin) = fixture();
    let out_dir = home.path().join("out");
    installer(&bin, &out_dir, INSTALL_SCRIPT, 0);
    fs::remove_file(bin.join("curl")).unwrap();

    let out = mccboard(home.path(), &bin, &["claude-code", "install", "--yes"]);

    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(stderr(&out).contains("needs curl"), "{}", stderr(&out));
    assert!(!home.path().join(".local").exists());
}

#[cfg(windows)]
#[test]
fn on_windows_install_names_the_powershell_command() {
    let (home, bin) = fixture();

    let out = mccboard(home.path(), &bin, &["claude-code", "install"]);

    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("irm https://claude.ai/install.ps1 | iex"),
        "{}",
        stderr(&out)
    );
}
