//! Launchers: one command per linked instance, `claude-b`, made from the account label
//! (spec stories 5, 25–29; ticket #7).
//!
//! Every test runs on a temporary home with the fake port: the shell rc file is the temporary
//! home's, and the user PATH is the fake's, so the real user environment is never touched.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{TempHome, UUID_A, UUID_B, UUID_C};
use mccboard_core::testing::{FAKE_LAUNCHER_SHIM, FakeEnvironment};
use mccboard_core::{
    AccountUpdate, AccountUuid, ClaudeProcess, CliInstall, Core, DriftItem, Error, Launcher,
    LauncherPlace, LauncherState, OsKind, Problem, ProcessConfigDir, Repair, RepairOutcome,
    UserPathChange,
};

const BEGIN: &str = "# >>> mccboard launchers >>>";
const END: &str = "# <<< mccboard launchers <<<";

/// A home with the primary instance (account A) and `.claude-b` (account B), registered.
fn home_with_b(os: OsKind) -> (TempHome, Core, FakeEnvironment) {
    let home = TempHome::new();
    home.primary().logged_in_as(UUID_A, "a@example.com");
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    let (core, env) = home.core();
    env.set_os(os);
    core.register(&home.path().join(".claude")).unwrap();
    core.register(&home.path().join(".claude-b")).unwrap();
    (home, core, env)
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn bashrc(home: &TempHome) -> PathBuf {
    home.path().join(".bashrc")
}

fn bin(home: &TempHome) -> PathBuf {
    home.path().join(".mccboard").join("bin")
}

/// The lines inside mccboard's block, without the markers and comments.
fn block_aliases(text: &str) -> Vec<String> {
    let start = text.find(BEGIN).expect("a launchers block");
    let end = text.find(END).expect("the end of the launchers block");
    text[start..end]
        .lines()
        .filter(|line| line.starts_with("alias "))
        .map(str::to_owned)
        .collect()
}

/// The alias line mccboard writes, quoted by its documented rule: the directory goes in
/// single quotes unless it is made only of letters, digits and `_@%+=:,./-`, then the whole
/// value does. Temporary folders can need quoting (`C:\Users\RUNNER~1\…` on Windows CI).
fn alias(name: &str, dir: &Path) -> String {
    let dir = dir.display().to_string();
    let plain = dir
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    let dir = if plain { dir } else { shell_quote(&dir) };
    format!(
        "alias {name}={}",
        shell_quote(&format!("CLAUDE_CONFIG_DIR={dir} claude"))
    )
}

/// Points the alias of `.claude-b` at `.claude-x` by hand, however its path is quoted.
fn point_the_alias_elsewhere(home: &TempHome) {
    let b = home.path().join(".claude-b").display().to_string();
    let x = home.path().join(".claude-x").display().to_string();
    let text = read(&bashrc(home));
    assert!(text.contains(&b), "{text}");
    fs::write(bashrc(home), text.replace(&b, &x)).unwrap();
}

fn uuid(s: &str) -> AccountUuid {
    AccountUuid::new(s)
}

// --- Linux: an alias in a marked block of the shell rc file ---

#[test]
fn registering_a_linked_instance_adds_its_alias_in_a_marked_block() {
    let home = TempHome::new();
    home.write(".bashrc", "# my settings\nexport EDITOR=vim\n");
    home.primary().logged_in_as(UUID_A, "a@example.com");
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();

    core.register(&home.path().join(".claude")).unwrap();
    core.register(&b.dir).unwrap();

    let text = read(&bashrc(&home));
    assert!(
        text.starts_with("# my settings\nexport EDITOR=vim\n"),
        "{text}"
    );
    assert_eq!(block_aliases(&text), vec![alias("claude-b", &b.dir)]);
    assert!(text.trim_end().ends_with(END), "{text}");
}

#[test]
fn the_primary_instance_keeps_plain_claude_and_gets_no_launcher() {
    let home = TempHome::new();
    home.primary().logged_in_as(UUID_A, "a@example.com");
    let (core, _env) = home.core();

    core.register(&home.path().join(".claude")).unwrap();

    assert!(!bashrc(&home).exists());
    let status = core.launchers().unwrap();
    assert!(status.instances.is_empty());
    assert_eq!(status.drift_count(), 0);
}

#[test]
fn a_zsh_user_gets_the_block_in_zshrc() {
    let home = TempHome::new();
    home.primary().logged_in_as(UUID_A, "a@example.com");
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let (core, env) = home.core();
    env.set_login_shell(Some("/usr/bin/zsh"));

    core.register(&b.dir).unwrap();

    assert!(!bashrc(&home).exists());
    let zshrc = home.path().join(".zshrc");
    assert_eq!(
        block_aliases(&read(&zshrc)),
        vec![alias("claude-b", &b.dir)]
    );
    assert_eq!(
        core.launchers().unwrap().place,
        LauncherPlace::RcFile { path: zshrc }
    );
}

#[test]
fn renaming_the_label_renames_the_launcher_and_the_override_wins() {
    let (home, core, _env) = home_with_b(OsKind::Linux);
    let b = home.path().join(".claude-b");

    core.update_account(
        &uuid(UUID_B),
        AccountUpdate {
            label: Some("Work".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        block_aliases(&read(&bashrc(&home))),
        vec![alias("claude-work", &b)]
    );

    core.update_account(
        &uuid(UUID_B),
        AccountUpdate {
            launcher: Some(Some("cw".into())),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(block_aliases(&read(&bashrc(&home))), vec![alias("cw", &b)]);

    core.update_account(
        &uuid(UUID_B),
        AccountUpdate {
            launcher: Some(None),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        block_aliases(&read(&bashrc(&home))),
        vec![alias("claude-work", &b)]
    );
}

#[test]
fn adding_renaming_and_removing_touch_only_the_block() {
    let home = TempHome::new();
    let before = "# top\nalias ll='ls -l'\n";
    home.write(".bashrc", before);
    home.primary().logged_in_as(UUID_A, "a@example.com");
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let c = home.linked(".claude-c");
    c.logged_in_as(UUID_C, "c@example.com");
    let (core, _env) = home.core();

    core.register(&b.dir).unwrap();
    // The user keeps editing the file after the block.
    let mut text = read(&bashrc(&home));
    text.push_str("export AFTER=1\n");
    fs::write(bashrc(&home), &text).unwrap();

    core.register(&c.dir).unwrap();
    core.update_account(
        &uuid(UUID_C),
        AccountUpdate {
            label: Some("Z".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let text = read(&bashrc(&home));
    assert!(text.starts_with(before), "{text}");
    assert!(
        text.ends_with(&format!("{END}\nexport AFTER=1\n")),
        "{text}"
    );
    assert_eq!(
        block_aliases(&text),
        vec![alias("claude-b", &b.dir), alias("claude-z", &c.dir)]
    );

    core.unregister(&b.dir).unwrap();
    core.unregister(&c.dir).unwrap();
    assert_eq!(read(&bashrc(&home)), format!("{before}export AFTER=1\n"));
}

#[test]
fn removing_the_last_launcher_restores_the_rc_file_exactly() {
    let home = TempHome::new();
    let before = "# my settings\nexport EDITOR=vim\n";
    home.write(".bashrc", before);
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();
    let b = home.path().join(".claude-b");

    core.register(&b).unwrap();
    assert_ne!(read(&bashrc(&home)), before);
    core.unregister(&b).unwrap();

    assert_eq!(read(&bashrc(&home)), before);
}

#[test]
fn an_instance_not_logged_in_gets_a_launcher_named_after_its_folder() {
    let home = TempHome::new();
    let work = home.linked(".claude-work");
    let odd = home.linked("claude configs/Team X");
    let (core, _env) = home.core();

    core.register(&work.dir).unwrap();
    core.register(&odd.dir).unwrap();

    let status = core.launchers().unwrap();
    let names: Vec<_> = status.instances.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["claude-work", "claude-team-x"]);
}

#[test]
fn two_instances_wanting_the_same_name_get_distinct_launchers() {
    let home = TempHome::new();
    // The same account logged in twice: both want `claude-b`.
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    home.linked(".claude-b2")
        .logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();

    core.register(&home.path().join(".claude-b")).unwrap();
    core.register(&home.path().join(".claude-b2")).unwrap();

    let status = core.launchers().unwrap();
    let names: Vec<_> = status.instances.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["claude-b", "claude-b-2"]);
}

#[test]
fn a_path_with_spaces_and_quotes_is_quoted_for_the_shell() {
    let home = TempHome::new();
    let odd = home.linked("it's a dir");
    let (core, _env) = home.core();

    core.register(&odd.dir).unwrap();

    let line = block_aliases(&read(&bashrc(&home))).remove(0);
    let quoted = format!("'{}'", odd.dir.display().to_string().replace('\'', r"'\''"));
    let expected = format!(
        "alias claude-it-s-a-dir={}",
        shell_quote(&format!("CLAUDE_CONFIG_DIR={quoted} claude"))
    );
    assert_eq!(line, expected);
    assert_eq!(
        core.launchers().unwrap().instances[0].state,
        LauncherState::Ok
    );
}

/// A quoted path reads back as written, and an alias pointing at another quoted path is
/// stale and says where it points: the reader undoes exactly the writer's quoting.
#[test]
fn a_quoted_path_round_trips_and_a_different_one_is_stale() {
    let home = TempHome::new();
    // `~` as in Windows short names (`RUNNER~1`); a backslash too where a name can hold one.
    let name = if cfg!(unix) { r"b~1\x" } else { "b~1" };
    let odd = home.linked(name);
    let (core, _env) = home.core();
    core.register(&odd.dir).unwrap();
    assert_eq!(
        core.launchers().unwrap().instances[0].state,
        LauncherState::Ok
    );

    let elsewhere = home.path().join("x~2 y");
    let text = read(&bashrc(&home));
    let written = odd.dir.display().to_string();
    assert!(text.contains(&format!("'\\''{written}'\\''")), "{text}");
    fs::write(
        bashrc(&home),
        text.replace(&written, &elsewhere.display().to_string()),
    )
    .unwrap();

    assert_eq!(
        core.launchers().unwrap().instances[0].state,
        LauncherState::Stale {
            found: Some(elsewhere)
        }
    );
    core.sync_launchers().unwrap();
    assert_eq!(core.launchers().unwrap().drift_count(), 0);
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// --- Status (for the doctor) and repair ---

#[test]
fn a_registered_linked_instance_reports_its_launcher_ok() {
    let (home, core, _env) = home_with_b(OsKind::Linux);

    let status = core.launchers().unwrap();

    assert_eq!(
        status.place,
        LauncherPlace::RcFile {
            path: bashrc(&home)
        }
    );
    assert_eq!(status.instances.len(), 1);
    let b = &status.instances[0];
    assert_eq!(b.config_dir, home.path().join(".claude-b"));
    assert_eq!(b.name, "claude-b");
    assert_eq!(b.state, LauncherState::Ok);
    assert!(status.orphans.is_empty());
    assert_eq!(status.drift_count(), 0);
    assert_eq!(
        status
            .instance(&home.path().join(".claude-b"))
            .map(|i| &i.state),
        Some(&LauncherState::Ok)
    );
}

#[test]
fn a_deleted_block_reads_as_missing_and_sync_restores_it() {
    let (home, core, _env) = home_with_b(OsKind::Linux);
    fs::write(bashrc(&home), "# cleaned up by hand\n").unwrap();

    let status = core.launchers().unwrap();
    assert_eq!(status.instances[0].state, LauncherState::Missing);
    assert_eq!(status.drift_count(), 1);

    let sync = core.sync_launchers().unwrap();
    assert_eq!(
        sync.created,
        vec![Launcher {
            name: "claude-b".into(),
            config_dir: Some(home.path().join(".claude-b")),
        }]
    );
    assert!(sync.updated.is_empty() && sync.removed.is_empty());
    assert_eq!(sync.user_path, None);
    assert_eq!(core.launchers().unwrap().drift_count(), 0);
    assert!(read(&bashrc(&home)).starts_with("# cleaned up by hand\n"));
}

#[test]
fn an_alias_pointing_elsewhere_is_stale_and_says_where_it_points() {
    let (home, core, _env) = home_with_b(OsKind::Linux);
    point_the_alias_elsewhere(&home);

    let status = core.launchers().unwrap();

    assert_eq!(
        status.instances[0].state,
        LauncherState::Stale {
            found: Some(home.path().join(".claude-x"))
        }
    );
    let sync = core.sync_launchers().unwrap();
    assert_eq!(sync.updated.len(), 1);
    assert_eq!(core.launchers().unwrap().drift_count(), 0);
}

/// The doctor's launcher items of `.claude-b`.
fn launcher_drift(home: &TempHome, core: &Core) -> Vec<DriftItem> {
    let doctor = core.doctor().unwrap();
    let b = doctor
        .instances
        .iter()
        .find(|i| i.config_dir == home.path().join(".claude-b"))
        .expect("the linked instance in the doctor's report");
    b.items
        .iter()
        .filter(|item| item.id.starts_with("launcher:"))
        .cloned()
        .collect()
}

#[test]
fn the_doctor_reports_a_missing_or_stale_launcher_as_drift_that_sync_repairs() {
    let (home, core, _env) = home_with_b(OsKind::Linux);
    assert!(launcher_drift(&home, &core).is_empty());

    fs::write(bashrc(&home), "# cleaned up by hand\n").unwrap();
    let missing = launcher_drift(&home, &core);
    assert_eq!(missing.len(), 1);
    let item = &missing[0];
    assert_eq!(item.id, "launcher:claude-b");
    assert_eq!(item.item, "claude-b");
    assert_eq!(item.path, bashrc(&home));
    assert_eq!(item.problem, Problem::MissingLauncher);
    assert_eq!(item.repair, Repair::SyncLaunchers);
    assert!(item.explanation.contains("claude-b"));
    assert!(item.repair_explanation.contains("mccboard launcher sync"));
    assert!(core.doctor().unwrap().has_drift());
    // The same item when asking about the one instance.
    let drift = core.drift(&home.path().join(".claude-b")).unwrap();
    assert!(drift.items.contains(item));

    core.sync_launchers().unwrap();
    point_the_alias_elsewhere(&home);
    let stale = launcher_drift(&home, &core);
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].id, "launcher:claude-b");
    assert_eq!(
        stale[0].problem,
        Problem::StaleLauncher {
            found: Some(home.path().join(".claude-x"))
        }
    );
    assert!(stale[0].explanation.contains(".claude-x"));

    core.sync_launchers().unwrap();
    assert!(launcher_drift(&home, &core).is_empty());
}

#[test]
fn launchers_that_cant_be_read_leave_the_doctor_working() {
    let (home, core, _env) = home_with_b(OsKind::Linux);
    fs::write(bashrc(&home), format!("{BEGIN}\nalias claude-b='x'\n")).unwrap();

    assert!(core.launchers().is_err());
    let doctor = core.doctor().unwrap();
    assert!(
        doctor
            .instances
            .iter()
            .flat_map(|i| &i.items)
            .all(|item| !item.id.starts_with("launcher:"))
    );
}

#[test]
fn repair_fixes_a_launcher_item_by_syncing_the_launchers() {
    let (home, core, env) = home_with_b(OsKind::Linux);
    let b = home.path().join(".claude-b");
    fs::write(bashrc(&home), "# cleaned up by hand\n").unwrap();
    assert_eq!(launcher_drift(&home, &core).len(), 1);
    // Launchers live outside the instance's folder: a running session is no obstacle.
    env.add_process(ClaudeProcess {
        pid: 7,
        name: "claude".into(),
        config_dir: ProcessConfigDir::Dir { path: b.clone() },
        cwd: None,
    });

    let report = core.repair(&b, Some("launcher:claude-b")).unwrap();

    assert_eq!(report.items.len(), 1);
    let item = &report.items[0];
    assert_eq!(item.id, "launcher:claude-b");
    assert_eq!(item.repair, Repair::SyncLaunchers);
    assert_eq!(item.outcome, RepairOutcome::Done { backup: None });
    assert_eq!(
        block_aliases(&read(&bashrc(&home))),
        vec![alias("claude-b", &b)]
    );
    assert!(launcher_drift(&home, &core).is_empty());
    assert!(
        report
            .drift
            .items
            .iter()
            .all(|i| !i.id.starts_with("launcher:"))
    );
    // Repaired already: the item is gone.
    assert!(matches!(
        core.repair(&b, Some("launcher:claude-b")),
        Err(Error::DriftItemNotFound { .. })
    ));
}

#[test]
fn repairing_every_item_of_an_instance_fixes_its_stale_launcher_too() {
    let (home, core, _env) = home_with_b(OsKind::Linux);
    let b = home.path().join(".claude-b");
    point_the_alias_elsewhere(&home);

    let report = core.repair(&b, None).unwrap();

    let launcher = report
        .items
        .iter()
        .find(|i| i.id == "launcher:claude-b")
        .expect("the launcher item among the repaired ones");
    assert_eq!(launcher.outcome, RepairOutcome::Done { backup: None });
    assert_eq!(
        block_aliases(&read(&bashrc(&home))),
        vec![alias("claude-b", &b)]
    );
    assert!(launcher_drift(&home, &core).is_empty());
    assert_eq!(core.repair(&b, None).unwrap().items, vec![]);
}

#[test]
fn a_launcher_no_instance_wants_is_an_orphan_and_sync_removes_it() {
    let (home, core, _env) = home_with_b(OsKind::Linux);
    let text = read(&bashrc(&home)).replace(
        END,
        &format!("alias claude-old='CLAUDE_CONFIG_DIR=/gone claude'\n{END}"),
    );
    fs::write(bashrc(&home), text).unwrap();

    let status = core.launchers().unwrap();
    assert_eq!(
        status.orphans,
        vec![Launcher {
            name: "claude-old".into(),
            config_dir: Some(PathBuf::from("/gone")),
        }]
    );
    assert_eq!(status.drift_count(), 1);

    let sync = core.sync_launchers().unwrap();
    assert_eq!(sync.removed.len(), 1);
    assert_eq!(
        block_aliases(&read(&bashrc(&home))),
        vec![alias("claude-b", &home.path().join(".claude-b"))]
    );
}

#[test]
fn syncing_twice_changes_nothing_the_second_time() {
    let (home, core, _env) = home_with_b(OsKind::Linux);
    let before = read(&bashrc(&home));

    let sync = core.sync_launchers().unwrap();

    assert!(sync.created.is_empty() && sync.updated.is_empty() && sync.removed.is_empty());
    assert_eq!(read(&bashrc(&home)), before);
}

#[test]
fn an_unfinished_block_is_reported_and_never_rewritten() {
    let home = TempHome::new();
    let broken = format!("# mine\n{BEGIN}\nalias claude-b='x'\n");
    home.write(".bashrc", &broken);
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();

    let error = core
        .register(&home.path().join(".claude-b"))
        .expect_err("the launchers can't be written");

    assert_eq!(error.code(), "launcher_block_corrupt");
    assert_eq!(error.path(), Some(bashrc(&home).as_path()));
    assert_eq!(read(&bashrc(&home)), broken);
    assert_eq!(
        core.launchers().unwrap_err().code(),
        "launcher_block_corrupt"
    );
    // The registration itself was saved; only the launcher is missing.
    assert_eq!(core.registry().unwrap().instances.len(), 1);
}

#[cfg(unix)]
#[test]
fn an_rc_file_that_is_a_link_is_edited_where_it_points() {
    let home = TempHome::new();
    home.write("dotfiles/bashrc", "# managed in dotfiles\n");
    std::os::unix::fs::symlink(home.path().join("dotfiles/bashrc"), bashrc(&home)).unwrap();
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();

    core.register(&home.path().join(".claude-b")).unwrap();

    assert!(fs::symlink_metadata(bashrc(&home)).unwrap().is_symlink());
    assert_eq!(
        block_aliases(&read(&home.path().join("dotfiles/bashrc"))).len(),
        1
    );
}

/// Arguments pass through unchanged: the alias, run by a real bash against a stand-in
/// `claude` that prints what it got.
#[cfg(unix)]
#[test]
fn the_alias_passes_every_argument_through_in_bash() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    if Command::new("bash").arg("-c").arg("true").status().is_err() {
        eprintln!("bash not found; skipping");
        return;
    }
    let home = TempHome::new();
    let b = home.linked("it's b");
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();
    let fake_bin = home.path().join("fake-bin");
    fs::create_dir(&fake_bin).unwrap();
    let claude = fake_bin.join("claude");
    // Written under another name and copied by a child `cp`: a descriptor this process held
    // open on `claude` itself could leak into a child forked by a test on another thread, and
    // bash would then fail to run `claude` with ETXTBSY ("Text file busy").
    let source = home.path().join("claude.sh");
    fs::write(
        &source,
        "#!/bin/sh\nprintf 'dir=%s\\n' \"$CLAUDE_CONFIG_DIR\"\nfor a in \"$@\"; do printf 'arg=%s\\n' \"$a\"; done\n",
    )
    .unwrap();
    let copied = Command::new("cp")
        .arg(&source)
        .arg(&claude)
        .status()
        .unwrap();
    assert!(copied.success());
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    let name = &core.launchers().unwrap().instances[0].name;

    let script = format!(
        "shopt -s expand_aliases\nsource \"$1\"\n{name} --bg --name 'x y'\n{name} agents\n"
    );
    let out = Command::new("bash")
        .arg("-c")
        .arg(script)
        .arg("bash")
        .arg(bashrc(&home))
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .unwrap();

    let stdout = String::from_utf8(out.stdout).unwrap();
    let dir = format!("dir={}", b.dir.display());
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        vec![
            dir.as_str(),
            "arg=--bg",
            "arg=--name",
            "arg=x y",
            dir.as_str(),
            "arg=agents"
        ],
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// --- Windows: a copy of the shim and its sidecar in mccboard's bin folder, on the user PATH ---

/// The sidecar mccboard writes for `config_dir`, as the shim reads it.
fn sidecar_line(config_dir: &Path) -> String {
    format!("CLAUDE_CONFIG_DIR={}", config_dir.display())
}

/// The files in the bin folder, sorted.
fn bin_files(home: &TempHome) -> Vec<String> {
    let mut files: Vec<_> = fs::read_dir(bin(home))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    files
}

/// The `.cmd` and `.ps1` scripts an earlier version of mccboard wrote for `config_dir`.
fn write_legacy_scripts(home: &TempHome, name: &str, config_dir: &Path) {
    let dir = config_dir.display().to_string();
    fs::write(
        bin(home).join(format!("{name}.cmd")),
        format!(
            "@echo off\r\nrem Managed by mccboard: starts Claude Code in the instance below. \
             Changes here are overwritten.\r\nsetlocal\r\nset \"CLAUDE_CONFIG_DIR={}\"\r\n\
             claude %*\r\n",
            dir.replace('%', "%%")
        ),
    )
    .unwrap();
    fs::write(
        bin(home).join(format!("{name}.ps1")),
        format!(
            "\u{feff}# Managed by mccboard: starts Claude Code in the instance below. Changes \
             here are overwritten.\r\n$previous = $env:CLAUDE_CONFIG_DIR\r\n\
             $env:CLAUDE_CONFIG_DIR = '{}'\r\ntry {{\r\n    & claude @args\r\n}} finally {{\r\n\
             \x20   $env:CLAUDE_CONFIG_DIR = $previous\r\n}}\r\nexit $LASTEXITCODE\r\n",
            dir.replace('\'', "''")
        ),
    )
    .unwrap();
}

#[test]
fn on_windows_a_linked_instance_gets_a_copy_of_the_shim_and_its_sidecar() {
    let (home, core, env) = home_with_b(OsKind::Windows);
    let b = home.path().join(".claude-b");

    assert_eq!(bin_files(&home), vec!["claude-b.exe", "claude-b.shim"]);
    assert_eq!(
        fs::read(bin(&home).join("claude-b.exe")).unwrap(),
        FAKE_LAUNCHER_SHIM
    );
    let sidecar = read(&bin(&home).join("claude-b.shim"));
    assert!(sidecar.contains("Managed by mccboard"), "{sidecar}");
    assert!(
        sidecar.lines().any(|line| line == sidecar_line(&b)),
        "{sidecar}"
    );
    assert!(!bashrc(&home).exists());
    assert_eq!(env.user_path(), vec![bin(&home)]);

    let status = core.launchers().unwrap();
    assert_eq!(
        status.place,
        LauncherPlace::BinFolder {
            path: bin(&home),
            on_user_path: true
        }
    );
    assert_eq!(status.instances[0].state, LauncherState::Ok);
    assert_eq!(status.drift_count(), 0);
}

#[test]
fn on_windows_the_bin_folder_is_added_to_the_user_path_once() {
    let (home, core, env) = home_with_b(OsKind::Windows);
    let c = home.linked(".claude-c");
    c.logged_in_as(UUID_C, "c@example.com");

    core.register(&c.dir).unwrap();
    let sync = core.sync_launchers().unwrap();

    assert_eq!(env.user_path_additions(), 1);
    assert_eq!(sync.user_path, Some(UserPathChange::AlreadyOn));
}

#[test]
fn on_windows_renaming_and_removing_replace_the_launchers_and_leave_other_files() {
    let (home, core, _env) = home_with_b(OsKind::Windows);
    fs::write(bin(&home).join("notes.txt"), "mine").unwrap();
    fs::write(bin(&home).join("other.cmd"), "@echo not mccboard's").unwrap();
    fs::write(bin(&home).join("tool.exe"), "MZ not mccboard's").unwrap();

    core.update_account(
        &uuid(UUID_B),
        AccountUpdate {
            launcher: Some(Some("cb".into())),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        bin_files(&home),
        vec!["cb.exe", "cb.shim", "notes.txt", "other.cmd", "tool.exe"]
    );

    core.unregister(&home.path().join(".claude-b")).unwrap();
    assert_eq!(bin_files(&home), vec!["notes.txt", "other.cmd", "tool.exe"]);
}

#[test]
fn on_windows_a_missing_program_or_sidecar_is_stale_and_sync_rewrites_it() {
    let (home, core, _env) = home_with_b(OsKind::Windows);
    let b = home.path().join(".claude-b");

    for file in ["claude-b.exe", "claude-b.shim"] {
        fs::remove_file(bin(&home).join(file)).unwrap();
        let status = core.launchers().unwrap();
        assert_eq!(
            status.instances[0].state,
            LauncherState::Stale { found: None },
            "{file}"
        );
        assert_eq!(status.drift_count(), 1);
        let sync = core.sync_launchers().unwrap();
        assert_eq!(sync.updated.len(), 1);
        assert_eq!(bin_files(&home), vec!["claude-b.exe", "claude-b.shim"]);
        assert_eq!(core.launchers().unwrap().drift_count(), 0);
    }

    fs::remove_file(bin(&home).join("claude-b.exe")).unwrap();
    fs::remove_file(bin(&home).join("claude-b.shim")).unwrap();
    assert_eq!(
        core.launchers().unwrap().instances[0].state,
        LauncherState::Missing
    );
    assert_eq!(core.sync_launchers().unwrap().created.len(), 1);
    assert!(
        read(&bin(&home).join("claude-b.shim"))
            .lines()
            .any(|line| line == sidecar_line(&b))
    );
}

#[test]
fn on_windows_a_sidecar_pointing_elsewhere_is_stale_and_says_where_it_points() {
    let (home, core, _env) = home_with_b(OsKind::Windows);
    let b = home.path().join(".claude-b");
    let x = home.path().join(".claude-x");
    let sidecar = bin(&home).join("claude-b.shim");
    fs::write(
        &sidecar,
        read(&sidecar).replace(&sidecar_line(&b), &sidecar_line(&x)),
    )
    .unwrap();

    assert_eq!(
        core.launchers().unwrap().instances[0].state,
        LauncherState::Stale { found: Some(x) }
    );
    core.sync_launchers().unwrap();
    assert_eq!(core.launchers().unwrap().drift_count(), 0);
    assert!(read(&sidecar).lines().any(|line| line == sidecar_line(&b)));
}

/// After an upgrade, mccboard ships another shim: every launcher's copy is out of date until a
/// sync replaces it.
#[test]
fn on_windows_a_program_other_than_the_shipped_shim_is_stale_and_sync_replaces_it() {
    let (home, core, env) = home_with_b(OsKind::Windows);
    let sidecar_before = read(&bin(&home).join("claude-b.shim"));
    env.set_launcher_shim(Some(b"MZ fake mccboard-shim.exe, version 2"));

    let status = core.launchers().unwrap();
    assert_eq!(
        status.instances[0].state,
        LauncherState::Stale { found: None }
    );
    let doctor = launcher_drift(&home, &core);
    assert_eq!(doctor.len(), 1);
    assert_eq!(doctor[0].problem, Problem::StaleLauncher { found: None });

    core.sync_launchers().unwrap();
    assert_eq!(
        fs::read(bin(&home).join("claude-b.exe")).unwrap(),
        b"MZ fake mccboard-shim.exe, version 2"
    );
    assert_eq!(read(&bin(&home).join("claude-b.shim")), sidecar_before);
    assert_eq!(core.launchers().unwrap().drift_count(), 0);
}

/// Launchers written as `.cmd` and `.ps1` scripts by an earlier version: stale for the doctor
/// until a sync (or a repair) replaces them with the program.
#[test]
fn on_windows_the_scripts_of_an_earlier_version_are_stale_and_sync_migrates_them() {
    let (home, core, _env) = home_with_b(OsKind::Windows);
    let b = home.path().join(".claude-b");
    let c = home.linked(".claude-c");
    c.logged_in_as(UUID_C, "c@example.com");
    core.register(&c.dir).unwrap();
    // As an earlier version left the folder: scripts only, one pointing elsewhere, one for an
    // instance gone since. And a script of the user's own.
    for file in bin_files(&home) {
        fs::remove_file(bin(&home).join(file)).unwrap();
    }
    write_legacy_scripts(&home, "claude-b", &b);
    write_legacy_scripts(&home, "claude-c", &home.path().join("it's 100% x"));
    write_legacy_scripts(&home, "claude-old", &home.path().join(".claude-old"));
    fs::write(bin(&home).join("mine.cmd"), "@echo mine\r\n").unwrap();

    let status = core.launchers().unwrap();
    let states: Vec<_> = status.instances.iter().map(|i| i.state.clone()).collect();
    assert_eq!(
        states,
        vec![
            LauncherState::Stale { found: None },
            LauncherState::Stale {
                found: Some(home.path().join("it's 100% x"))
            },
        ]
    );
    assert_eq!(
        status.orphans,
        vec![Launcher {
            name: "claude-old".into(),
            config_dir: Some(home.path().join(".claude-old")),
        }]
    );
    assert_eq!(status.drift_count(), 3);
    let doctor = launcher_drift(&home, &core);
    assert_eq!(doctor.len(), 1);
    assert_eq!(doctor[0].id, "launcher:claude-b");
    assert_eq!(doctor[0].problem, Problem::StaleLauncher { found: None });
    assert!(doctor[0].explanation.contains("earlier version"));

    let sync = core.sync_launchers().unwrap();

    assert_eq!(sync.updated.len(), 2);
    assert_eq!(sync.removed.len(), 1);
    assert_eq!(
        bin_files(&home),
        vec![
            "claude-b.exe",
            "claude-b.shim",
            "claude-c.exe",
            "claude-c.shim",
            "mine.cmd"
        ]
    );
    assert_eq!(core.launchers().unwrap().drift_count(), 0);
}

/// PowerShell picks `claude-b.ps1` over `claude-b.exe` in the same folder: an earlier
/// version's script left next to the program still makes the launcher stale.
#[test]
fn on_windows_a_script_of_an_earlier_version_next_to_the_program_is_stale() {
    let (home, core, _env) = home_with_b(OsKind::Windows);
    write_legacy_scripts(&home, "claude-b", &home.path().join(".claude-b"));
    fs::remove_file(bin(&home).join("claude-b.cmd")).unwrap();

    assert_eq!(
        core.launchers().unwrap().instances[0].state,
        LauncherState::Stale { found: None }
    );
    core.sync_launchers().unwrap();
    assert_eq!(bin_files(&home), vec!["claude-b.exe", "claude-b.shim"]);
    assert_eq!(core.launchers().unwrap().drift_count(), 0);
}

#[test]
fn on_windows_repair_migrates_the_scripts_of_an_earlier_version() {
    let (home, core, _env) = home_with_b(OsKind::Windows);
    let b = home.path().join(".claude-b");
    for file in bin_files(&home) {
        fs::remove_file(bin(&home).join(file)).unwrap();
    }
    write_legacy_scripts(&home, "claude-b", &b);

    let report = core.repair(&b, Some("launcher:claude-b")).unwrap();

    assert_eq!(
        report.items[0].outcome,
        RepairOutcome::Done { backup: None }
    );
    assert_eq!(bin_files(&home), vec!["claude-b.exe", "claude-b.shim"]);
    assert!(launcher_drift(&home, &core).is_empty());
}

/// A user's own scripts with a launcher's name are theirs: never removed, and no reason to call
/// the launcher stale.
#[test]
fn on_windows_scripts_mccboard_didnt_write_are_left_alone() {
    let (home, core, _env) = home_with_b(OsKind::Windows);
    fs::write(bin(&home).join("claude-b.cmd"), "@echo mine\r\n").unwrap();

    assert_eq!(
        core.launchers().unwrap().instances[0].state,
        LauncherState::Ok
    );
    core.sync_launchers().unwrap();
    assert_eq!(
        bin_files(&home),
        vec!["claude-b.cmd", "claude-b.exe", "claude-b.shim"]
    );
}

#[test]
fn on_windows_a_missing_shim_is_a_clear_error_and_the_status_still_reads() {
    let home = TempHome::new();
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    let (core, env) = home.core();
    env.set_os(OsKind::Windows);
    env.set_launcher_shim(None);

    let error = core
        .register(&home.path().join(".claude-b"))
        .expect_err("no launcher can be written");

    assert_eq!(error.code(), "launcher_shim_missing");
    assert!(error.to_string().contains("mccboard-shim"), "{error}");
    // The registration itself was saved; only the launcher is missing.
    assert_eq!(core.registry().unwrap().instances.len(), 1);
    let status = core.launchers().unwrap();
    assert_eq!(status.instances[0].state, LauncherState::Missing);
    assert_eq!(launcher_drift(&home, &core).len(), 1);

    // Launchers written before keep working, and their sidecars can still be fixed.
    env.set_launcher_shim(Some(FAKE_LAUNCHER_SHIM));
    core.sync_launchers().unwrap();
    env.set_launcher_shim(None);
    assert_eq!(core.launchers().unwrap().drift_count(), 0);
    fs::remove_file(bin(&home).join("claude-b.shim")).unwrap();
    core.sync_launchers().unwrap();
    assert_eq!(core.launchers().unwrap().drift_count(), 0);
    assert_eq!(bin_files(&home), vec!["claude-b.exe", "claude-b.shim"]);
}

/// A launcher program that was running when it had to be replaced is moved aside
/// (`claude-b.exe.old`); a later sync deletes what is left aside.
#[test]
fn on_windows_programs_moved_aside_are_deleted_by_a_later_sync() {
    let (home, core, _env) = home_with_b(OsKind::Windows);
    fs::write(bin(&home).join("claude-b.exe.old"), "MZ").unwrap();
    fs::write(bin(&home).join("claude-c.EXE.OLD-12"), "MZ").unwrap();
    // Only names ending as a moved-aside program does.
    fs::write(bin(&home).join("notes.exe.older.txt"), "mine").unwrap();
    fs::write(bin(&home).join("keep.exe.old-x"), "mine").unwrap();

    let sync = core.sync_launchers().unwrap();

    assert!(sync.created.is_empty() && sync.updated.is_empty() && sync.removed.is_empty());
    assert_eq!(
        bin_files(&home),
        vec![
            "claude-b.exe",
            "claude-b.shim",
            "keep.exe.old-x",
            "notes.exe.older.txt"
        ]
    );
}

/// A launcher whose name ends in `.exe.old` is a launcher, not a program moved aside: it
/// survives every sync.
#[test]
fn on_windows_a_launcher_named_like_a_program_moved_aside_is_kept() {
    let (home, core, _env) = home_with_b(OsKind::Windows);
    core.update_account(
        &uuid(UUID_B),
        AccountUpdate {
            launcher: Some(Some("cb.exe.old".into())),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(bin_files(&home), vec!["cb.exe.old.exe", "cb.exe.old.shim"]);

    for _ in 0..2 {
        let sync = core.sync_launchers().unwrap();
        assert!(sync.created.is_empty() && sync.updated.is_empty() && sync.removed.is_empty());
        assert_eq!(bin_files(&home), vec!["cb.exe.old.exe", "cb.exe.old.shim"]);
        assert_eq!(core.launchers().unwrap().drift_count(), 0);
    }
}

#[test]
fn on_windows_a_user_path_that_cant_be_changed_is_reported_not_fatal() {
    let home = TempHome::new();
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    let (core, env) = home.core();
    env.set_os(OsKind::Windows);
    env.set_user_path_fails(true);

    core.register(&home.path().join(".claude-b")).unwrap();
    let sync = core.sync_launchers().unwrap();

    assert!(matches!(
        sync.user_path,
        Some(UserPathChange::NotAdded { .. })
    ));
    let status = core.launchers().unwrap();
    assert_eq!(
        status.place,
        LauncherPlace::BinFolder {
            path: bin(&home),
            on_user_path: false
        }
    );
    assert_eq!(status.instances[0].state, LauncherState::Ok);
    assert_eq!(status.drift_count(), 1);
}

#[test]
fn no_token_reaches_a_launcher() {
    for os in [OsKind::Linux, OsKind::Windows] {
        let (home, core, _env) = home_with_b(os);
        home.primary()
            .credentials_expiring(common::now() + jiff::SignedDuration::from_hours(5));
        core.sync_launchers().unwrap();
        let rc = fs::read_to_string(bashrc(&home)).unwrap_or_default();
        assert!(!rc.contains("FIXTURE"));
        if let Ok(entries) = fs::read_dir(bin(&home)) {
            for entry in entries {
                let bytes = fs::read(entry.unwrap().path()).unwrap();
                assert!(!String::from_utf8_lossy(&bytes).contains("FIXTURE"));
            }
        }
        let json = serde_json::to_string(&core.launchers().unwrap()).unwrap();
        assert!(!json.contains("FIXTURE"));
    }
}

// --- Windows: mccboard's own CLI, `mccboard.exe`, copied into the bin folder ---

const CLI_V1: &[u8] = b"MZ fake mccboard.exe, version 1";
const CLI_V2: &[u8] = b"MZ fake mccboard.exe, version 2";

/// Ships `bytes` as mccboard's CLI, in a folder of the temporary home playing the app's
/// install folder, and returns that folder.
fn ship_cli(home: &TempHome, env: &FakeEnvironment, bytes: &[u8]) -> PathBuf {
    let folder = home.path().join("Programs").join("mccboard");
    fs::create_dir_all(&folder).unwrap();
    let cli = folder.join("mccboard.exe");
    fs::write(&cli, bytes).unwrap();
    env.set_shipped_cli(Some(&cli));
    folder
}

#[test]
fn on_windows_sync_copies_the_shipped_cli_into_the_bin_folder_and_only_when_it_differs() {
    let (home, core, env) = home_with_b(OsKind::Windows);
    let folder = ship_cli(&home, &env, CLI_V1);

    let sync = core.sync_launchers().unwrap();

    assert_eq!(sync.cli, Some(CliInstall::Copied));
    let copy = bin(&home).join("mccboard.exe");
    assert_eq!(fs::read(&copy).unwrap(), CLI_V1);
    // Beside it, the folder it came from: where the copy finds the app and the shim.
    assert_eq!(
        read(&bin(&home).join("mccboard.origin")).trim_end(),
        folder.display().to_string()
    );
    assert_eq!(
        bin_files(&home),
        vec![
            "claude-b.exe",
            "claude-b.shim",
            "mccboard.exe",
            "mccboard.origin"
        ]
    );
    // The CLI is no launcher: nothing to fix, nothing orphaned.
    let status = core.launchers().unwrap();
    assert!(status.orphans.is_empty());
    assert_eq!(status.drift_count(), 0);

    // Identical: left alone, not even rewritten.
    let long_ago = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1 << 30);
    fs::File::options()
        .write(true)
        .open(&copy)
        .unwrap()
        .set_modified(long_ago)
        .unwrap();
    let sync = core.sync_launchers().unwrap();
    assert_eq!(sync.cli, Some(CliInstall::UpToDate));
    assert_eq!(fs::metadata(&copy).unwrap().modified().unwrap(), long_ago);

    // An upgrade ships another CLI: the next sync rewrites the copy.
    ship_cli(&home, &env, CLI_V2);
    let sync = core.sync_launchers().unwrap();
    assert_eq!(sync.cli, Some(CliInstall::Copied));
    assert_eq!(fs::read(&copy).unwrap(), CLI_V2);
    assert!(sync.created.is_empty() && sync.updated.is_empty() && sync.removed.is_empty());
}

#[test]
fn on_windows_a_copy_of_the_cli_changed_by_hand_is_rewritten() {
    let (home, core, env) = home_with_b(OsKind::Windows);
    ship_cli(&home, &env, CLI_V1);
    fs::write(bin(&home).join("mccboard.exe"), "MZ something else").unwrap();

    assert_eq!(core.sync_launchers().unwrap().cli, Some(CliInstall::Copied));
    assert_eq!(fs::read(bin(&home).join("mccboard.exe")).unwrap(), CLI_V1);
}

/// A program without the CLI next to it (a development build of the app alone): the sync
/// still does its work, and the bin folder's copy, if any, stays as it is.
#[test]
fn on_windows_a_cli_not_shipped_leaves_the_copy_alone_and_the_sync_works() {
    let (home, core, env) = home_with_b(OsKind::Windows);
    fs::write(bin(&home).join("mccboard.exe"), CLI_V1).unwrap();
    env.set_shipped_cli(None);
    fs::remove_file(bin(&home).join("claude-b.exe")).unwrap();

    let sync = core.sync_launchers().unwrap();

    assert!(
        matches!(sync.cli, Some(CliInstall::NotCopied { .. })),
        "{sync:?}"
    );
    assert_eq!(sync.updated.len(), 1);
    assert_eq!(fs::read(bin(&home).join("mccboard.exe")).unwrap(), CLI_V1);
}

/// The copy in the bin folder, running the sync itself without knowing where it came from:
/// it is the CLI to install, so nothing is copied.
#[test]
fn on_windows_the_copy_in_the_bin_folder_is_never_copied_onto_itself() {
    let (home, core, env) = home_with_b(OsKind::Windows);
    let copy = bin(&home).join("mccboard.exe");
    fs::write(&copy, CLI_V1).unwrap();
    env.set_shipped_cli(Some(&copy));

    assert_eq!(
        core.sync_launchers().unwrap().cli,
        Some(CliInstall::UpToDate)
    );
    assert_eq!(fs::read(&copy).unwrap(), CLI_V1);
    assert!(!bin(&home).join("mccboard.origin").exists());
}

/// A running `mccboard.exe` can't be overwritten or deleted: it is moved aside, as a running
/// launcher is, and a later sync deletes the copy moved aside.
#[cfg(windows)]
#[test]
fn on_windows_a_running_cli_is_moved_aside_and_a_later_sync_deletes_it() {
    use std::process::{Command, Stdio};

    let (home, core, env) = home_with_b(OsKind::Windows);
    // A real program, kept running while its stdin stays open.
    let copy = bin(&home).join("mccboard.exe");
    let system = std::env::var_os("SystemRoot").expect("SystemRoot");
    fs::copy(Path::new(&system).join("System32").join("cmd.exe"), &copy).unwrap();
    let mut running = Command::new(&copy)
        .arg("/q")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert!(
        fs::remove_file(&copy).is_err(),
        "a running program can't be deleted"
    );
    ship_cli(&home, &env, CLI_V1);

    let sync = core.sync_launchers();

    // Before any assertion, so a failure doesn't leave the program running.
    running.kill().unwrap();
    running.wait().unwrap();
    assert_eq!(sync.unwrap().cli, Some(CliInstall::Copied));
    assert_eq!(fs::read(&copy).unwrap(), CLI_V1);
    assert!(bin(&home).join("mccboard.exe.old").is_file());

    core.sync_launchers().unwrap();
    assert!(!bin(&home).join("mccboard.exe.old").exists());
    assert_eq!(fs::read(&copy).unwrap(), CLI_V1);
}

/// The app copies the CLI when it starts, so an update of the app refreshes it, and deletes
/// what earlier syncs moved aside. The launchers are left to the sync.
#[test]
fn on_windows_installing_the_cli_copies_it_without_touching_the_launchers() {
    let (home, core, env) = home_with_b(OsKind::Windows);
    fs::remove_file(bin(&home).join("claude-b.exe")).unwrap();
    fs::write(bin(&home).join("mccboard.exe.old"), "MZ").unwrap();
    ship_cli(&home, &env, CLI_V1);

    assert_eq!(core.install_cli(), Some(CliInstall::Copied));
    assert_eq!(core.install_cli(), Some(CliInstall::UpToDate));

    assert_eq!(
        bin_files(&home),
        vec!["claude-b.shim", "mccboard.exe", "mccboard.origin"]
    );
}

/// The bin folder is made for the CLI even when no launcher needs it yet.
#[test]
fn on_windows_installing_the_cli_creates_the_bin_folder() {
    let home = TempHome::new();
    let (core, env) = home.core();
    env.set_os(OsKind::Windows);
    ship_cli(&home, &env, CLI_V1);

    assert_eq!(core.install_cli(), Some(CliInstall::Copied));
    assert_eq!(bin_files(&home), vec!["mccboard.exe", "mccboard.origin"]);
}

#[test]
fn on_linux_the_cli_is_not_copied_anywhere() {
    let (home, core, env) = home_with_b(OsKind::Linux);
    ship_cli(&home, &env, CLI_V1);

    assert_eq!(core.install_cli(), None);
    assert_eq!(core.sync_launchers().unwrap().cli, None);
    assert!(!bin(&home).exists());
}

/// `mccboard` is the CLI's name in the bin folder: no launcher may take it.
#[test]
fn mccboard_is_refused_as_a_launcher_name() {
    let (_home, core, _env) = home_with_b(OsKind::Windows);
    for name in ["mccboard", "MccBoard"] {
        let error = core
            .update_account(
                &uuid(UUID_B),
                AccountUpdate {
                    launcher: Some(Some(name.into())),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(error.code(), "invalid_launcher", "{name:?}");
    }
}
