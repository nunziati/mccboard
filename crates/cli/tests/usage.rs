//! Smoke tests for `mccboard usage`. None of them can reach the endpoint: they use `--cached`,
//! homes with no token at all, or an expired token that a fake Claude Code doesn't renew.

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

/// A home whose primary instance is logged in as A, registered; with a token only if asked.
fn registered_home(with_token: bool) -> tempfile::TempDir {
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
    if with_token {
        fs::write(
            primary.join(".credentials.json"),
            format!(
                r#"{{"claudeAiOauth":{{"accessToken":"{FIXTURE_TOKEN}","refreshToken":"{FIXTURE_TOKEN}","expiresAt":4102444800000}}}}"#
            ),
        )
        .unwrap();
    }
    let out = mccboard(home.path(), &["register", "~/.claude"]);
    assert!(out.status.success());
    home
}

fn write_state(home: &Path, contents: &str) {
    let state = home.join(".mccboard").join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("usage.json"), contents).unwrap();
}

fn assert_no_token(out: &Output) {
    for stream in [&out.stdout, &out.stderr] {
        let text = String::from_utf8_lossy(stream);
        assert!(!text.contains("FIXTURE"), "{text}");
    }
}

#[test]
fn cached_usage_json_lists_accounts_without_calling() {
    let home = registered_home(true);

    let out = mccboard(home.path(), &["usage", "--cached", "--json"]);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["accounts"][0]["account"]["uuid"], UUID_A);
    assert_eq!(json["accounts"][0]["situation"]["freshness"], "no_reading");
    assert_eq!(
        json["accounts"][0]["situation"]["blocker"],
        serde_json::Value::Null
    );
    assert_eq!(json["accounts"][0]["called"], false);
    assert_no_token(&out);
}

#[test]
fn cached_usage_shows_the_last_reading_with_its_windows() {
    let home = registered_home(true);
    write_state(
        home.path(),
        &format!(
            r#"{{"version":2,"accounts":{{"{UUID_A}":{{
                "reading":{{"source":"endpoint","read_at":"2020-01-01T12:00:00Z",
                  "session":{{"utilization":35.0,"resets_at":"2099-01-01T00:00:00Z"}},
                  "weekly":{{"utilization":14.0,"resets_at":"2099-01-02T00:00:00Z"}},
                  "models":[{{"model":"Fable","utilization":58.0,"resets_at":"2099-01-02T00:00:00Z"}}]}},
                "last_failure":null,"interval_secs":60,"next_at":"2026-10-15T12:01:00Z"}}}}}}"#
        ),
    );

    let out = mccboard(home.path(), &["usage", "--cached"]);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout.clone()).unwrap();
    assert!(stdout.contains("session"), "{stdout}");
    assert!(stdout.contains("35%"), "{stdout}");
    assert!(stdout.contains("Fable (weekly)"), "{stdout}");
    assert!(stdout.contains("58%"), "{stdout}");
    // Read long ago by the real clock.
    assert!(stdout.contains("(stale)"), "{stdout}");
    assert_no_token(&out);
}

#[test]
fn a_corrupt_state_file_is_discarded_with_a_warning() {
    let home = registered_home(true);
    write_state(home.path(), "{ nope");

    let out = mccboard(home.path(), &["usage", "--cached"]);

    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("discarded"));
}

#[test]
fn refreshing_an_account_without_a_token_reports_it_not_logged_in() {
    let home = registered_home(false);

    let out = mccboard(home.path(), &["usage", "--json"]);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json["accounts"][0]["situation"]["blocker"]["kind"],
        "not_logged_in"
    );
    assert_eq!(json["accounts"][0]["called"], false);
}

#[test]
fn usage_with_nothing_registered_says_so() {
    let home = tempfile::tempdir().unwrap();

    let out = mccboard(home.path(), &["usage"]);

    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("No account registered"));
}

// --- Claude Code renewing an expired token (#30) ------------------------------------------

/// A home whose primary instance is logged in as A with a token that expired in 2020.
fn expired_home() -> tempfile::TempDir {
    let home = registered_home(false);
    fs::write(
        home.path().join(".claude").join(".credentials.json"),
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"{FIXTURE_TOKEN}","refreshToken":"{FIXTURE_TOKEN}","expiresAt":1577836800000}}}}"#
        ),
    )
    .unwrap();
    home
}

/// A folder holding only a fake `claude` that appends what it was run with to `<bin>/seen`
/// and exits 0 without renewing anything, so nothing can reach the endpoint.
fn fake_claude(home: &Path) -> std::path::PathBuf {
    let bin = home.join("fake-bin");
    fs::create_dir_all(&bin).unwrap();
    let seen = bin.join("seen");
    if cfg!(windows) {
        fs::write(
            bin.join("claude.cmd"),
            format!(
                "@echo off\r\n>>\"{}\" echo %CLAUDE_CONFIG_DIR%^|%USERPROFILE%^|%CD%^|%*\r\n\
                 echo {{\"loggedIn\": true, \"email\": \"a@example.com\"}}\r\nexit /b 0\r\n",
                seen.display()
            ),
        )
        .unwrap();
    } else {
        // Written next to it, then copied by `cp`: this process never holds the program open
        // for writing while other tests start processes (ETXTBSY).
        let source = home.join("claude.txt");
        fs::write(
            &source,
            format!(
                "#!/bin/sh\necho \"${{CLAUDE_CONFIG_DIR-}}|$HOME|$PWD|$*\" >> '{}'\n\
                 echo '{{\"loggedIn\": true, \"email\": \"a@example.com\"}}'\nexit 0\n",
                seen.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let copied = Command::new("cp")
            .arg(&source)
            .arg(bin.join("claude"))
            .status()
            .unwrap();
        assert!(copied.success());
    }
    bin
}

/// `mccboard` with a PATH where the only `claude` is the fake one: never a real Claude Code.
fn mccboard_with_fake_claude(home: &Path, bin: &Path, args: &[&str]) -> Output {
    let system: Vec<std::path::PathBuf> = if cfg!(windows) {
        // cmd.exe, which runs the fake `claude.cmd`.
        vec![std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32")]
    } else {
        vec!["/usr/bin".into(), "/bin".into()]
    };
    let path = std::env::join_paths(std::iter::once(bin.to_path_buf()).chain(system)).unwrap();
    Command::new(env!("CARGO_BIN_EXE_mccboard"))
        .arg("--home")
        .arg(home)
        .args(args)
        .env("PATH", path)
        .output()
        .unwrap()
}

fn seen(bin: &Path) -> Vec<String> {
    fs::read_to_string(bin.join("seen"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn an_expired_token_is_handed_to_claude_code_once_and_the_outcome_shown() {
    let home = expired_home();
    let bin = fake_claude(home.path());

    let out = mccboard_with_fake_claude(home.path(), &bin, &["usage"]);

    assert!(out.status.success(), "{out:?}");
    assert_no_token(&out);
    let seen = seen(&bin);
    assert_eq!(seen.len(), 1, "{seen:?}");
    let fields: Vec<&str> = seen[0].trim().split('|').collect();
    // The primary instance: no CLAUDE_CONFIG_DIR, the home is its folder's parent.
    assert_eq!(fields[0], "");
    let same = |a: &str, b: &Path| {
        assert_eq!(
            Path::new(a).canonicalize().unwrap(),
            b.canonicalize().unwrap()
        )
    };
    same(fields[1], home.path());
    same(fields[2], &home.path().join(".mccboard/state/renewal"));
    assert_eq!(fields[3], "mcp get mccboard-renewal-no-such-server");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("token expired"), "{text}");
    assert!(text.contains("the token is still expired"), "{text}");
    assert!(text.contains("next try after"), "{text}");
    // Claude Code's own output names the account; mccboard shows none of it.
    assert!(!text.contains("loggedIn"), "{text}");

    // The next refresh within 30 minutes doesn't run it again.
    let again = mccboard_with_fake_claude(home.path(), &bin, &["usage", "--json"]);
    assert!(again.status.success());
    assert_eq!(self::seen(&bin).len(), 1);
    let report: serde_json::Value = serde_json::from_slice(&again.stdout).unwrap();
    let renewal = &report["accounts"][0]["situation"]["renewal"];
    assert_eq!(renewal["state"], "failed");
    assert_eq!(renewal["failure"]["kind"], "still_expired");
}

#[test]
fn with_renewal_off_claude_code_is_not_run() {
    let home = expired_home();
    let bin = fake_claude(home.path());

    let set = mccboard(home.path(), &["settings", "--renew-expired-tokens", "off"]);
    assert!(set.status.success(), "{set:?}");
    assert!(String::from_utf8_lossy(&set.stdout).contains("renew-expired-tokens: off"));
    let out = mccboard_with_fake_claude(home.path(), &bin, &["usage"]);

    assert!(out.status.success());
    assert!(seen(&bin).is_empty());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--renew-expired-tokens on"), "{text}");

    let shown = mccboard(home.path(), &["settings", "--json"]);
    let settings: serde_json::Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(settings["renew_expired_tokens"], false);
}

// --- The taskbar widget's preferences (#45) ---------------------------------------------------

#[test]
fn the_taskbar_widget_preferences_are_shown_and_changed() {
    let home = registered_home(true);

    let shown = mccboard(home.path(), &["settings"]);
    let text = String::from_utf8_lossy(&shown.stdout);
    assert!(text.contains("taskbar-widget: on"), "{text}");
    assert!(text.contains("taskbar-widget-labels: off"), "{text}");

    let set = mccboard(
        home.path(),
        &[
            "settings",
            "--taskbar-widget",
            "off",
            "--taskbar-widget-labels",
            "on",
            "--json",
        ],
    );
    assert!(set.status.success(), "{set:?}");
    let settings: serde_json::Value = serde_json::from_slice(&set.stdout).unwrap();
    assert_eq!(settings["taskbar_widget"], false);
    assert_eq!(settings["taskbar_widget_labels"], true);
    let config = std::fs::read_to_string(home.path().join(".mccboard/config.toml")).unwrap();
    assert!(config.contains("taskbar_widget = false"), "{config}");
}
