//! `mccboard dashboard` without a terminal: it draws nothing, touches nothing and names the
//! commands that print the same, in the language mccboard would use.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn dashboard(home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mccboard"))
        .arg("--home")
        .arg(home)
        .arg("dashboard")
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

#[test]
fn without_a_terminal_it_names_the_commands_that_print_the_same() {
    let home = home_speaking("en");

    let out = dashboard(home.path());

    assert_eq!(out.status.code(), Some(1));
    // Not a byte of the TUI: no alternate screen, no raw mode, nothing drawn.
    assert!(out.stdout.is_empty(), "{:?}", out.stdout);
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("`mccboard dashboard` needs a terminal"),
        "{stderr}"
    );
    assert!(stderr.contains("`mccboard usage`"), "{stderr}");
    assert!(stderr.contains("`mccboard list`"), "{stderr}");
    // Nothing was read from the endpoint, nothing written.
    assert!(!home.path().join(".mccboard").join("state").exists());
}

#[test]
fn the_message_follows_the_language_of_mccboard() {
    let home = home_speaking("it");

    let out = dashboard(home.path());

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("`mccboard dashboard` ha bisogno di un terminale"),
        "{stderr}"
    );
    assert!(stderr.contains("`mccboard usage`"), "{stderr}");
    assert!(stderr.contains("`mccboard list`"), "{stderr}");
}
