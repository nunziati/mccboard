//! Removing an instance (spec stories 44–47 and 66; ticket #12): from mccboard only, or with
//! its folder behind an explicit choice and the session guard. The launcher always goes with
//! the instance; the primary instance is never removed. Runs on temporary homes with real
//! links (junctions on Windows).

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use common::{FIXTURE_TOKEN, TempHome, UUID_A, UUID_B};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    ClaudeProcess, Core, Error, FolderOutcome, ProcessConfigDir, Removal, RemoveLevel,
};

// ---------------------------------------------------------------------------------------------
// Fixtures

fn write(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Creates a link at `at` pointing to `target`, the way a user would by hand.
fn link(target: impl AsRef<Path>, at: impl AsRef<Path>) {
    let (target, at) = (target.as_ref(), at.as_ref());
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, at).unwrap();
    #[cfg(windows)]
    {
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, at).unwrap();
        } else {
            std::os::windows::fs::symlink_file(target, at).unwrap();
        }
    }
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Every path under `root` (links not followed) with what it is. Two equal snapshots mean
/// nothing changed.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                out.insert(relative, "link".into());
            } else if meta.is_dir() {
                out.insert(relative, "dir".into());
                stack.push(path);
            } else {
                out.insert(
                    relative,
                    format!("file {}", fs::read_to_string(&path).unwrap()),
                );
            }
        }
    }
    out
}

/// A registered primary instance holding every linked item and two personal skills.
fn primary(home: &TempHome, core: &Core) -> PathBuf {
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(common::now() + jiff::SignedDuration::from_hours(5));
    let p = primary.dir.clone();
    write(p.join("settings.json"), r#"{"model": "opus"}"#);
    write(p.join("CLAUDE.md"), "# Me\n");
    for dir in ["agents", "hooks", "output-styles", "scripts"] {
        write(p.join(dir).join("README.md"), dir);
    }
    write(p.join("skills/review/SKILL.md"), "---\nname: review\n---\n");
    write(p.join("skills/deploy/SKILL.md"), "---\nname: deploy\n---\n");
    write(p.join("projects/-home-me/session.jsonl"), "{}");
    core.register(&p).unwrap();
    p
}

/// A registered linked instance logged in as B, with every linked item linked to the primary
/// instance by a repair (junctions for folders on Windows), and private items of its own.
fn linked(home: &TempHome, core: &Core) -> PathBuf {
    let instance = home.linked(".claude-b");
    instance
        .logged_in_as(UUID_B, "b@example.com")
        .credentials_expiring(common::now() + jiff::SignedDuration::from_hours(5));
    let b = instance.dir.clone();
    write(b.join("history.jsonl"), "b's history");
    write(b.join("projects/-home-me/b.jsonl"), "b's session");
    write(b.join("skills/synced/org_b/notes/SKILL.md"), "b's synced");
    core.register(&b).unwrap();
    core.repair(&b, None).unwrap();
    b
}

fn setup() -> (TempHome, Core, FakeEnvironment, PathBuf, PathBuf) {
    let home = TempHome::new();
    let (core, env) = home.core();
    let p = primary(&home, &core);
    let b = linked(&home, &core);
    (home, core, env, p, b)
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

fn registered(core: &Core) -> Vec<PathBuf> {
    core.registry()
        .unwrap()
        .instances
        .into_iter()
        .map(|i| i.config_dir)
        .collect()
}

/// The launcher names that exist now, wherever this machine keeps them.
fn launcher_names(core: &Core) -> Vec<String> {
    let status = core.launchers().unwrap();
    status
        .instances
        .iter()
        .filter(|l| l.state != mccboard_core::LauncherState::Missing)
        .map(|l| l.name.clone())
        .chain(status.orphans.iter().map(|o| o.name.clone()))
        .collect()
}

/// Every launcher mccboard wrote, as text: the shell rc files' contents and the bin folder.
fn launcher_files(home: &TempHome) -> String {
    let mut text = String::new();
    for rc in [".bashrc", ".zshrc", ".profile"] {
        text.push_str(&fs::read_to_string(home.path().join(rc)).unwrap_or_default());
    }
    for folder in [".mccboard/bin", ".local/bin"] {
        if let Ok(entries) = fs::read_dir(home.path().join(folder)) {
            for entry in entries.flatten() {
                text.push_str(&entry.file_name().to_string_lossy());
                text.push('\n');
            }
        }
    }
    text
}

// ---------------------------------------------------------------------------------------------
// Removing from mccboard only

#[test]
fn removing_from_mccboard_forgets_the_instance_and_its_launcher_and_keeps_the_folder() {
    let (home, core, _env, p, b) = setup();
    assert_eq!(launcher_names(&core), vec!["claude-b"]);
    assert!(launcher_files(&home).contains("claude-b"));
    let before = snapshot(&b);

    let removal = core.remove(&b, RemoveLevel::FromMccboard).unwrap();

    assert_eq!(
        removal,
        Removal {
            config_dir: b.clone(),
            folder: FolderOutcome::Kept,
        }
    );
    assert_eq!(registered(&core), vec![p]);
    assert_eq!(launcher_names(&core), Vec::<String>::new());
    assert!(!launcher_files(&home).contains("claude-b"));
    assert_eq!(snapshot(&b), before);
}

#[test]
fn removing_from_mccboard_ignores_running_sessions_since_the_folder_is_untouched() {
    let (_home, core, env, _p, b) = setup();
    env.add_process(claude(4242, dir(&b)));

    let removal = core.remove(&b, RemoveLevel::FromMccboard).unwrap();

    assert_eq!(removal.folder, FolderOutcome::Kept);
    assert!(b.join(".credentials.json").is_file());
}

#[test]
fn the_account_keeps_its_label_and_color_after_its_instance_is_removed() {
    let (_home, core, _env, _p, b) = setup();
    let accounts = core.registry().unwrap().accounts;

    core.remove(&b, RemoveLevel::WithFolder).unwrap();

    assert_eq!(core.registry().unwrap().accounts, accounts);
}

// ---------------------------------------------------------------------------------------------
// Removing with the folder

#[test]
fn removing_with_the_folder_deletes_it_and_the_launcher() {
    let (home, core, _env, p, b) = setup();

    let removal = core.remove(&b, RemoveLevel::WithFolder).unwrap();

    assert_eq!(
        removal,
        Removal {
            config_dir: b.clone(),
            folder: FolderOutcome::Deleted,
        }
    );
    assert!(!exists(&b));
    assert_eq!(registered(&core), vec![p]);
    assert_eq!(launcher_names(&core), Vec::<String>::new());
    assert!(!launcher_files(&home).contains("claude-b"));
}

#[test]
fn deleting_a_linked_instance_removes_its_links_never_what_they_point_to() {
    let (home, core, _env, p, b) = setup();
    // A folder link and a file link made by hand, plus one inside a real subfolder.
    link(p.join("agents"), b.join("my-agents"));
    link(p.join("CLAUDE.md"), b.join("notes.md"));
    link(p.join("skills/review"), b.join("extra/review"));
    // A link to something outside both instances.
    let outside = home.path().join("work/keep.txt");
    write(&outside, "keep me");
    link(&outside, b.join("keep.txt"));
    let primary_before = snapshot(&p);
    assert!(
        fs::symlink_metadata(b.join("settings.json"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::symlink_metadata(b.join("agents"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::symlink_metadata(b.join("skills/review"))
            .unwrap()
            .file_type()
            .is_symlink()
    );

    core.remove(&b, RemoveLevel::WithFolder).unwrap();

    assert!(!exists(&b));
    assert_eq!(snapshot(&p), primary_before);
    assert_eq!(
        fs::read_to_string(p.join("settings.json")).unwrap(),
        r#"{"model": "opus"}"#
    );
    assert!(p.join("agents/README.md").is_file());
    assert!(p.join("skills/review/SKILL.md").is_file());
    assert!(p.join("skills/deploy/SKILL.md").is_file());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep me");
    // The primary instance is still healthy for the doctor.
    assert_eq!(core.drift(&p).unwrap().drift_count, 0);
}

#[test]
fn read_only_files_inside_the_folder_are_deleted_too() {
    // Plugin marketplaces are git clones, whose objects are read-only.
    let (_home, core, _env, _p, b) = setup();
    let object = b.join("plugins/marketplaces/m/.git/objects/ab/cdef");
    write(&object, "blob");
    let mut permissions = fs::metadata(&object).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&object, permissions).unwrap();

    core.remove(&b, RemoveLevel::WithFolder).unwrap();

    assert!(!exists(&b));
}

#[test]
fn a_folder_already_gone_is_just_removed_from_mccboard() {
    let (_home, core, _env, p, b) = setup();
    fs::remove_dir_all(&b).unwrap();

    let removal = core.remove(&b, RemoveLevel::WithFolder).unwrap();

    assert_eq!(removal.folder, FolderOutcome::AlreadyGone);
    assert_eq!(registered(&core), vec![p]);
    assert_eq!(launcher_names(&core), Vec::<String>::new());
}

#[test]
fn the_path_is_accepted_in_any_spelling() {
    let (_home, core, _env, _p, b) = setup();

    core.remove("~/x/../.claude-b/".as_ref(), RemoveLevel::WithFolder)
        .unwrap();

    assert!(!exists(&b));
}

// ---------------------------------------------------------------------------------------------
// The session guard

#[test]
fn deleting_the_folder_refuses_while_claude_code_uses_it_and_changes_nothing() {
    let (home, core, env, p, b) = setup();
    env.add_process(ClaudeProcess {
        cwd: Some(home.path().join("work")),
        ..claude(4242, dir(&b))
    });
    let before = snapshot(&b);
    let launchers = launcher_files(&home);

    let error = core.remove(&b, RemoveLevel::WithFolder).unwrap_err();

    let Error::InstanceInUse { path, processes } = &error else {
        panic!("{error}")
    };
    assert_eq!(path, &b);
    assert_eq!(
        processes.iter().map(|p| p.pid).collect::<Vec<_>>(),
        vec![4242]
    );
    assert_eq!(error.code(), "instance_in_use");
    assert_eq!(snapshot(&b), before);
    assert_eq!(registered(&core), vec![p, b.clone()]);
    assert_eq!(launcher_files(&home), launchers);

    env.set_processes(vec![]);
    assert_eq!(
        core.remove(&b, RemoveLevel::WithFolder).unwrap().folder,
        FolderOutcome::Deleted
    );
}

#[test]
fn a_session_found_only_in_claude_code_session_registry_also_blocks_deletion() {
    let (_home, core, env, _p, b) = setup();
    write(b.join("sessions/5150.json"), r#"{"pid":5150}"#);
    env.set_processes(vec![claude(5150, ProcessConfigDir::Unknown)]);

    let error = core.remove(&b, RemoveLevel::WithFolder).unwrap_err();

    assert!(matches!(error, Error::InstanceInUse { .. }), "{error}");
    assert!(b.join(".credentials.json").is_file());
}

#[test]
fn sessions_of_other_instances_do_not_block_deletion() {
    let (home, core, env, _p, b) = setup();
    let c = home.linked(".claude-c").dir.clone();
    env.set_processes(vec![
        claude(1, ProcessConfigDir::Default),
        claude(2, dir(&c)),
        claude(3, ProcessConfigDir::Unknown),
    ]);

    core.remove(&b, RemoveLevel::WithFolder).unwrap();

    assert!(!exists(&b));
}

// ---------------------------------------------------------------------------------------------
// The primary instance and other folders mccboard never deletes

#[test]
fn the_primary_instance_is_never_removed_at_either_level() {
    let (home, core, _env, p, b) = setup();
    let before = snapshot(&p);

    for level in [RemoveLevel::FromMccboard, RemoveLevel::WithFolder] {
        for spelling in [
            p.clone(),
            "~/.claude/".into(),
            home.path().join("x/../.claude"),
        ] {
            let error = core.remove(&spelling, level).unwrap_err();
            assert!(matches!(error, Error::PrimaryInstance { .. }), "{error}");
            assert_eq!(error.code(), "primary_instance");
            assert_eq!(error.path(), Some(p.as_path()));
        }
    }

    assert_eq!(snapshot(&p), before);
    assert_eq!(registered(&core), vec![p.clone(), b]);
}

#[test]
fn unregistering_the_primary_instance_is_refused_too() {
    let (_home, core, _env, p, b) = setup();

    let error = core.unregister(&p).unwrap_err();

    assert!(matches!(error, Error::PrimaryInstance { .. }), "{error}");
    assert_eq!(registered(&core), vec![p, b]);
}

#[test]
fn an_unregistered_instance_is_not_removed_and_its_folder_is_kept() {
    let home = TempHome::new();
    let (core, _env) = home.core();
    let c = home.linked(".claude-c").dir.clone();
    write(c.join("history.jsonl"), "c's history");

    let error = core.remove(&c, RemoveLevel::WithFolder).unwrap_err();

    assert!(matches!(error, Error::NotRegistered { .. }), "{error}");
    assert!(c.join("history.jsonl").is_file());
}

#[test]
fn a_folder_holding_the_home_is_never_deleted() {
    let home = TempHome::new();
    let (core, _env) = home.core();
    let p = primary(&home, &core);
    // A registered "instance" whose folder is the home itself, holding the primary instance.
    core.register(home.path()).unwrap();
    let before = snapshot(&p);

    let error = core
        .remove(home.path(), RemoveLevel::WithFolder)
        .unwrap_err();

    assert!(matches!(error, Error::FolderNotDeletable { .. }), "{error}");
    assert_eq!(error.code(), "folder_not_deletable");
    assert_eq!(snapshot(&p), before);
    assert_eq!(registered(&core).len(), 2);
    // Removing it from mccboard only is fine.
    core.remove(home.path(), RemoveLevel::FromMccboard).unwrap();
    assert_eq!(registered(&core), vec![p]);
}

#[test]
fn an_instance_folder_that_is_a_link_to_the_primary_instance_is_never_deleted() {
    let home = TempHome::new();
    let (core, _env) = home.core();
    let p = primary(&home, &core);
    let alias = home.path().join(".claude-alias");
    link(&p, &alias);
    core.register(&alias).unwrap();
    let before = snapshot(&p);

    let error = core.remove(&alias, RemoveLevel::WithFolder).unwrap_err();

    assert!(matches!(error, Error::FolderNotDeletable { .. }), "{error}");
    assert_eq!(snapshot(&p), before);
    assert!(exists(&alias));
}

#[test]
fn an_instance_folder_that_is_a_link_elsewhere_is_not_followed() {
    let home = TempHome::new();
    let (core, _env) = home.core();
    primary(&home, &core);
    let real = home.path().join("data/claude-b");
    write(real.join("history.jsonl"), "b's history");
    let alias = home.path().join(".claude-b");
    link(&real, &alias);
    core.register(&alias).unwrap();

    let error = core.remove(&alias, RemoveLevel::WithFolder).unwrap_err();

    assert!(matches!(error, Error::FolderNotDeletable { .. }), "{error}");
    assert!(real.join("history.jsonl").is_file());
    assert!(exists(&alias));
}

// ---------------------------------------------------------------------------------------------
// Token safety

#[test]
fn no_token_in_removals_or_errors() {
    let (home, core, env, p, b) = setup();
    env.add_process(claude(7, dir(&b)));

    let refused = core.remove(&b, RemoveLevel::WithFolder).unwrap_err();
    let primary = core.remove(&p, RemoveLevel::WithFolder).unwrap_err();
    env.set_processes(vec![]);
    let removal = core.remove(&b, RemoveLevel::WithFolder).unwrap();

    for text in [
        refused.to_string(),
        format!("{refused:?}"),
        primary.to_string(),
        serde_json::to_string(&removal).unwrap(),
    ] {
        assert!(!text.contains(FIXTURE_TOKEN), "{text}");
    }
    for (path, text) in home.mccboard_files() {
        assert!(!text.contains(FIXTURE_TOKEN), "{}", path.display());
    }
}
