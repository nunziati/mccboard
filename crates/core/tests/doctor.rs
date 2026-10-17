//! The doctor: link drift of linked instances against the sharing table (ADR 0005), and
//! unknown items asked about once (spec stories 19–20, 30–35, 43, 68; ticket #6).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{FIXTURE_TOKEN, TempHome, UUID_A, UUID_B};
use mccboard_core::{
    DriftItem, Error, InstanceDrift, InstanceKind, Problem, Repair, SharingAnswer,
};

/// Creates a link at `at` pointing to `target` (absolute or relative, as given): a symlink, or
/// on Windows a directory or file symlink depending on what `target` is.
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

fn mkdir(path: impl AsRef<Path>) {
    fs::create_dir_all(path).unwrap();
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
        write(p.join(dir).join("README.md"), "x");
    }
    write(p.join("skills/review/SKILL.md"), "---\nname: review\n---\n");
    write(p.join("skills/deploy/SKILL.md"), "---\nname: deploy\n---\n");
    write(p.join("skills/synced/org_a/notes/SKILL.md"), "synced");
    write(p.join("skills/.trash/old/SKILL.md"), "trashed");
    write(p.join("skills/README.txt"), "not a skill");
    write(
        p.join("plugins/installed_plugins.json"),
        r#"{"version":2,"plugins":{}}"#,
    );
    write(p.join("projects/-home-me/session.jsonl"), "{}");
    write(p.join("history.jsonl"), "{}");
    p
}

/// A linked instance set up the safe way: every linked item a single-hop link to the
/// primary's, one link per personal skill, its own `skills/synced/` and `plugins/`.
fn safe_linked(home: &TempHome, name: &str, p: &Path) -> PathBuf {
    let linked = home.linked(name);
    linked
        .logged_in_as(UUID_B, "b@example.com")
        .credentials_expiring(common::now() + jiff::SignedDuration::from_hours(5));
    let b = linked.dir.clone();
    for item in [
        "settings.json",
        "CLAUDE.md",
        "agents",
        "hooks",
        "output-styles",
        "scripts",
    ] {
        link(p.join(item), b.join(item));
    }
    link(p.join("skills/review"), b.join("skills/review"));
    link(p.join("skills/deploy"), b.join("skills/deploy"));
    write(b.join("skills/synced/org_b/mine/SKILL.md"), "b's synced");
    write(
        b.join("plugins/installed_plugins.json"),
        r#"{"version":2,"plugins":{}}"#,
    );
    write(b.join("projects/-home-me/other.jsonl"), "{}");
    write(b.join("history.jsonl"), "{}");
    b
}

fn drift_of(core: &mccboard_core::Core, dir: &Path) -> InstanceDrift {
    core.drift(dir).unwrap()
}

fn ids(drift: &InstanceDrift) -> Vec<&str> {
    drift.items.iter().map(|i| i.id.as_str()).collect()
}

fn item<'a>(drift: &'a InstanceDrift, id: &str) -> &'a DriftItem {
    drift
        .items
        .iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| panic!("no drift item {id} in {:?}", ids(drift)))
}

#[test]
fn a_safely_linked_instance_has_no_drift() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = safe_linked(&home, ".claude-b", &p);
    let (core, _env) = home.core();
    core.register(&p).unwrap();
    core.register(&b).unwrap();

    let doctor = core.doctor().unwrap();

    assert_eq!(doctor.instances.len(), 2);
    for instance in &doctor.instances {
        assert_eq!(instance.items, vec![], "{}", instance.config_dir.display());
        assert_eq!(instance.drift_count, 0);
    }
    assert_eq!(doctor.drift_count, 0);
    assert!(!doctor.has_drift());
    assert_eq!(doctor.questions, vec![]);
}

#[test]
fn the_primary_instance_never_drifts() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let (core, _env) = home.core();

    let drift = drift_of(&core, &p);

    assert_eq!(drift.kind, InstanceKind::Primary);
    assert_eq!(drift.items, vec![]);
}

#[test]
fn missing_links_and_unlinked_skills_are_drift_with_a_link_to_create() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = home.linked(".claude-b").dir.clone();
    let (core, _env) = home.core();

    let drift = drift_of(&core, &b);

    assert_eq!(
        ids(&drift),
        vec![
            "link:settings.json",
            "link:CLAUDE.md",
            "link:agents",
            "link:hooks",
            "link:output-styles",
            "link:scripts",
            "link:skills/deploy",
            "link:skills/review",
            // b has no .claude.json yet: the primary's Global config key is to align too.
            "config:autoConnectIde",
        ]
    );
    let settings = item(&drift, "link:settings.json");
    assert_eq!(settings.problem, Problem::MissingLink);
    assert_eq!(settings.item, "settings.json");
    assert_eq!(settings.path, b.join("settings.json"));
    assert_eq!(
        settings.repair,
        Repair::CreateLink {
            target: p.join("settings.json")
        }
    );
    assert!(settings.explanation.contains("settings.json"));
    assert!(settings.repair_explanation.contains("link"));

    let skill = item(&drift, "link:skills/review");
    assert_eq!(
        skill.problem,
        Problem::UnlinkedSkill {
            skill: "review".into()
        }
    );
    assert_eq!(skill.path, b.join("skills").join("review"));
    assert_eq!(
        skill.repair,
        Repair::CreateLink {
            target: p.join("skills").join("review")
        }
    );
    assert!(skill.explanation.contains("review"));
    assert_eq!(drift.drift_count, 9);
}

#[test]
fn a_new_personal_skill_in_the_primary_is_drift_until_linked() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = safe_linked(&home, ".claude-b", &p);
    let (core, _env) = home.core();
    assert_eq!(drift_of(&core, &b).items, vec![]);

    write(p.join("skills/fresh/SKILL.md"), "---\nname: fresh\n---\n");

    assert_eq!(ids(&drift_of(&core, &b)), vec!["link:skills/fresh"]);
    link(p.join("skills/fresh"), b.join("skills/fresh"));
    assert_eq!(drift_of(&core, &b).items, vec![]);
}

#[test]
fn a_broken_link_is_drift() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = safe_linked(&home, ".claude-b", &p);
    remove_link(&b.join("agents"));
    link(home.path().join("nowhere"), b.join("agents"));
    let (core, _env) = home.core();

    let drift = drift_of(&core, &b);

    assert_eq!(ids(&drift), vec!["link:agents"]);
    let agents = item(&drift, "link:agents");
    assert_eq!(
        agents.problem,
        Problem::BrokenLink {
            target: Some(home.path().join("nowhere"))
        }
    );
    assert_eq!(
        agents.repair,
        Repair::ReplaceLink {
            target: p.join("agents")
        }
    );
    assert!(agents.explanation.contains("nowhere"));
}

#[test]
fn items_the_primary_lacks_are_not_expected() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = safe_linked(&home, ".claude-b", &p);
    fs::remove_file(p.join("CLAUDE.md")).unwrap();
    fs::remove_dir_all(p.join("scripts")).unwrap();
    remove_link(&b.join("scripts"));
    let (core, _env) = home.core();

    // b's CLAUDE.md link is broken now, but there is nothing in the primary to link to.
    assert_eq!(drift_of(&core, &b).items, vec![]);
}

#[test]
fn a_link_pointing_elsewhere_is_drift() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = safe_linked(&home, ".claude-b", &p);
    write(home.path().join("dotfiles/CLAUDE.md"), "# Elsewhere\n");
    remove_link(&b.join("CLAUDE.md"));
    link(home.path().join("dotfiles/CLAUDE.md"), b.join("CLAUDE.md"));
    let (core, _env) = home.core();

    let drift = drift_of(&core, &b);

    assert_eq!(ids(&drift), vec!["link:CLAUDE.md"]);
    let claude_md = item(&drift, "link:CLAUDE.md");
    assert_eq!(
        claude_md.problem,
        Problem::WrongTarget {
            target: home.path().join("dotfiles/CLAUDE.md")
        }
    );
    assert_eq!(
        claude_md.repair,
        Repair::ReplaceLink {
            target: p.join("CLAUDE.md")
        }
    );
}

#[test]
fn a_link_chain_is_drift_even_when_it_ends_at_the_primary() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = safe_linked(&home, ".claude-b", &p);
    // c links settings.json to b's link, which links to the primary's: two hops.
    let c = safe_linked(&home, ".claude-c", &p);
    remove_link(&c.join("settings.json"));
    link(b.join("settings.json"), c.join("settings.json"));
    let (core, _env) = home.core();

    let drift = drift_of(&core, &c);

    assert_eq!(ids(&drift), vec!["link:settings.json"]);
    let settings = item(&drift, "link:settings.json");
    assert_eq!(
        settings.problem,
        Problem::LinkChain {
            target: b.join("settings.json")
        }
    );
    assert_eq!(
        settings.repair,
        Repair::ReplaceLink {
            target: p.join("settings.json")
        }
    );
}

#[test]
fn a_relative_link_to_the_primary_is_fine() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = safe_linked(&home, ".claude-b", &p);
    remove_link(&b.join("settings.json"));
    link(
        Path::new("..").join(".claude").join("settings.json"),
        b.join("settings.json"),
    );
    remove_link(&b.join("skills/review"));
    link(
        Path::new("..")
            .join("..")
            .join(".claude")
            .join("skills")
            .join("review"),
        b.join("skills/review"),
    );
    let (core, _env) = home.core();

    assert_eq!(drift_of(&core, &b).items, vec![]);
}

#[test]
fn a_separate_copy_where_a_link_belongs_is_drift_repaired_with_a_backup() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = safe_linked(&home, ".claude-b", &p);
    remove_link(&b.join("settings.json"));
    write(b.join("settings.json"), r#"{"model": "sonnet"}"#);
    remove_link(&b.join("skills/deploy"));
    write(b.join("skills/deploy/SKILL.md"), "an old copy");
    let (core, _env) = home.core();

    let drift = drift_of(&core, &b);

    assert_eq!(
        ids(&drift),
        vec!["link:settings.json", "link:skills/deploy"]
    );
    for (id, target) in [
        ("link:settings.json", p.join("settings.json")),
        ("link:skills/deploy", p.join("skills").join("deploy")),
    ] {
        let found = item(&drift, id);
        assert_eq!(found.problem, Problem::NotALink);
        assert_eq!(found.repair, Repair::BackupAndLink { target });
        assert!(found.repair_explanation.contains("backup"));
    }
}

#[test]
fn the_hand_made_mini_pc_layout_needs_skills_and_plugins_migrated() {
    // The layout built by hand on the Linux mini PC: every linked item linked, but `skills/`
    // and `plugins/` linked as whole folders.
    let home = TempHome::new();
    let p = full_primary(&home);
    let linked = home.linked(".claude-b");
    linked.logged_in_as(UUID_B, "b@example.com");
    let b = linked.dir.clone();
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
    write(b.join("history.jsonl"), "{}");
    let (core, _env) = home.core();
    core.register(&p).unwrap();
    core.register(&b).unwrap();

    let doctor = core.doctor().unwrap();
    let drift = doctor.instances.iter().find(|i| i.config_dir == b).unwrap();

    assert_eq!(ids(drift), vec!["migrate:skills", "migrate:plugins"]);
    let skills = item(drift, "migrate:skills");
    assert_eq!(
        skills.problem,
        Problem::WholeFolderLink {
            target: Some(p.join("skills"))
        }
    );
    assert_eq!(
        skills.repair,
        Repair::MigrateSkills {
            skills: vec!["deploy".into(), "review".into()]
        }
    );
    assert!(skills.explanation.contains("synced"));
    let plugins = item(drift, "migrate:plugins");
    assert_eq!(
        plugins.problem,
        Problem::WholeFolderLink {
            target: Some(p.join("plugins"))
        }
    );
    assert_eq!(plugins.repair, Repair::MigratePlugins);
    assert_eq!(doctor.drift_count, 2);
    assert!(doctor.has_drift());
}

#[test]
fn private_items_are_never_drift() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = safe_linked(&home, ".claude-b", &p);
    // b's private items differ from the primary's in every way.
    write(b.join("todos/x.json"), "[]");
    write(b.join("sessions/123.json"), "{}");
    write(b.join("plugins/known_marketplaces.json"), "{}");
    write(b.join(".credentials.json"), "{}");
    write(b.join("skills/synced/.bucket-org_b"), "");
    write(b.join("skills/only-in-b/SKILL.md"), "b's own");
    let (core, _env) = home.core();

    assert_eq!(drift_of(&core, &b).items, vec![]);
}

#[test]
fn hooks_in_the_primary_settings_are_not_drift() {
    // Claude Code doesn't run them twice in a linked instance (field test, 2026-10).
    let home = TempHome::new();
    let p = full_primary(&home);
    write(
        p.join("settings.json"),
        r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "notify"}]}]}}"#,
    );
    let b = safe_linked(&home, ".claude-b", &p);
    let (core, _env) = home.core();
    core.register(&p).unwrap();
    core.register(&b).unwrap();

    let doctor = core.doctor().unwrap();

    assert!(doctor.instances.iter().all(|i| i.items.is_empty()));
    assert!(!doctor.has_drift());
}

#[test]
fn ids_stay_the_same_whatever_is_wrong_with_the_item() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = home.linked(".claude-b").dir.clone();
    let (core, _env) = home.core();
    let missing = drift_of(&core, &b);

    link(home.path().join("nowhere"), b.join("hooks"));
    let broken = drift_of(&core, &b);

    assert_eq!(item(&missing, "link:hooks").problem, Problem::MissingLink);
    assert!(matches!(
        item(&broken, "link:hooks").problem,
        Problem::BrokenLink { .. }
    ));
    let _ = p;
}

#[test]
fn unknown_items_in_the_primary_are_asked_about() {
    let home = TempHome::new();
    let p = full_primary(&home);
    mkdir(p.join("gsd-core"));
    write(p.join("commands/ship.md"), "ship it");
    write(p.join("statusline.sh"), "echo hi");
    // Claude Code's own files and folders are private without asking.
    for name in [
        "todos",
        "sessions",
        "shell-snapshots",
        "file-history",
        "cache",
        "state",
        "backups",
        "debug",
        "telemetry",
        "plans",
    ] {
        mkdir(p.join(name));
    }
    for name in [
        ".credentials.json",
        "settings.local.json",
        ".last-cleanup",
        "mcp-needs-auth-cache.json",
        "remote-settings.json",
        "policy-limits.json",
    ] {
        write(p.join(name), "{}");
    }
    let (core, _env) = home.core();

    let questions = core.sharing_questions().unwrap();

    let asked: Vec<_> = questions.iter().map(|q| q.item.as_str()).collect();
    assert_eq!(asked, vec!["commands", "gsd-core", "statusline.sh"]);
    assert!(questions[0].is_dir);
    assert!(!questions[2].is_dir);
    assert_eq!(questions[1].path, p.join("gsd-core"));
    assert_eq!(core.doctor().unwrap().questions, questions);
    // Asking writes nothing.
    assert_eq!(home.config_text(), None);
}

#[test]
fn claude_codes_runtime_items_are_never_asked_about() {
    let home = TempHome::new();
    let p = full_primary(&home);
    // Folders and files Claude Code keeps per config directory (#17), and the daemon's
    // auth state.
    for name in [
        "agent-memory-local",
        "agent-memory-project",
        "api-dumps",
        "bridge-spawn",
        "downloads",
        "file-transfers",
        "local-settings",
        "mcp-skill-archives",
        "project-settings",
        "remote",
        "scratch",
        "storage-v2",
        "systemd",
    ] {
        mkdir(p.join(name));
    }
    for name in [
        "active-time.json",
        "antproto.json",
        "computer-use.lock",
        "server-sessions.json",
        "server.lock",
        "daemon-auth-cooldown",
        "daemon-auth-status.json",
        "daemon-auth-foo",
    ] {
        write(p.join(name), "{}");
    }
    // The prompt `/loop` reads: written by the user, so still asked about. So are the
    // user's own files that merely start like the daemon's.
    write(p.join("loop.md"), "Check the deploy.\n");
    write(p.join("daemon-start.sh"), "echo start\n");
    mkdir(p.join("daemon-tools"));
    write(p.join("daemonic"), "x");
    let b = safe_linked(&home, ".claude-b", &p);
    let (core, _env) = home.core();
    core.register(&b).unwrap();

    let asked: Vec<_> = core
        .sharing_questions()
        .unwrap()
        .into_iter()
        .map(|q| q.item)
        .collect();

    assert_eq!(
        asked,
        vec!["daemon-start.sh", "daemon-tools", "daemonic", "loop.md"]
    );
    assert_eq!(drift_of(&core, &b).items, vec![]);
}

/// Answers field configs hold from before #17, when Claude Code's runtime items were still
/// asked about: a Windows laptop's and a Linux mini PC's.
const FIELD_ANSWERS: &str = "\
[sharing.items]
daemon-auth-cooldown = \"private\"
\"daemon-auth-status.json\" = \"private\"
downloads = \"private\"
remote = \"private\"
\"statusline-command.ps1\" = \"link\"
\"statusline-command.sh\" = \"link\"
";

#[test]
fn stored_answers_for_items_now_known_private_are_ignored() {
    let home = TempHome::new();
    let p = full_primary(&home);
    for name in [
        "daemon-auth-cooldown",
        "daemon-auth-status.json",
        "statusline-command.ps1",
        "statusline-command.sh",
    ] {
        write(p.join(name), "x");
    }
    for name in ["downloads", "remote", "scratch"] {
        mkdir(p.join(name));
    }
    let b = safe_linked(&home, ".claude-b", &p);
    link(
        p.join("statusline-command.ps1"),
        b.join("statusline-command.ps1"),
    );
    link(
        p.join("statusline-command.sh"),
        b.join("statusline-command.sh"),
    );
    mkdir(b.join("downloads"));
    // A stored "link" for a name now known private is ignored too: the table wins.
    let text = format!("{FIELD_ANSWERS}scratch = \"link\"\n");
    home.write_config(&text);
    let (core, _env) = home.core();
    core.register(&b).unwrap();

    let doctor = core.doctor().unwrap();

    assert_eq!(doctor.questions, vec![]);
    assert_eq!(doctor.instances.len(), 1);
    assert_eq!(doctor.instances[0].items, vec![]);
    // The answers still in force keep working: an unlinked statusline script is drift.
    fs::remove_file(b.join("statusline-command.sh")).unwrap();
    assert_eq!(
        ids(&drift_of(&core, &b)),
        vec!["link:statusline-command.sh"]
    );
    // Names now in the table take no new answer, and the stored lines stay as written.
    assert!(
        core.answer_sharing("downloads", SharingAnswer::Link)
            .is_err()
    );
    core.answer_sharing("gsd-core", SharingAnswer::Private)
        .unwrap();
    let saved = home.config_text().unwrap();
    assert!(saved.starts_with(&text), "{saved}");
}

#[test]
fn a_stored_answer_for_a_linked_item_is_still_reported() {
    let home = TempHome::new();
    full_primary(&home);
    home.write_config("[sharing.items]\n\"settings.json\" = \"private\"\n");
    let (core, _env) = home.core();

    let error = core.doctor().unwrap_err();

    assert!(matches!(error, Error::ConfigCorrupt { .. }), "{error}");
    assert!(error.detail().unwrap().contains("settings.json"));
}

#[test]
fn a_stored_answer_for_an_item_now_known_private_must_still_be_well_formed() {
    // A bad answer, and keys that match a private pattern but are no plain name.
    for (text, culprit) in [
        ("[sharing.items]\ndownloads = \"maybe\"\n", "downloads"),
        ("[sharing.items]\n\"x/y.lock\" = \"private\"\n", "x/y.lock"),
        (
            "[sharing.items]\n\"daemon-auth-/../a\" = \"private\"\n",
            "daemon-auth-/../a",
        ),
    ] {
        let home = TempHome::new();
        full_primary(&home);
        home.write_config(text);
        let (core, _env) = home.core();

        let error = core.doctor().unwrap_err();

        assert!(
            matches!(error, Error::ConfigCorrupt { .. }),
            "{text}: {error}"
        );
        assert!(error.detail().unwrap().contains(culprit), "{text}: {error}");
        assert_eq!(home.config_text().unwrap(), text);
    }
}

#[test]
fn an_answer_is_remembered_for_every_instance_and_across_restarts() {
    let home = TempHome::new();
    let p = full_primary(&home);
    write(p.join("gsd-core/VERSION"), "1");
    write(p.join("keybindings.json"), "[]");
    let b = safe_linked(&home, ".claude-b", &p);
    let c = safe_linked(&home, ".claude-c", &p);
    let (core, _env) = home.core();
    core.register(&b).unwrap();
    core.register(&c).unwrap();

    core.answer_sharing("gsd-core", SharingAnswer::Link)
        .unwrap();
    core.answer_sharing("keybindings.json", SharingAnswer::Private)
        .unwrap();

    let (restarted, _env) = home.core();
    assert_eq!(restarted.sharing_questions().unwrap(), vec![]);
    let doctor = restarted.doctor().unwrap();
    for instance in &doctor.instances {
        assert_eq!(ids(instance), vec!["link:gsd-core"]);
        let gsd = item(instance, "link:gsd-core");
        assert_eq!(gsd.problem, Problem::MissingLink);
        assert_eq!(
            gsd.repair,
            Repair::CreateLink {
                target: p.join("gsd-core")
            }
        );
    }

    link(p.join("gsd-core"), b.join("gsd-core"));
    assert_eq!(restarted.drift(&b).unwrap().items, vec![]);

    let config: toml::Table = home.config_text().unwrap().parse().unwrap();
    assert_eq!(
        config["sharing"]["items"]["gsd-core"].as_str(),
        Some("link")
    );
    assert_eq!(
        config["sharing"]["items"]["keybindings.json"].as_str(),
        Some("private")
    );
}

#[test]
fn an_answer_can_be_changed_and_keeps_the_rest_of_the_file() {
    let home = TempHome::new();
    let p = full_primary(&home);
    mkdir(p.join("gsd-core"));
    home.write_config("# my settings\n[sharing.items]\ngsd-core = \"link\" # for GSD\n");
    let (core, _env) = home.core();

    core.answer_sharing("gsd-core", SharingAnswer::Private)
        .unwrap();

    let text = home.config_text().unwrap();
    assert!(text.starts_with("# my settings\n"), "{text}");
    assert!(text.contains("gsd-core = \"private\" # for GSD"), "{text}");
}

#[test]
fn items_in_the_sharing_table_take_no_answer() {
    let home = TempHome::new();
    full_primary(&home);
    let (core, _env) = home.core();

    for item in [
        "settings.json",
        "skills",
        "plugins",
        "projects",
        "../x",
        "a/b",
        "",
    ] {
        let error = core.answer_sharing(item, SharingAnswer::Link).unwrap_err();
        assert!(
            matches!(error, Error::InvalidSharingItem { .. }),
            "{item}: {error}"
        );
        assert_eq!(error.code(), "invalid_sharing_item");
    }
    assert_eq!(home.config_text(), None);
}

#[test]
fn a_bad_answer_written_by_hand_is_reported_and_never_overwritten() {
    let home = TempHome::new();
    full_primary(&home);
    let text = "[sharing.items]\ngsd-core = \"maybe\"\n";
    home.write_config(text);
    let (core, _env) = home.core();

    let error = core.doctor().unwrap_err();

    assert!(matches!(error, Error::ConfigCorrupt { .. }), "{error}");
    assert!(error.detail().unwrap().contains("gsd-core"));
    assert!(core.answer_sharing("x", SharingAnswer::Link).is_err());
    assert_eq!(home.config_text().unwrap(), text);
}

#[test]
fn the_doctor_covers_registered_instances_only() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = home.linked(".claude-b").dir.clone();
    home.linked(".claude-c");
    let (core, _env) = home.core();
    core.register(&b).unwrap();

    let doctor = core.doctor().unwrap();

    let dirs: Vec<_> = doctor.instances.iter().map(|i| &i.config_dir).collect();
    assert_eq!(dirs, vec![&b]);
    assert_eq!(doctor.instances[0].kind, InstanceKind::Linked);
    // Eight links, plus config:autoConnectIde (b has no .claude.json).
    assert_eq!(doctor.drift_count, 9);
    let _ = p;
}

#[test]
fn a_registered_instance_whose_folder_is_gone_reports_no_link_drift() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let b = home.linked(".claude-b").dir.clone();
    let (core, _env) = home.core();
    core.register(&b).unwrap();
    fs::remove_dir_all(&b).unwrap();

    let doctor = core.doctor().unwrap();

    assert_eq!(doctor.instances[0].items, vec![]);
    let _ = p;
}

#[test]
fn without_a_primary_instance_nothing_is_expected() {
    let home = TempHome::new();
    let b = home.linked(".claude-b").dir.clone();
    let (core, _env) = home.core();

    assert_eq!(drift_of(&core, &b).items, vec![]);
    assert_eq!(core.sharing_questions().unwrap(), vec![]);
}

#[test]
fn when_the_primary_item_is_itself_a_link_the_real_item_is_its_target() {
    let home = TempHome::new();
    let p = full_primary(&home);
    let dotfiles = home.path().join("dotfiles").join("settings.json");
    fs::create_dir_all(dotfiles.parent().unwrap()).unwrap();
    fs::rename(p.join("settings.json"), &dotfiles).unwrap();
    link(&dotfiles, p.join("settings.json"));
    let b = safe_linked(&home, ".claude-b", &p);
    let (core, _env) = home.core();

    // b links to the primary's link: two hops to the real file.
    let drift = drift_of(&core, &b);
    let settings = item(&drift, "link:settings.json");
    assert_eq!(
        settings.problem,
        Problem::LinkChain {
            target: p.join("settings.json")
        }
    );
    assert_eq!(
        settings.repair,
        Repair::ReplaceLink {
            target: dotfiles.clone()
        }
    );

    remove_link(&b.join("settings.json"));
    link(&dotfiles, b.join("settings.json"));
    assert_eq!(drift_of(&core, &b).items, vec![]);
}

#[test]
fn no_token_in_the_doctor_report() {
    let home = TempHome::new();
    let p = full_primary(&home);
    home.primary()
        .credentials_expiring(common::now() + jiff::SignedDuration::from_hours(5));
    let b = safe_linked(&home, ".claude-b", &p);
    remove_link(&b.join("settings.json"));
    let (core, _env) = home.core();
    core.register(&p).unwrap();
    core.register(&b).unwrap();

    let json = serde_json::to_string(&core.doctor().unwrap()).unwrap();

    assert!(!json.contains(FIXTURE_TOKEN));
    assert!(!json.contains("FIXTURE"));
}

#[cfg(windows)]
#[test]
fn junctions_are_links_too() {
    use std::process::Command;

    let home = TempHome::new();
    let p = full_primary(&home);
    let b = safe_linked(&home, ".claude-b", &p);
    for (at, target) in [
        (b.join("agents"), p.join("agents")),
        (b.join("skills").join("review"), home.path().join("nowhere")),
    ] {
        remove_link(&at);
        if at.ends_with("review") {
            mkdir(home.path().join("nowhere"));
        }
        let status = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&at)
            .arg(&target)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let (core, _env) = home.core();

    let drift = drift_of(&core, &b);

    // The junction to the primary's agents is a fine link; the one elsewhere is drift.
    assert_eq!(ids(&drift), vec!["link:skills/review"]);
    assert!(matches!(
        item(&drift, "link:skills/review").problem,
        Problem::WrongTarget { .. }
    ));
}

// ---------------------------------------------------------------------------------------------
// A private item that is still a link to the primary's (#28)

#[test]
fn a_private_item_still_linked_to_the_primary_after_an_update_is_drift() {
    // Before #17, `downloads` was asked about and answered "link", and repair linked it.
    // Now the table makes it private: the answer is ignored, the link still shares it.
    let home = TempHome::new();
    let p = full_primary(&home);
    write(p.join("downloads/file.zip"), "the primary's download");
    let b = safe_linked(&home, ".claude-b", &p);
    link(p.join("downloads"), b.join("downloads"));
    home.write_config("[sharing.items]\ndownloads = \"link\"\n");
    let (core, _env) = home.core();
    core.register(&b).unwrap();

    let doctor = core.doctor().unwrap();

    let drift = &doctor.instances[0];
    assert_eq!(ids(drift), vec!["private:downloads"]);
    let downloads = item(drift, "private:downloads");
    assert_eq!(downloads.item, "downloads");
    assert_eq!(downloads.path, b.join("downloads"));
    assert_eq!(
        downloads.problem,
        Problem::LinkedPrivateItem {
            target: p.join("downloads"),
            ignored_answer: true,
        }
    );
    assert_eq!(
        downloads.repair,
        Repair::Unlink {
            target: p.join("downloads")
        }
    );
    assert!(
        downloads.explanation.contains("belongs to each instance"),
        "{}",
        downloads.explanation
    );
    assert!(
        downloads.explanation.contains("config.toml") && downloads.explanation.contains("ignored"),
        "{}",
        downloads.explanation
    );
    assert!(
        downloads.repair_explanation.contains("Remove the link"),
        "{}",
        downloads.repair_explanation
    );
    assert_eq!(doctor.drift_count, 1);
    assert_eq!(doctor.questions, vec![]);
}

#[test]
fn private_items_by_name_prefix_or_suffix_linked_to_the_primary_are_drift() {
    let home = TempHome::new();
    let p = full_primary(&home);
    mkdir(p.join("scratch"));
    write(p.join("daemon-auth-cooldown"), "{}");
    write(p.join("computer-use.lock"), "");
    let b = safe_linked(&home, ".claude-b", &p);
    for name in ["scratch", "daemon-auth-cooldown", "computer-use.lock"] {
        link(p.join(name), b.join(name));
    }
    let (core, _env) = home.core();

    let drift = drift_of(&core, &b);

    assert_eq!(
        ids(&drift),
        vec![
            "private:computer-use.lock",
            "private:daemon-auth-cooldown",
            "private:scratch",
        ]
    );
    for found in &drift.items {
        assert_eq!(
            found.problem,
            Problem::LinkedPrivateItem {
                target: p.join(&found.item),
                ignored_answer: false,
            }
        );
        assert!(
            !found.explanation.contains("config.toml"),
            "{}",
            found.explanation
        );
        // Only the credentials file warns of a login to redo.
        assert!(!found.explanation.contains("logged out"));
        assert!(!found.repair_explanation.contains("logged out"));
    }
}

#[test]
fn credentials_linked_to_the_primary_are_drift_that_warns_of_the_login_to_redo() {
    let home = TempHome::new();
    let p = full_primary(&home);
    home.primary()
        .credentials_expiring(common::now() + jiff::SignedDuration::from_hours(5));
    let b = safe_linked(&home, ".claude-b", &p);
    fs::remove_file(b.join(".credentials.json")).unwrap();
    link(p.join(".credentials.json"), b.join(".credentials.json"));
    let (core, _env) = home.core();

    let drift = drift_of(&core, &b);

    assert_eq!(ids(&drift), vec!["private:.credentials.json"]);
    let credentials = item(&drift, "private:.credentials.json");
    assert_eq!(
        credentials.repair,
        Repair::Unlink {
            target: p.join(".credentials.json")
        }
    );
    for text in [&credentials.explanation, &credentials.repair_explanation] {
        assert!(text.contains("logged out"), "{text}");
        assert!(text.contains("\"Log in\""), "{text}");
        assert!(text.contains("mccboard login"), "{text}");
        assert!(!text.contains("logout"), "{text}");
    }
    let json = serde_json::to_string(&drift).unwrap();
    assert!(!json.contains("FIXTURE"));
}

#[test]
fn an_item_answered_private_that_is_still_a_link_to_the_primary_is_drift() {
    // The user changed the answer from "link" to "private" after repair had linked it.
    let home = TempHome::new();
    let p = full_primary(&home);
    write(p.join("gsd-core/VERSION"), "1");
    let b = safe_linked(&home, ".claude-b", &p);
    link(p.join("gsd-core"), b.join("gsd-core"));
    home.write_config("[sharing.items]\ngsd-core = \"private\"\n");
    let (core, _env) = home.core();

    let drift = drift_of(&core, &b);

    assert_eq!(ids(&drift), vec!["private:gsd-core"]);
    assert_eq!(
        item(&drift, "private:gsd-core").problem,
        Problem::LinkedPrivateItem {
            target: p.join("gsd-core"),
            ignored_answer: false,
        }
    );
}

#[test]
fn a_private_item_linked_through_a_chain_to_the_primary_is_drift() {
    let home = TempHome::new();
    let p = full_primary(&home);
    mkdir(p.join("downloads"));
    let b = safe_linked(&home, ".claude-b", &p);
    let c = safe_linked(&home, ".claude-c", &p);
    link(p.join("downloads"), b.join("downloads"));
    link(b.join("downloads"), c.join("downloads"));
    let (core, _env) = home.core();

    let drift = drift_of(&core, &c);

    assert_eq!(ids(&drift), vec!["private:downloads"]);
    assert_eq!(
        item(&drift, "private:downloads").problem,
        Problem::LinkedPrivateItem {
            target: b.join("downloads"),
            ignored_answer: false,
        }
    );
}

#[test]
fn a_private_item_linked_anywhere_but_the_primary_is_left_alone() {
    // The user's own setup: not reported.
    let home = TempHome::new();
    let p = full_primary(&home);
    mkdir(p.join("downloads"));
    mkdir(home.path().join("Downloads"));
    write(home.path().join("dotfiles/gsd-core/VERSION"), "1");
    mkdir(p.join("gsd-core"));
    let b = safe_linked(&home, ".claude-b", &p);
    link(home.path().join("Downloads"), b.join("downloads"));
    link(home.path().join("dotfiles/gsd-core"), b.join("gsd-core"));
    // A private item linked to where nothing is, and a link to the primary's private item
    // under another name.
    link(home.path().join("nowhere"), b.join("scratch"));
    link(p.join("downloads"), b.join("daemon-auth-elsewhere"));
    home.write_config("[sharing.items]\ngsd-core = \"private\"\n");
    let (core, _env) = home.core();

    assert_eq!(drift_of(&core, &b).items, vec![]);
}

#[test]
fn an_unanswered_item_linked_to_the_primary_is_not_drift() {
    let home = TempHome::new();
    let p = full_primary(&home);
    mkdir(p.join("gsd-core"));
    let b = safe_linked(&home, ".claude-b", &p);
    link(p.join("gsd-core"), b.join("gsd-core"));
    let (core, _env) = home.core();

    assert_eq!(drift_of(&core, &b).items, vec![]);
    assert_eq!(core.sharing_questions().unwrap()[0].item, "gsd-core");
}

#[cfg(windows)]
#[test]
fn a_junction_to_the_primary_private_folder_is_drift() {
    use std::process::Command;

    let home = TempHome::new();
    let p = full_primary(&home);
    mkdir(p.join("downloads"));
    let b = safe_linked(&home, ".claude-b", &p);
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(b.join("downloads"))
        .arg(p.join("downloads"))
        .status()
        .unwrap();
    assert!(status.success());
    let (core, _env) = home.core();

    let drift = drift_of(&core, &b);

    assert_eq!(ids(&drift), vec!["private:downloads"]);
    assert_eq!(
        item(&drift, "private:downloads").repair,
        Repair::Unlink {
            target: p.join("downloads")
        }
    );
}

/// Removes a link (file or directory symlink, or junction) without touching its target.
fn remove_link(path: &Path) {
    if fs::remove_file(path).is_err() {
        fs::remove_dir(path).unwrap();
    }
}
