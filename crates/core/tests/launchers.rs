//! Launchers: one command per linked instance, `claude-b`, made from the account label
//! (spec stories 5, 25–29; ticket #7).
//!
//! Every test runs on a temporary home with the fake port: the shell rc file is the temporary
//! home's, and the user PATH is the fake's, so the real user environment is never touched.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{TempHome, UUID_A, UUID_B, UUID_C};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    AccountUpdate, AccountUuid, ClaudeProcess, Core, DriftItem, Error, Launcher, LauncherPlace,
    LauncherState, OsKind, Problem, ProcessConfigDir, Repair, RepairOutcome, UserPathChange,
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

fn alias(name: &str, dir: &Path) -> String {
    format!("alias {name}='CLAUDE_CONFIG_DIR={} claude'", dir.display())
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
    let text = read(&bashrc(&home)).replace(".claude-b claude", ".claude-x claude");
    fs::write(bashrc(&home), text).unwrap();

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
    assert!(!item.informational);
    assert!(item.explanation.contains("claude-b"));
    assert!(item.repair_explanation.contains("mccboard launcher sync"));
    assert!(core.doctor().unwrap().has_drift());
    // The same item when asking about the one instance.
    let drift = core.drift(&home.path().join(".claude-b")).unwrap();
    assert!(drift.items.contains(item));

    core.sync_launchers().unwrap();
    let text = read(&bashrc(&home)).replace(".claude-b claude", ".claude-x claude");
    fs::write(bashrc(&home), text).unwrap();
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
    let text = read(&bashrc(&home)).replace(".claude-b claude", ".claude-x claude");
    fs::write(bashrc(&home), text).unwrap();

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
    fs::write(
        &claude,
        "#!/bin/sh\nprintf 'dir=%s\\n' \"$CLAUDE_CONFIG_DIR\"\nfor a in \"$@\"; do printf 'arg=%s\\n' \"$a\"; done\n",
    )
    .unwrap();
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

// --- Windows: .cmd and .ps1 scripts in mccboard's bin folder, on the user PATH ---

#[test]
fn on_windows_a_linked_instance_gets_cmd_and_ps1_scripts_in_the_bin_folder() {
    let (home, core, env) = home_with_b(OsKind::Windows);
    let b = home.path().join(".claude-b");

    let cmd = read(&bin(&home).join("claude-b.cmd"));
    let ps1 = read(&bin(&home).join("claude-b.ps1"));

    assert!(
        cmd.contains(&format!("set \"CLAUDE_CONFIG_DIR={}\"", b.display())),
        "{cmd}"
    );
    assert!(cmd.contains("claude %*"), "{cmd}");
    assert!(
        ps1.contains(&format!("$env:CLAUDE_CONFIG_DIR = '{}'", b.display())),
        "{ps1}"
    );
    assert!(ps1.contains("& claude @args"), "{ps1}");
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
fn on_windows_renaming_and_removing_replace_the_scripts() {
    let (home, core, _env) = home_with_b(OsKind::Windows);
    fs::write(bin(&home).join("notes.txt"), "mine").unwrap();
    fs::write(bin(&home).join("other.cmd"), "@echo not mccboard's").unwrap();

    core.update_account(
        &uuid(UUID_B),
        AccountUpdate {
            launcher: Some(Some("cb".into())),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(bin(&home).join("cb.cmd").is_file());
    assert!(bin(&home).join("cb.ps1").is_file());
    assert!(!bin(&home).join("claude-b.cmd").exists());
    assert!(!bin(&home).join("claude-b.ps1").exists());

    core.unregister(&home.path().join(".claude-b")).unwrap();
    let mut left: Vec<_> = fs::read_dir(bin(&home))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, vec!["notes.txt", "other.cmd"]);
}

#[test]
fn on_windows_a_missing_script_is_stale_and_sync_rewrites_it() {
    let (home, core, _env) = home_with_b(OsKind::Windows);
    fs::remove_file(bin(&home).join("claude-b.ps1")).unwrap();

    let status = core.launchers().unwrap();
    assert_eq!(
        status.instances[0].state,
        LauncherState::Stale {
            found: Some(home.path().join(".claude-b"))
        }
    );

    core.sync_launchers().unwrap();
    assert!(bin(&home).join("claude-b.ps1").is_file());

    fs::remove_file(bin(&home).join("claude-b.ps1")).unwrap();
    fs::remove_file(bin(&home).join("claude-b.cmd")).unwrap();
    assert_eq!(
        core.launchers().unwrap().instances[0].state,
        LauncherState::Missing
    );
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

/// Arguments pass through unchanged in cmd.exe and PowerShell, against a stand-in `claude`.
#[cfg(windows)]
#[test]
fn the_scripts_pass_every_argument_through_on_windows() {
    use std::process::Command;

    let (home, core, _env) = home_with_b(OsKind::Windows);
    let b = home.path().join(".claude-b");
    let fake_bin = home.path().join("fake-bin");
    fs::create_dir(&fake_bin).unwrap();
    fs::write(
        fake_bin.join("claude.cmd"),
        "@echo off\r\necho dir=%CLAUDE_CONFIG_DIR%\r\necho args=%*\r\n",
    )
    .unwrap();
    let path = format!(
        "{};{};{}",
        fake_bin.display(),
        bin(&home).display(),
        std::env::var("PATH").unwrap_or_default()
    );
    assert_eq!(
        core.launchers().unwrap().instances[0].state,
        LauncherState::Ok
    );

    let out = Command::new("cmd")
        .args(["/C", "claude-b", "--bg", "--name", "x"])
        .env("PATH", &path)
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(&format!("dir={}", b.display())), "{stdout}");
    assert!(stdout.contains("args=--bg --name x"), "{stdout}");

    let out = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(bin(&home).join("claude-b.ps1"))
        .args(["agents", "--bg"])
        .env("PATH", &path)
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(&format!("dir={}", b.display())), "{stdout}");
    assert!(stdout.contains("args=agents --bg"), "{stdout}");
}

#[test]
fn no_token_reaches_a_launcher() {
    let (home, core, _env) = home_with_b(OsKind::Linux);
    home.primary()
        .credentials_expiring(common::now() + jiff::SignedDuration::from_hours(5));
    core.sync_launchers().unwrap();
    assert!(!read(&bashrc(&home)).contains("FIXTURE"));
    let json = serde_json::to_string(&core.launchers().unwrap()).unwrap();
    assert!(!json.contains("FIXTURE"));
}
