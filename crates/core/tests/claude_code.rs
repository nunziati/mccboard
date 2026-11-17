//! Claude Code itself (#62): whether it is found and its version, and its official installer.
//!
//! Every test runs on a temporary home with the fake environment port, which plays the PATH,
//! `claude --version` and the installer. `~/.local/bin` is a real folder of the temporary home.

mod common;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use common::{FIXTURE_REFRESH, FIXTURE_TOKEN, TempHome, UUID_A, now};
use jiff::SignedDuration;
use mccboard_core::testing::FAKE_CLAUDE_VERSION;
use mccboard_core::{
    CLAUDE_CODE_INSTALL_COMMAND, CLAUDE_CODE_WINDOWS_INSTALL_COMMAND, ClaudeCode, ClaudeRun, Error,
    InstallerExit, InstallerInvocation, OsKind,
};
use serde_json::json;

const ON_PATH: &str = "/usr/bin/claude";
const CURL: &str = "/usr/bin/curl";
const BASH: &str = "/usr/bin/bash";

/// Writes a program at `<home>/.local/bin/<name>`, as the official installer leaves it.
fn put_in_local_bin(home: &Path, name: &str) -> PathBuf {
    let bin = home.join(".local").join("bin");
    fs::create_dir_all(&bin).unwrap();
    let program = bin.join(name);
    fs::write(&program, "#!/bin/sh\necho '2.1.301 (Claude Code)'\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    }
    program
}

fn exit(download_exit_code: Option<i32>, exit_code: Option<i32>) -> InstallerExit {
    InstallerExit {
        download_exit_code,
        exit_code,
    }
}

/// A fake machine with `curl` and `bash` on its PATH, Claude Code nowhere.
fn with_installer_tools(
    home: &TempHome,
) -> (mccboard_core::Core, mccboard_core::testing::FakeEnvironment) {
    let (core, env) = home.core();
    env.set_program("curl", Some(Path::new(CURL)));
    env.set_program("bash", Some(Path::new(BASH)));
    (core, env)
}

#[test]
fn claude_code_on_the_path_is_found_with_the_first_line_of_its_version() {
    let home = TempHome::new();
    let (core, env) = home.core();
    env.set_program("claude", Some(Path::new(ON_PATH)));
    env.on_claude_version(|_| Ok(ClaudeRun::ok("  2.1.301 (Claude Code)  \nmore\n")));

    let found = core.claude_code();

    assert_eq!(
        found,
        ClaudeCode::Found {
            program: PathBuf::from(ON_PATH),
            on_path: true,
            version: Some("2.1.301 (Claude Code)".to_owned()),
        }
    );
    assert!(found.is_found());
    assert_eq!(env.claude_version_runs(), vec![PathBuf::from(ON_PATH)]);
}

#[test]
fn claude_code_on_the_path_comes_before_the_one_in_local_bin() {
    let home = TempHome::new();
    let (core, env) = home.core();
    put_in_local_bin(home.path(), "claude");
    env.set_program("claude", Some(Path::new(ON_PATH)));

    let found = core.claude_code();

    assert_eq!(
        found,
        ClaudeCode::Found {
            program: PathBuf::from(ON_PATH),
            on_path: true,
            version: Some(FAKE_CLAUDE_VERSION.to_owned()),
        }
    );
}

#[test]
fn claude_code_only_in_local_bin_is_found_off_the_path() {
    let home = TempHome::new();
    let (core, env) = home.core();
    let local = put_in_local_bin(home.path(), "claude");

    let found = core.claude_code();

    assert_eq!(
        found,
        ClaudeCode::Found {
            program: local.clone(),
            on_path: false,
            version: Some(FAKE_CLAUDE_VERSION.to_owned()),
        }
    );
    // The version is read from the program found there.
    assert_eq!(env.claude_version_runs(), vec![local]);
}

#[test]
fn on_windows_claude_code_in_local_bin_is_claude_exe() {
    let home = TempHome::new();
    let (core, env) = home.core();
    env.set_os(OsKind::Windows);
    put_in_local_bin(home.path(), "claude");
    assert_eq!(core.claude_code(), ClaudeCode::Missing);

    let local = put_in_local_bin(home.path(), "claude.exe");

    assert!(matches!(
        core.claude_code(),
        ClaudeCode::Found { program, on_path: false, .. } if program == local
    ));
}

#[cfg(unix)]
#[test]
fn a_file_in_local_bin_that_cannot_run_is_not_claude_code() {
    use std::os::unix::fs::PermissionsExt;
    let home = TempHome::new();
    let (core, env) = home.core();
    let local = put_in_local_bin(home.path(), "claude");
    fs::set_permissions(&local, fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(core.claude_code(), ClaudeCode::Missing);
    assert!(env.claude_version_runs().is_empty());
}

#[test]
fn a_version_that_cannot_be_read_leaves_claude_code_found_without_one() {
    let cases: Vec<(&str, io::Result<ClaudeRun>)> = vec![
        (
            "exits with an error",
            Ok(ClaudeRun::exit(1, "2.1.301", "boom")),
        ),
        ("prints nothing", Ok(ClaudeRun::ok(""))),
        (
            "prints an empty first line",
            Ok(ClaudeRun::ok("\n2.1.301\n")),
        ),
        (
            "hangs",
            Err(io::Error::new(io::ErrorKind::TimedOut, "fake: too slow")),
        ),
        (
            "can't start",
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fake: permission denied",
            )),
        ),
    ];
    for (case, outcome) in cases {
        let home = TempHome::new();
        let (core, env) = home.core();
        env.set_program("claude", Some(Path::new(ON_PATH)));
        let outcome = std::sync::Mutex::new(Some(outcome));
        env.on_claude_version(move |_| outcome.lock().unwrap().take().unwrap());

        assert_eq!(
            core.claude_code(),
            ClaudeCode::Found {
                program: PathBuf::from(ON_PATH),
                on_path: true,
                version: None,
            },
            "{case}"
        );
    }
}

#[test]
fn without_claude_code_anywhere_it_is_missing_and_nothing_runs() {
    let home = TempHome::new();
    let (core, env) = home.core();

    let found = core.claude_code();

    assert_eq!(found, ClaudeCode::Missing);
    assert!(!found.is_found());
    assert!(env.claude_version_runs().is_empty());
}

#[test]
fn claude_code_reads_as_a_state_in_json() {
    let found = ClaudeCode::Found {
        program: PathBuf::from(ON_PATH),
        on_path: false,
        version: None,
    };

    assert_eq!(
        serde_json::to_value(&found).unwrap(),
        json!({"state": "found", "program": ON_PATH, "on_path": false, "version": null})
    );
    assert_eq!(
        serde_json::to_value(ClaudeCode::Missing).unwrap(),
        json!({"state": "missing"})
    );
}

#[test]
fn the_installer_runs_with_curl_and_bash_of_the_path_in_the_home_then_claude_code_is_found() {
    let home = TempHome::new();
    let (core, env) = with_installer_tools(&home);
    // Plays the official installer: Claude Code lands in ~/.local/bin, not yet on the PATH.
    env.on_claude_install(|install| {
        put_in_local_bin(&install.home, "claude");
        Ok(exit(Some(0), Some(0)))
    });
    assert_eq!(core.claude_code(), ClaudeCode::Missing);

    let found = core.install_claude_code().unwrap();

    assert_eq!(
        found,
        ClaudeCode::Found {
            program: home.path().join(".local").join("bin").join("claude"),
            on_path: false,
            version: Some(FAKE_CLAUDE_VERSION.to_owned()),
        }
    );
    assert_eq!(
        env.installer_invocations(),
        vec![InstallerInvocation {
            curl: PathBuf::from(CURL),
            bash: PathBuf::from(BASH),
            home: home.path().to_path_buf(),
        }]
    );
}

#[test]
fn an_installer_that_ends_well_but_installs_nothing_leaves_claude_code_missing() {
    let home = TempHome::new();
    let (core, env) = with_installer_tools(&home);

    assert_eq!(core.install_claude_code().unwrap(), ClaudeCode::Missing);
    assert_eq!(env.installer_invocations().len(), 1);
}

#[test]
fn an_installer_that_fails_reports_its_exit_code() {
    let home = TempHome::new();
    let (core, env) = with_installer_tools(&home);
    env.on_claude_install(|_| Ok(exit(Some(0), Some(3))));

    let error = core.install_claude_code().unwrap_err();

    assert!(
        matches!(error, Error::ClaudeInstallerFailed { exit_code: Some(3) }),
        "{error:?}"
    );
    assert_eq!(error.code(), "claude_installer_failed");
    assert!(error.to_string().contains("exit code 3"), "{error}");
}

#[test]
fn an_installer_stopped_by_a_signal_fails_without_an_exit_code() {
    let home = TempHome::new();
    let (core, env) = with_installer_tools(&home);
    env.on_claude_install(|_| Ok(exit(Some(0), None)));

    let error = core.install_claude_code().unwrap_err();

    assert!(
        matches!(error, Error::ClaudeInstallerFailed { exit_code: None }),
        "{error:?}"
    );
}

#[test]
fn an_installer_that_could_not_be_downloaded_reports_curls_exit_code() {
    let home = TempHome::new();
    let (core, env) = with_installer_tools(&home);
    // curl failed, and bash ran the empty script it got, with success.
    env.on_claude_install(|_| Ok(exit(Some(6), Some(0))));

    let error = core.install_claude_code().unwrap_err();

    assert!(
        matches!(
            error,
            Error::ClaudeInstallerNotDownloaded { exit_code: Some(6) }
        ),
        "{error:?}"
    );
    assert_eq!(error.code(), "claude_installer_not_downloaded");
    assert!(error.to_string().contains("exit code 6"), "{error}");
}

#[test]
fn when_both_fail_the_installers_own_failure_is_reported() {
    let home = TempHome::new();
    let (core, env) = with_installer_tools(&home);
    // bash stopped early, and curl couldn't write the rest of the script (curl's 23).
    env.on_claude_install(|_| Ok(exit(Some(23), Some(1))));

    let error = core.install_claude_code().unwrap_err();

    assert!(
        matches!(error, Error::ClaudeInstallerFailed { exit_code: Some(1) }),
        "{error:?}"
    );
}

#[test]
fn without_curl_or_bash_the_installer_is_refused_before_anything_runs() {
    let home = TempHome::new();
    let (core, env) = home.core();

    let error = core.install_claude_code().unwrap_err();
    assert!(
        matches!(&error, Error::ClaudeInstallerNeedsProgram { program } if program == "curl"),
        "{error:?}"
    );
    assert_eq!(error.code(), "claude_installer_needs_program");
    assert_eq!(error.detail(), Some("curl"));

    env.set_program("curl", Some(Path::new(CURL)));
    let error = core.install_claude_code().unwrap_err();
    assert!(
        matches!(&error, Error::ClaudeInstallerNeedsProgram { program } if program == "bash"),
        "{error:?}"
    );
    assert!(error.to_string().contains("bash"), "{error}");

    assert!(env.installer_invocations().is_empty());
}

#[test]
fn an_installer_that_cannot_start_says_why() {
    let home = TempHome::new();
    let (core, env) = with_installer_tools(&home);
    env.on_claude_install(|_| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "fake: bash can't be run",
        ))
    });

    let error = core.install_claude_code().unwrap_err();

    assert!(
        matches!(error, Error::ClaudeInstallerNotStarted { .. }),
        "{error:?}"
    );
    assert_eq!(error.code(), "claude_installer_not_started");
    assert!(error.detail().unwrap().contains("bash can't be run"));
}

#[test]
fn on_windows_the_installer_is_refused_with_the_powershell_command() {
    let home = TempHome::new();
    let (core, env) = with_installer_tools(&home);
    env.set_os(OsKind::Windows);

    let error = core.install_claude_code().unwrap_err();

    assert!(
        matches!(error, Error::ClaudeInstallerOnWindows),
        "{error:?}"
    );
    assert_eq!(error.code(), "claude_installer_on_windows");
    assert_eq!(
        error.detail(),
        Some("irm https://claude.ai/install.ps1 | iex")
    );
    assert_eq!(
        CLAUDE_CODE_WINDOWS_INSTALL_COMMAND,
        "irm https://claude.ai/install.ps1 | iex"
    );
    assert!(
        error
            .to_string()
            .contains(CLAUDE_CODE_WINDOWS_INSTALL_COMMAND),
        "{error}"
    );
    assert!(env.installer_invocations().is_empty());
}

#[test]
fn the_installer_command_shown_is_the_official_one() {
    assert_eq!(
        CLAUDE_CODE_INSTALL_COMMAND,
        "curl -fsSL https://claude.ai/install.sh | bash"
    );
}

#[test]
fn neither_finding_nor_installing_claude_code_shows_or_touches_a_credential() {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() + SignedDuration::from_hours(8));
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() + SignedDuration::from_hours(8));
    let before = (primary.credentials_bytes(), b.credentials_bytes());
    let (core, env) = with_installer_tools(&home);
    env.on_claude_install(|install| {
        put_in_local_bin(&install.home, "claude");
        Ok(exit(Some(0), Some(0)))
    });

    let mut shown = vec![serde_json::to_string(&core.claude_code()).unwrap()];
    shown.push(serde_json::to_string(&core.install_claude_code().unwrap()).unwrap());
    env.on_claude_install(|_| Ok(exit(Some(0), Some(2))));
    shown.push(core.install_claude_code().unwrap_err().to_string());
    shown.push(format!("{:?}", env.installer_invocations()));

    for text in &shown {
        assert!(!text.contains(FIXTURE_TOKEN), "{text}");
        assert!(!text.contains(FIXTURE_REFRESH), "{text}");
    }
    assert_eq!((primary.credentials_bytes(), b.credentials_bytes()), before);
}
