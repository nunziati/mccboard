//! Repairing link drift (spec stories 34, 39–42, 48–50; ticket #9): every repair passes the
//! session guard, links in a single hop, backs up real items first, never touches private
//! items, and is idempotent. Runs on temporary homes with real links (junctions on Windows).

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use common::{FIXTURE_TOKEN, TempHome, UUID_A, UUID_B};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    ClaudeProcess, Core, Error, ProcessConfigDir, Repair, RepairOutcome, RepairReport,
};

// ---------------------------------------------------------------------------------------------
// Fixtures

/// Creates a link at `at` pointing to `target`, the way a user would by hand.
fn link(target: impl AsRef<Path>, at: impl AsRef<Path>) {
    let (target, at) = (target.as_ref(), at.as_ref());
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, at).unwrap();
    #[cfg(windows)]
    {
        let resolved = at.parent().unwrap().join(target);
        if resolved.is_dir() {
            std::os::windows::fs::symlink_dir(target, at).unwrap();
        } else {
            std::os::windows::fs::symlink_file(target, at).unwrap();
        }
    }
}

fn write(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap()
}

fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// A link's first hop, as written.
fn first_hop(path: &Path) -> PathBuf {
    let target = fs::read_link(path).unwrap();
    let text = target.to_string_lossy().into_owned();
    // Windows may report a junction's target with the verbatim prefix.
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

/// A primary instance holding every linked item, two personal skills, the account's synced
/// skills, a trashed skill and plugins.
fn full_primary(home: &TempHome) -> PathBuf {
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    let p = primary.dir.clone();
    write(p.join("settings.json"), r#"{"model": "opus"}"#);
    write(p.join("CLAUDE.md"), "# Me\n");
    for dir in ["agents", "hooks", "output-styles", "scripts"] {
        write(p.join(dir).join("README.md"), dir);
    }
    write(p.join("skills/review/SKILL.md"), "---\nname: review\n---\n");
    write(p.join("skills/deploy/SKILL.md"), "---\nname: deploy\n---\n");
    write(
        p.join("skills/synced/org_".to_owned() + UUID_A + "/notes/SKILL.md"),
        "a's synced",
    );
    write(p.join("skills/.trash/old/SKILL.md"), "trashed");
    write(
        p.join("plugins/installed_plugins.json"),
        r#"{"version":2,"plugins":{}}"#,
    );
    write(p.join("projects/-home-me/session.jsonl"), "{}");
    p
}

/// A registered linked instance with nothing linked yet, logged in as B, with private items.
fn bare_linked(home: &TempHome, core: &Core, name: &str) -> PathBuf {
    let linked = home.linked(name);
    linked
        .logged_in_as(UUID_B, "b@example.com")
        .credentials_expiring(common::now() + jiff::SignedDuration::from_hours(5));
    let b = linked.dir.clone();
    write(b.join("history.jsonl"), "b's history");
    write(b.join("projects/-home-me/b.jsonl"), "b's session");
    write(b.join("todos/t.json"), "[]");
    core.register(&b).unwrap();
    b
}

/// The hand-made layout of the Linux mini PC: every linked item linked, but `skills/` and
/// `plugins/` linked as whole folders.
fn mini_pc_linked(home: &TempHome, core: &Core, p: &Path) -> PathBuf {
    let b = bare_linked(home, core, ".claude-b");
    for item in [
        "settings.json",
        "CLAUDE.md",
        "agents",
        "hooks",
        "output-styles",
        "scripts",
        "skills",
        "plugins",
    ] {
        link(p.join(item), b.join(item));
    }
    b
}

fn setup() -> (TempHome, Core, FakeEnvironment, PathBuf) {
    let home = TempHome::new();
    let p = full_primary(&home);
    let (core, env) = home.core();
    core.register(&p).unwrap();
    (home, core, env, p)
}

fn claude(pid: u32, config_dir: ProcessConfigDir) -> ClaudeProcess {
    ClaudeProcess {
        pid,
        name: "claude".into(),
        config_dir,
        cwd: None,
    }
}

fn dir(path: &Path) -> ProcessConfigDir {
    ProcessConfigDir::Dir {
        path: path.to_path_buf(),
    }
}

/// Every path under `root` (links not followed) with what it is: file contents, a link's
/// first hop, or a folder. Two equal snapshots mean nothing changed.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                out.insert(relative, format!("link -> {}", first_hop(&path).display()));
            } else if meta.is_dir() {
                out.insert(relative, "dir".into());
                stack.push(path);
            } else {
                out.insert(relative, format!("file {}", read(&path)));
            }
        }
    }
    out
}

fn done(report: &RepairReport) -> Vec<&str> {
    report
        .items
        .iter()
        .filter(|i| matches!(i.outcome, RepairOutcome::Done { .. }))
        .map(|i| i.id.as_str())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Repairing links

#[test]
fn repairing_a_bare_instance_links_every_item_in_a_single_hop() {
    let (home, core, _env, p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");

    let report = core.repair(&b, None).unwrap();

    assert_eq!(
        done(&report),
        vec![
            "link:settings.json",
            "link:CLAUDE.md",
            "link:agents",
            "link:hooks",
            "link:output-styles",
            "link:scripts",
            "link:skills/deploy",
            "link:skills/review",
        ]
    );
    assert_eq!(report.config_dir, b);
    assert_eq!(report.failed(), 0);
    assert_eq!(report.drift.items, vec![]);
    for item in ["settings.json", "CLAUDE.md", "agents", "scripts"] {
        assert!(is_link(&b.join(item)), "{item}");
        assert_eq!(first_hop(&b.join(item)), p.join(item));
    }
    assert_eq!(
        first_hop(&b.join("skills").join("review")),
        p.join("skills").join("review")
    );
    assert_eq!(read(b.join("settings.json")), r#"{"model": "opus"}"#);
    assert_eq!(read(b.join("agents/README.md")), "agents");
    // skills/ itself is a real folder: only personal skills are linked.
    assert!(!is_link(&b.join("skills")));
    assert!(!b.join("skills/synced").exists());
    assert!(!b.join("skills/.trash").exists());
    assert!(!b.join("plugins").exists());
    assert_eq!(core.drift(&b).unwrap().items, vec![]);
}

#[test]
fn repair_is_idempotent() {
    let (home, core, _env, p) = setup();
    let b = mini_pc_linked(&home, &core, &p);
    fs::remove_file(b.join("CLAUDE.md")).unwrap();
    write(b.join("CLAUDE.md"), "an old copy");
    core.repair(&b, None).unwrap();
    let after_first = (snapshot(home.path()), snapshot(&b));

    let second = core.repair(&b, None).unwrap();

    assert_eq!(second.items, vec![]);
    assert_eq!(second.drift.items, vec![]);
    assert_eq!((snapshot(home.path()), snapshot(&b)), after_first);
}

#[test]
fn one_item_is_repaired_by_its_id_and_nothing_else_changes() {
    let (home, core, _env, p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");

    let report = core.repair(&b, Some("link:agents")).unwrap();

    assert_eq!(done(&report), vec!["link:agents"]);
    assert_eq!(first_hop(&b.join("agents")), p.join("agents"));
    assert!(!b.join("hooks").exists());
    assert!(!b.join("settings.json").exists());
    assert_eq!(report.drift.drift_count, 7);
    assert!(report.drift.items.iter().all(|i| i.id != "link:agents"));
}

#[test]
fn an_id_with_no_drift_is_an_error_that_changes_nothing() {
    let (home, core, _env, _p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    let before = snapshot(&b);

    let error = core.repair(&b, Some("link:nothing-here")).unwrap_err();

    assert!(matches!(error, Error::DriftItemNotFound { .. }), "{error}");
    assert_eq!(error.code(), "drift_item_not_found");
    assert!(error.to_string().contains("link:nothing-here"));
    assert_eq!(snapshot(&b), before);
}

#[test]
fn broken_wrong_and_chained_links_are_replaced_and_their_targets_kept() {
    let (home, core, _env, p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    core.repair(&b, None).unwrap();
    let c = bare_linked(&home, &core, ".claude-c");
    core.repair(&c, None).unwrap();
    // agents leads nowhere, CLAUDE.md to someone else's file, settings.json to b's link.
    write(home.path().join("dotfiles/CLAUDE.md"), "# Elsewhere\n");
    write(home.path().join("dotfiles/agents/a.md"), "elsewhere");
    remove_link(&c.join("agents"));
    link(home.path().join("nowhere"), c.join("agents"));
    remove_link(&c.join("CLAUDE.md"));
    link(home.path().join("dotfiles/CLAUDE.md"), c.join("CLAUDE.md"));
    remove_link(&c.join("settings.json"));
    link(b.join("settings.json"), c.join("settings.json"));
    remove_link(&c.join("skills/review"));
    link(home.path().join("dotfiles/agents"), c.join("skills/review"));

    let report = core.repair(&c, None).unwrap();

    assert_eq!(report.drift.items, vec![]);
    for item in ["agents", "CLAUDE.md", "settings.json"] {
        assert_eq!(first_hop(&c.join(item)), p.join(item), "{item}");
    }
    assert_eq!(
        first_hop(&c.join("skills/review")),
        p.join("skills").join("review")
    );
    for (_, outcome) in report.items.iter().map(|i| (&i.id, &i.outcome)) {
        assert_eq!(outcome, &RepairOutcome::Done { backup: None });
    }
    // Replacing a link never touches what it pointed to.
    assert_eq!(
        read(home.path().join("dotfiles/CLAUDE.md")),
        "# Elsewhere\n"
    );
    assert_eq!(read(home.path().join("dotfiles/agents/a.md")), "elsewhere");
    assert!(is_link(&b.join("settings.json")));
    assert_eq!(read(p.join("settings.json")), r#"{"model": "opus"}"#);
}

#[test]
fn a_real_copy_is_moved_to_a_timestamped_backup_before_linking() {
    let (home, core, _env, p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    write(b.join("settings.json"), r#"{"model": "sonnet"}"#);
    write(b.join("skills/deploy/SKILL.md"), "an old copy");
    write(b.join("skills/deploy/notes.txt"), "my notes");

    let report = core.repair(&b, None).unwrap();

    let backups = home
        .path()
        .join(".mccboard/state/backups/20261015T120000Z/.claude-b");
    let settings = report
        .items
        .iter()
        .find(|i| i.id == "link:settings.json")
        .unwrap();
    assert_eq!(
        settings.outcome,
        RepairOutcome::Done {
            backup: Some(backups.join("settings.json"))
        }
    );
    assert!(matches!(settings.repair, Repair::BackupAndLink { .. }));
    assert_eq!(
        read(backups.join("settings.json")),
        r#"{"model": "sonnet"}"#
    );
    assert_eq!(read(backups.join("skills/deploy/SKILL.md")), "an old copy");
    assert_eq!(read(backups.join("skills/deploy/notes.txt")), "my notes");
    assert_eq!(first_hop(&b.join("settings.json")), p.join("settings.json"));
    assert_eq!(
        first_hop(&b.join("skills/deploy")),
        p.join("skills").join("deploy")
    );
    // The primary's copies are untouched.
    assert_eq!(read(p.join("settings.json")), r#"{"model": "opus"}"#);
    assert_eq!(
        read(p.join("skills/deploy/SKILL.md")),
        "---\nname: deploy\n---\n"
    );
    assert_eq!(report.drift.items, vec![]);
}

#[test]
fn backups_made_in_the_same_second_never_overwrite_each_other() {
    let (home, core, _env, _p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    write(b.join("CLAUDE.md"), "first copy");
    core.repair(&b, Some("link:CLAUDE.md")).unwrap();
    remove_link(&b.join("CLAUDE.md"));
    write(b.join("CLAUDE.md"), "second copy");

    let report = core.repair(&b, Some("link:CLAUDE.md")).unwrap();

    let state = home.path().join(".mccboard/state/backups");
    assert_eq!(
        read(state.join("20261015T120000Z/.claude-b/CLAUDE.md")),
        "first copy"
    );
    let RepairOutcome::Done {
        backup: Some(second),
    } = &report.items[0].outcome
    else {
        panic!("{:?}", report.items)
    };
    assert_eq!(read(second), "second copy");
    assert!(second.starts_with(&state));
}

#[test]
fn private_items_are_never_touched() {
    let (home, core, _env, p) = setup();
    let b = mini_pc_linked(&home, &core, &p);
    let credentials = read(b.join(".credentials.json"));
    let global_config = read(b.join(".claude.json"));
    let primary_before = snapshot(&p);

    core.repair(&b, None).unwrap();

    assert_eq!(read(b.join(".credentials.json")), credentials);
    assert_eq!(read(b.join(".claude.json")), global_config);
    assert_eq!(read(b.join("history.jsonl")), "b's history");
    assert_eq!(read(b.join("projects/-home-me/b.jsonl")), "b's session");
    assert_eq!(read(b.join("todos/t.json")), "[]");
    assert_eq!(snapshot(&p), primary_before);
}

#[test]
fn a_failed_item_is_reported_and_the_rest_still_repaired() {
    let (home, core, _env, p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    // A file named skills: no skill link can go inside it.
    write(b.join("skills"), "not a folder");

    let report = core.repair(&b, None).unwrap();

    assert_eq!(report.failed(), 2);
    for item in &report.items {
        if item.id.starts_with("link:skills/") {
            assert!(
                matches!(item.outcome, RepairOutcome::Failed { .. }),
                "{item:?}"
            );
        } else {
            assert!(
                matches!(item.outcome, RepairOutcome::Done { .. }),
                "{item:?}"
            );
        }
    }
    assert_eq!(first_hop(&b.join("hooks")), p.join("hooks"));
    assert_eq!(read(b.join("skills")), "not a folder");
    assert_eq!(report.drift.drift_count, 2);
}

// ---------------------------------------------------------------------------------------------
// The session guard

#[test]
fn repair_refuses_while_claude_code_uses_the_instance_and_lists_the_processes() {
    let (home, core, env, _p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    env.add_process(ClaudeProcess {
        cwd: Some(home.path().join("work")),
        ..claude(4242, dir(&b))
    });
    let before = snapshot(&b);

    let error = core.repair(&b, None).unwrap_err();

    let Error::InstanceInUse { path, processes } = &error else {
        panic!("{error}")
    };
    assert_eq!(path, &b);
    assert_eq!(processes.len(), 1);
    assert_eq!(processes[0].pid, 4242);
    assert_eq!(error.code(), "instance_in_use");
    assert_eq!(error.processes().unwrap()[0].pid, 4242);
    assert!(error.to_string().contains("4242"), "{error}");
    assert_eq!(snapshot(&b), before);

    env.set_processes(vec![]);
    assert_eq!(core.repair(&b, None).unwrap().drift.items, vec![]);
}

#[test]
fn only_processes_using_that_config_directory_block_it() {
    let (home, core, env, p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    let c = home.linked(".claude-c").dir.clone();
    env.set_processes(vec![
        // The primary instance (no CLAUDE_CONFIG_DIR) and another linked instance.
        claude(1, ProcessConfigDir::Default),
        claude(2, dir(&c)),
        // A process whose environment can't be read and that isn't in b's session registry.
        claude(3, ProcessConfigDir::Unknown),
    ]);

    assert_eq!(core.processes_using(&b).unwrap(), vec![]);
    assert_eq!(
        core.processes_using(&p)
            .unwrap()
            .iter()
            .map(|p| p.pid)
            .collect::<Vec<_>>(),
        vec![1]
    );
    core.repair(&b, None).unwrap();
}

#[test]
fn the_same_folder_spelled_differently_is_matched() {
    let (home, core, env, _p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    let spelled = home.path().join("x").join("..").join(".claude-b");
    env.set_processes(vec![claude(7, dir(&spelled))]);

    assert_eq!(core.processes_using(&b).unwrap().len(), 1);
    assert!(matches!(
        core.repair(&b, None),
        Err(Error::InstanceInUse { .. })
    ));
}

#[test]
fn claude_code_session_registry_identifies_processes_whose_environment_is_unreadable() {
    let (home, core, env, _p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    // Claude Code registers each running session in <config dir>/sessions/<pid>.json.
    write(b.join("sessions/5150.json"), r#"{"pid":5150}"#);
    // A stale registration of a process that is gone.
    write(b.join("sessions/6000.json"), r#"{"pid":6000}"#);
    env.set_processes(vec![claude(5150, ProcessConfigDir::Unknown)]);

    let using = core.processes_using(&b).unwrap();

    assert_eq!(using.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![5150]);
    assert!(matches!(
        core.repair(&b, None),
        Err(Error::InstanceInUse { .. })
    ));
}

#[test]
fn with_nothing_to_repair_a_running_session_is_no_obstacle() {
    let (home, core, env, _p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    core.repair(&b, None).unwrap();
    env.add_process(claude(9, dir(&b)));

    let report = core.repair(&b, None).unwrap();

    assert_eq!(report.items, vec![]);
}

// ---------------------------------------------------------------------------------------------
// Windows Developer Mode and the desktop app

#[test]
fn without_developer_mode_file_links_are_refused_before_anything_changes() {
    let (home, core, env, p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    env.set_developer_mode(false);
    let before = snapshot(&b);

    let error = core.repair(&b, None).unwrap_err();

    assert!(matches!(error, Error::DeveloperModeOff), "{error}");
    assert_eq!(error.code(), "developer_mode_off");
    assert!(error.to_string().contains("Developer Mode"), "{error}");
    assert_eq!(snapshot(&b), before);

    // Folders need no Developer Mode (junctions on Windows).
    let report = core.repair(&b, Some("link:agents")).unwrap();
    assert_eq!(done(&report), vec!["link:agents"]);
    assert_eq!(first_hop(&b.join("agents")), p.join("agents"));

    env.set_developer_mode(true);
    assert_eq!(core.repair(&b, None).unwrap().drift.items, vec![]);
}

#[test]
fn links_are_never_created_inside_the_claude_desktop_app_data_folder() {
    let (home, core, _env, _p) = setup();
    for folder in [
        "AppData/Roaming/Claude/code",
        "AppData/Local/Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude/code",
        "AppData/Local/AnthropicClaude/code",
    ] {
        let at = home.path().join(folder);
        fs::create_dir_all(&at).unwrap();
        core.register(&at).unwrap();
        let before = snapshot(&at);

        let error = core.repair(&at, None).unwrap_err();

        assert!(
            matches!(error, Error::DesktopAppFolder { .. }),
            "{folder}: {error}"
        );
        assert_eq!(error.code(), "desktop_app_folder");
        assert_eq!(snapshot(&at), before, "{folder}");
    }
}

// ---------------------------------------------------------------------------------------------
// What repair never does

#[test]
fn the_primary_instance_is_never_relinked() {
    let (home, core, env, p) = setup();
    env.add_process(claude(1, ProcessConfigDir::Default));
    let before = snapshot(&p);

    let report = core.repair(&p, None).unwrap();

    assert_eq!(report.items, vec![]);
    assert_eq!(snapshot(&p), before);
    let _ = home;
}

#[test]
fn only_registered_instances_are_repaired() {
    let (home, core, _env, _p) = setup();
    let c = home.linked(".claude-c").dir.clone();

    let error = core.repair(&c, None).unwrap_err();

    assert!(matches!(error, Error::NotRegistered { .. }), "{error}");
    assert_eq!(fs::read_dir(&c).unwrap().count(), 0);
}

#[test]
fn no_token_in_reports_or_errors() {
    let (home, core, env, p) = setup();
    home.primary()
        .credentials_expiring(common::now() + jiff::SignedDuration::from_hours(5));
    let b = mini_pc_linked(&home, &core, &p);
    env.add_process(claude(3, dir(&b)));
    let error = core.repair(&b, None).unwrap_err();
    env.set_processes(vec![]);

    let report = core.repair(&b, None).unwrap();

    let text = format!(
        "{error} {error:?} {} {report:?}",
        serde_json::to_string(&report).unwrap()
    );
    assert!(!text.contains(FIXTURE_TOKEN));
    assert!(!text.contains("FIXTURE"));
    let backups = home.path().join(".mccboard/state");
    if backups.exists() {
        for (_, contents) in snapshot(&backups) {
            assert!(!contents.contains("FIXTURE"));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Migrating the hand-made layout (story 34)

#[test]
fn the_mini_pc_layout_is_migrated_to_the_safe_layout() {
    let (home, core, _env, p) = setup();
    let b = mini_pc_linked(&home, &core, &p);
    let primary_before = snapshot(&p);

    let report = core.repair(&b, None).unwrap();

    assert_eq!(done(&report), vec!["migrate:skills", "migrate:plugins"]);
    // skills/: a real folder with one link per personal skill, no synced or trash shared.
    let skills = b.join("skills");
    assert!(!is_link(&skills));
    assert!(skills.is_dir());
    let mut names: Vec<String> = fs::read_dir(&skills)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, vec!["deploy", "review"]);
    assert_eq!(
        first_hop(&skills.join("review")),
        p.join("skills").join("review")
    );
    // plugins/: the instance's own, empty until its plugins are reinstalled.
    assert!(!is_link(&b.join("plugins")));
    assert!(b.join("plugins").is_dir());
    assert_eq!(fs::read_dir(b.join("plugins")).unwrap().count(), 0);
    // The primary keeps everything, its synced skills and plugins included.
    assert_eq!(snapshot(&p), primary_before);
    assert_eq!(report.drift.items, vec![]);
    assert_eq!(core.drift(&b).unwrap().items, vec![]);

    // Idempotent: a second run finds nothing to do.
    let after = snapshot(&b);
    assert_eq!(core.repair(&b, None).unwrap().items, vec![]);
    assert_eq!(snapshot(&b), after);
}

#[test]
fn migration_keeps_the_instance_own_synced_skills_from_the_shared_folder() {
    let (home, core, _env, p) = setup();
    // With skills/ shared, account B's claude.ai skills landed in the primary's synced folder
    // too, next to A's.
    let bucket = format!("org_{UUID_B}");
    write(
        p.join("skills/synced").join(&bucket).join("mine/SKILL.md"),
        "b's synced",
    );
    write(
        p.join("skills/synced").join(format!(".bucket-{bucket}")),
        "",
    );
    let b = mini_pc_linked(&home, &core, &p);

    core.repair(&b, Some("migrate:skills")).unwrap();

    let synced = b.join("skills/synced");
    assert!(!is_link(&synced));
    assert_eq!(
        read(synced.join(&bucket).join("mine/SKILL.md")),
        "b's synced"
    );
    assert!(synced.join(format!(".bucket-{bucket}")).is_file());
    // A's claude.ai skills stay A's.
    assert!(!synced.join(format!("org_{UUID_A}")).exists());
    assert!(p.join("skills/synced").join(&bucket).exists());
}

#[test]
fn migration_keeps_the_synced_skills_of_a_folder_the_instance_had_to_itself() {
    let (home, core, _env, p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");
    // skills/ linked whole to a folder of b's own, outside the primary.
    let own = home.path().join("b-skills");
    write(own.join("synced/org_x/mine/SKILL.md"), "b's synced");
    write(own.join("review/SKILL.md"), "an old review");
    link(&own, b.join("skills"));

    let report = core.repair(&b, None).unwrap();

    assert!(report.items.iter().any(|i| i.id == "migrate:skills"));
    assert_eq!(
        read(b.join("skills/synced/org_x/mine/SKILL.md")),
        "b's synced"
    );
    assert_eq!(
        first_hop(&b.join("skills/review")),
        p.join("skills").join("review")
    );
    // The old folder is left as it was.
    assert_eq!(read(own.join("review/SKILL.md")), "an old review");
    assert_eq!(read(own.join("synced/org_x/mine/SKILL.md")), "b's synced");
    assert_eq!(report.drift.items, vec![]);
}

// ---------------------------------------------------------------------------------------------
// Real links on each OS

#[cfg(windows)]
#[test]
fn on_windows_folders_are_junctions_and_files_symlinks() {
    use std::os::windows::fs::FileTypeExt;

    let (home, core, _env, _p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");

    core.repair(&b, None).unwrap();

    for folder in ["agents", "hooks", "skills/review"] {
        assert!(junction::exists(b.join(folder)).unwrap(), "{folder}");
    }
    let settings = fs::symlink_metadata(b.join("settings.json")).unwrap();
    assert!(settings.file_type().is_symlink_file());
    assert!(!junction::exists(b.join("settings.json")).unwrap_or(false));
}

#[cfg(unix)]
#[test]
fn on_linux_every_link_is_a_symlink_to_an_absolute_path() {
    let (home, core, _env, p) = setup();
    let b = bare_linked(&home, &core, ".claude-b");

    core.repair(&b, None).unwrap();

    for item in ["agents", "settings.json"] {
        let target = fs::read_link(b.join(item)).unwrap();
        assert!(target.is_absolute());
        assert_eq!(target, p.join(item));
    }
}

/// Removes a link (file or directory symlink, or junction) without touching its target.
fn remove_link(path: &Path) {
    if fs::remove_file(path).is_err() {
        fs::remove_dir(path).unwrap();
    }
}
