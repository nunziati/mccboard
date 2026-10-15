//! Smoke tests for `mccboard usage`. None of them can reach the endpoint: they use `--cached`,
//! or homes with no token at all.

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
    assert_eq!(json["accounts"][0]["status"]["status"], "never_read");
    assert_eq!(json["accounts"][0]["called"], false);
    assert_no_token(&out);
}

#[test]
fn cached_usage_shows_the_last_reading_with_its_windows() {
    let home = registered_home(true);
    write_state(
        home.path(),
        &format!(
            r#"{{"version":1,"accounts":{{"{UUID_A}":{{
                "reading":{{"source":"endpoint","read_at":"2020-01-01T12:00:00Z",
                  "session":{{"utilization":35.0,"resets_at":"2099-01-01T00:00:00Z"}},
                  "weekly":{{"utilization":14.0,"resets_at":"2099-01-02T00:00:00Z"}},
                  "models":[{{"model":"Fable","utilization":58.0,"resets_at":"2099-01-02T00:00:00Z"}}]}},
                "last_failure":null,"interval_secs":60,"next_call_at":"2026-10-15T12:01:00Z"}}}}}}"#
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
    assert_eq!(json["accounts"][0]["status"]["status"], "not_logged_in");
    assert_eq!(json["accounts"][0]["called"], false);
}

#[test]
fn usage_with_nothing_registered_says_so() {
    let home = tempfile::tempdir().unwrap();

    let out = mccboard(home.path(), &["usage"]);

    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("No account registered"));
}
