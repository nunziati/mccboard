//! `mccboard setup` without a terminal: it draws nothing, changes nothing and lists the
//! commands that do its steps, in the language mccboard would use. `mccboard setup --help`
//! succeeds, as install.sh checks before starting the setup.

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

fn home_speaking(language: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".mccboard").join("config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(
        &config,
        format!("[preferences]\nlanguage = \"{language}\"\n"),
    )
    .unwrap();
    home
}

/// The commands of the eight steps, in either language.
const COMMANDS: &[&str] = &[
    "1. mccboard claude-code install --yes",
    "2. mccboard scan",
    "mccboard register",
    "3. mccboard add",
    "mccboard login",
    "4. mccboard doctor",
    "mccboard share",
    "5. mccboard repair",
    "6. mccboard launcher sync",
    "7. mccboard settings --renew-expired-tokens on|off",
    "8. mccboard list",
];

#[test]
fn without_a_terminal_it_lists_the_commands_of_its_steps_and_changes_nothing() {
    let home = tempfile::tempdir().unwrap();

    let out = mccboard(home.path(), &["setup"]);

    assert_eq!(out.status.code(), Some(1));
    // Not a byte of the TUI: no alternate screen, no raw mode, nothing drawn.
    assert!(out.stdout.is_empty(), "{:?}", out.stdout);
    let stderr = String::from_utf8(out.stderr).unwrap();
    for command in COMMANDS {
        assert!(
            stderr.contains(command),
            "{command:?} missing in:\n{stderr}"
        );
    }
    // Nothing written: no config, no state, no launchers.
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 0);
}

#[test]
fn the_message_follows_the_language_of_mccboard() {
    for (language, heading) in [
        (
            "en",
            "`mccboard setup` needs a terminal. Without one, run its steps as commands:",
        ),
        (
            "it",
            "`mccboard setup` ha bisogno di un terminale. Senza, esegui i suoi passi come comandi:",
        ),
    ] {
        let home = home_speaking(language);

        let out = mccboard(home.path(), &["setup"]);

        assert_eq!(out.status.code(), Some(1));
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.contains(heading), "{stderr}");
        for command in COMMANDS {
            assert!(
                stderr.contains(command),
                "{command:?} missing in:\n{stderr}"
            );
        }
    }
}

#[test]
fn its_help_succeeds_without_a_terminal() {
    let home = tempfile::tempdir().unwrap();

    let out = mccboard(home.path(), &["setup", "--help"]);

    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Set mccboard up one step at a time"),
        "{stdout}"
    );
}
