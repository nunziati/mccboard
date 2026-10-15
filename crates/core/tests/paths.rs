//! Where mccboard keeps its own files (AGENTS.md: `~/.mccboard/`, never `~/.claude`).

mod common;

use common::TempHome;
use mccboard_core::InstanceKind;

#[test]
fn mccboards_files_live_in_dot_mccboard_with_state_apart_from_config() {
    let home = TempHome::new();
    let (core, _env) = home.core();

    let paths = core.paths();

    assert_eq!(paths.root(), home.path().join(".mccboard"));
    assert_eq!(
        paths.config_file(),
        home.path().join(".mccboard").join("config.toml")
    );
    assert_eq!(
        paths.state_dir(),
        home.path().join(".mccboard").join("state")
    );
}

#[test]
fn only_the_default_config_directory_is_primary() {
    let home = TempHome::new();
    let (core, _env) = home.core();

    assert_eq!(
        core.kind_of(&home.path().join(".claude")),
        InstanceKind::Primary
    );
    assert_eq!(
        core.kind_of(&home.path().join(".claude-b")),
        InstanceKind::Linked
    );
    assert_eq!(
        core.kind_of(&home.path().join("work").join("claude")),
        InstanceKind::Linked
    );
}
