//! Aligned items (spec stories 17–18, 36–37 and 41; ticket #10): the Global config keys of
//! `.claude.json` and the installed plugins and marketplaces. The doctor reports what differs
//! from the primary instance; repair writes only the listed keys and installs what is missing
//! with Claude Code's own `claude plugin` commands, run through the environment port.

mod common;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use common::{TempHome, UUID_A, UUID_B};
use mccboard_core::testing::FakeEnvironment;
use mccboard_core::{
    ClaudeInvocation, ClaudeProcess, ClaudeRun, Core, DriftItem, Error, InstanceDrift, Problem,
    ProcessConfigDir, Repair, RepairOutcome, RepairReport,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------------
// Fixtures

fn write(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap()
}

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_str(&read(path)).unwrap()
}

/// The primary's `~/.claude.json`: logged in as A, with some Global config keys set.
fn primary_global_config() -> Value {
    json!({
        "numStartups": 40,
        "theme": "dark",
        "oauthAccount": {
            "accountUuid": UUID_A,
            "emailAddress": "a@example.com",
        },
        "defaultToAgentsView": true,
        "diffTool": "auto",
        "copyOnSelect": false,
        "autoConnectIde": true,
        "externalEditorContext": {"enabled": true, "lines": 3},
        "projects": {"/home/me/a": {"allowedTools": []}},
    })
}

/// A linked instance's `.claude.json`: logged in as B, keys partly aligned.
fn linked_global_config() -> Value {
    json!({
        "numStartups": 2,
        "theme": "light",
        "oauthAccount": {
            "accountUuid": UUID_B,
            "emailAddress": "b@example.com",
        },
        "diffTool": "terminal",
        "autoConnectIde": true,
        "userID": "b-user",
        "projects": {"/home/me/b": {"allowedTools": ["Bash"]}},
        "externalEditorContext": {"enabled": true, "lines": 3},
    })
}

struct Fixture {
    home: TempHome,
    core: Core,
    env: FakeEnvironment,
    primary: PathBuf,
}

fn fixture() -> Fixture {
    let home = TempHome::new();
    let primary = home.primary();
    primary.global_config(primary_global_config());
    let (core, env) = home.core();
    core.register(&primary.dir).unwrap();
    Fixture {
        primary: primary.dir.clone(),
        home,
        core,
        env,
    }
}

impl Fixture {
    /// A registered linked instance with the given `.claude.json` (none for `None`).
    fn linked(&self, name: &str, global_config: Option<Value>) -> PathBuf {
        let linked = self.home.linked(name);
        if let Some(value) = global_config {
            // Claude Code writes its global config pretty-printed with two spaces.
            write(
                &linked.global_config,
                &(serde_json::to_string_pretty(&value).unwrap() + "\n"),
            );
        }
        self.core.register(&linked.dir).unwrap();
        linked.dir
    }
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

fn outcome<'a>(report: &'a RepairReport, id: &str) -> &'a RepairOutcome {
    &report
        .items
        .iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| panic!("{id} not repaired: {:?}", report.items))
        .outcome
}

/// Writes `installed_plugins.json` (v2) with the given `(id, scope)` entries.
fn installed(config_dir: &Path, plugins: &[(&str, &str)]) {
    let mut map = serde_json::Map::new();
    for (id, scope) in plugins {
        let entries = map
            .entry(id.to_string())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap();
        let mut entry = json!({
            "scope": scope,
            "installPath": config_dir.join("plugins/cache").join(id),
            "version": "1.0.0",
            "installedAt": "2026-10-01T10:00:00.000Z",
        });
        if *scope != "user" {
            entry["projectPath"] = json!("/home/me/project");
        }
        entries.push(entry);
    }
    write(
        config_dir.join("plugins/installed_plugins.json"),
        &json!({"version": 2, "plugins": map}).to_string(),
    );
}

/// Writes `known_marketplaces.json` with the given `(name, source)` entries.
fn marketplaces(config_dir: &Path, entries: &[(&str, Value)]) {
    let mut map = serde_json::Map::new();
    for (name, source) in entries {
        map.insert(
            name.to_string(),
            json!({
                "source": source,
                "installLocation": config_dir.join("plugins/marketplaces").join(name),
                "lastUpdated": "2026-10-01T10:00:00.000Z",
            }),
        );
    }
    write(
        config_dir.join("plugins/known_marketplaces.json"),
        &Value::Object(map).to_string(),
    );
}

/// The primary's plugins: two from `tools` (GitHub), one from `local` (a folder), plus
/// plugins that are never aligned: a project-scoped one, a skills-folder one and a claude.ai
/// one.
fn primary_plugins(p: &Path) {
    marketplaces(
        p,
        &[
            ("tools", json!({"source": "github", "repo": "owner/tools"})),
            (
                "local",
                json!({"source": "directory", "path": "/home/me/my-marketplace"}),
            ),
        ],
    );
    installed(
        p,
        &[
            ("review@tools", "user"),
            ("deploy@tools", "user"),
            ("notes@local", "user"),
            ("lint@tools", "project"),
            ("mine@skills-dir", "user"),
            ("shared@synced", "user"),
        ],
    );
}

/// A `claude plugin` that works like Claude Code's: adding a marketplace records it in the
/// instance's `known_marketplaces.json`, installing a plugin in its `installed_plugins.json`.
/// `fail` maps a plugin id or marketplace source to the message its command fails with.
fn working_claude(env: &FakeEnvironment, fail: &'static [(&'static str, &'static str)]) {
    env.on_claude(move |invocation: &ClaudeInvocation| {
        let args: Vec<&str> = invocation.args.iter().map(String::as_str).collect();
        let dir = &invocation.config_dir;
        let failure = fail.iter().find(|(what, _)| args.contains(what));
        let command = match args.as_slice() {
            ["plugin", "marketplace", "add", ..] => "marketplace-add",
            ["plugin", "install", ..] => "install",
            _ => return Ok(ClaudeRun::exit(1, "", "unexpected command")),
        };
        if let Some((_, message)) = failure {
            let line = json!({"command": command, "outcome": "failed", "message": message});
            return Ok(ClaudeRun::exit(1, &format!("{line}\n"), ""));
        }
        match args.as_slice() {
            ["plugin", "marketplace", "add", source, ..] => {
                let name = source.rsplit(['/', '\\']).next().unwrap().to_owned();
                let path = dir.join("plugins/known_marketplaces.json");
                let mut known: Value = fs::read_to_string(&path)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_else(|| json!({}));
                let name = match name.as_str() {
                    "my-marketplace" => "local".to_owned(),
                    other => other.to_owned(),
                };
                known[&name] = json!({"source": {"source": "github", "repo": source}});
                write(&path, &known.to_string());
            }
            ["plugin", "install", id, ..] => {
                let path = dir.join("plugins/installed_plugins.json");
                let mut file: Value = fs::read_to_string(&path)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_else(|| json!({"version": 2, "plugins": {}}));
                file["plugins"][*id] = json!([{"scope": "user", "version": "1.0.0"}]);
                write(&path, &file.to_string());
            }
            _ => unreachable!(),
        }
        let line = json!({"command": command, "outcome": "ok", "message": "done"});
        Ok(ClaudeRun::ok(&format!("{line}\n")))
    });
}

fn invocations(env: &FakeEnvironment) -> Vec<(PathBuf, Vec<String>)> {
    env.claude_invocations()
        .into_iter()
        .map(|i| (i.config_dir, i.args))
        .collect()
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------------------------
// Global config keys: the doctor

#[test]
fn a_global_config_key_that_differs_is_drift_with_both_values() {
    let f = fixture();
    let b = f.linked(".claude-b", Some(linked_global_config()));

    let drift = f.core.drift(&b).unwrap();

    let diff_tool = item(&drift, "config:diffTool");
    assert_eq!(diff_tool.item, "diffTool");
    assert_eq!(diff_tool.path, b.join(".claude.json"));
    assert_eq!(
        diff_tool.problem,
        Problem::MisalignedKey {
            key: "diffTool".into(),
            primary: json!("auto"),
            instance: Some(json!("terminal")),
        }
    );
    assert_eq!(
        diff_tool.repair,
        Repair::AlignKey {
            key: "diffTool".into(),
            value: json!("auto"),
        }
    );
    assert!(
        diff_tool.explanation.contains("\"auto\""),
        "{}",
        diff_tool.explanation
    );
    assert!(
        diff_tool.explanation.contains("\"terminal\""),
        "{}",
        diff_tool.explanation
    );

    let copy_on_select = item(&drift, "config:copyOnSelect");
    assert_eq!(
        copy_on_select.problem,
        Problem::MisalignedKey {
            key: "copyOnSelect".into(),
            primary: json!(false),
            instance: None,
        }
    );
}

#[test]
fn only_listed_keys_set_in_the_primary_are_compared() {
    let f = fixture();
    let b = f.linked(".claude-b", Some(linked_global_config()));

    let drift = f.core.drift(&b).unwrap();

    // theme, numStartups, projects and userID differ too, but aren't Global config keys to
    // align; autoConnectIde and externalEditorContext match; leftArrowOpensAgents is set in
    // neither.
    assert_eq!(
        ids(&drift),
        vec![
            "config:defaultToAgentsView",
            "config:diffTool",
            "config:copyOnSelect"
        ]
    );
    assert_eq!(drift.drift_count, 3);
}

#[test]
fn a_key_only_the_linked_instance_sets_is_left_alone() {
    let f = fixture();
    let mut linked = primary_global_config();
    linked["prStatusFooterEnabled"] = json!(false);
    let b = f.linked(".claude-b", Some(linked));

    assert_eq!(f.core.drift(&b).unwrap().items, vec![]);
}

#[test]
fn the_primary_instance_has_no_aligned_drift() {
    let f = fixture();
    primary_plugins(&f.primary);

    assert_eq!(f.core.drift(&f.primary).unwrap().items, vec![]);
}

#[test]
fn the_doctor_counts_key_drift_of_every_registered_instance() {
    let f = fixture();
    f.linked(".claude-b", Some(linked_global_config()));
    f.linked(".claude-c", Some(primary_global_config()));

    let doctor = f.core.doctor().unwrap();

    assert_eq!(doctor.drift_count, 3);
    assert!(doctor.has_drift());
}

// ---------------------------------------------------------------------------------------------
// Global config keys: repair

#[test]
fn repair_writes_only_the_listed_keys_and_keeps_everything_else() {
    let f = fixture();
    let b = f.linked(".claude-b", Some(linked_global_config()));
    let primary_before = read(f.home.path().join(".claude.json"));
    let credentials = "{\"claudeAiOauth\":{\"expiresAt\":1}}";
    write(b.join(".credentials.json"), credentials);

    let report = f.core.repair(&b, None).unwrap();

    assert_eq!(report.failed(), 0);
    assert_eq!(report.drift.items, vec![]);
    for id in [
        "config:defaultToAgentsView",
        "config:diffTool",
        "config:copyOnSelect",
    ] {
        assert_eq!(outcome(&report, id), &RepairOutcome::Done { backup: None });
    }
    let mut expected = linked_global_config();
    expected["diffTool"] = json!("auto");
    expected["copyOnSelect"] = json!(false);
    expected["defaultToAgentsView"] = json!(true);
    assert_eq!(read_json(b.join(".claude.json")), expected);
    // Every other key keeps its place; new keys go at the end.
    let text = read(b.join(".claude.json"));
    let order: Vec<&str> = ["numStartups", "theme", "oauthAccount", "diffTool", "userID"]
        .into_iter()
        .collect();
    let positions: Vec<usize> = order
        .iter()
        .map(|k| text.find(&format!("\"{k}\"")).unwrap())
        .collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{text}");
    assert!(text.find("\"externalEditorContext\"") < text.find("\"copyOnSelect\""));
    // Nothing else changes: not the primary, not the credentials, no backup.
    assert_eq!(read(f.home.path().join(".claude.json")), primary_before);
    assert_eq!(read(b.join(".credentials.json")), credentials);
    assert!(!f.home.path().join(".mccboard/state/backups").exists());
    assert!(f.env.claude_invocations().is_empty());
}

#[test]
fn one_key_is_repaired_by_its_id() {
    let f = fixture();
    let b = f.linked(".claude-b", Some(linked_global_config()));

    let report = f.core.repair(&b, Some("config:diffTool")).unwrap();

    assert_eq!(report.items.len(), 1);
    assert_eq!(read_json(b.join(".claude.json"))["diffTool"], json!("auto"));
    assert_eq!(read_json(b.join(".claude.json")).get("copyOnSelect"), None);
    assert_eq!(
        ids(&report.drift),
        vec!["config:defaultToAgentsView", "config:copyOnSelect"]
    );
}

#[test]
fn key_repair_is_idempotent() {
    let f = fixture();
    let b = f.linked(".claude-b", Some(linked_global_config()));
    f.core.repair(&b, None).unwrap();
    let after_first = read(b.join(".claude.json"));

    let second = f.core.repair(&b, None).unwrap();

    assert_eq!(second.items, vec![]);
    assert_eq!(read(b.join(".claude.json")), after_first);
}

#[test]
fn an_instance_without_global_config_gets_one_with_the_keys_only() {
    let f = fixture();
    let b = f.linked(".claude-b", None);

    let report = f.core.repair(&b, None).unwrap();

    assert_eq!(report.failed(), 0);
    assert_eq!(
        read_json(b.join(".claude.json")),
        json!({
            "defaultToAgentsView": true,
            "autoConnectIde": true,
            "diffTool": "auto",
            "externalEditorContext": {"enabled": true, "lines": 3},
            "copyOnSelect": false,
        })
    );
}

#[test]
fn an_unreadable_global_config_is_never_overwritten() {
    let f = fixture();
    let b = f.linked(".claude-b", None);
    write(b.join(".claude.json"), "{ not json");

    let drift = f.core.drift(&b).unwrap();
    let report = f.core.repair(&b, None).unwrap();

    assert_eq!(drift.items, vec![]);
    assert_eq!(report.items, vec![]);
    assert_eq!(read(b.join(".claude.json")), "{ not json");
}

#[test]
fn key_repair_refuses_while_claude_code_runs_in_the_instance() {
    let f = fixture();
    let b = f.linked(".claude-b", Some(linked_global_config()));
    let before = read(b.join(".claude.json"));
    f.env.add_process(ClaudeProcess {
        pid: 4242,
        name: "claude".into(),
        config_dir: ProcessConfigDir::Dir { path: b.clone() },
        cwd: None,
    });

    let error = f.core.repair(&b, None).unwrap_err();

    assert!(matches!(error, Error::InstanceInUse { .. }), "{error}");
    assert_eq!(read(b.join(".claude.json")), before);
}

// ---------------------------------------------------------------------------------------------
// Plugins and marketplaces: the doctor

#[test]
fn plugins_and_marketplaces_missing_from_a_linked_instance_are_drift() {
    let f = fixture();
    primary_plugins(&f.primary);
    let b = f.linked(".claude-b", Some(primary_global_config()));

    let drift = f.core.drift(&b).unwrap();

    assert_eq!(
        ids(&drift),
        vec![
            "marketplace:local",
            "marketplace:tools",
            "plugin:deploy@tools",
            "plugin:notes@local",
            "plugin:review@tools",
        ]
    );
    let tools = item(&drift, "marketplace:tools");
    assert_eq!(
        tools.problem,
        Problem::MissingMarketplace {
            marketplace: "tools".into()
        }
    );
    assert_eq!(
        tools.repair,
        Repair::AddMarketplace {
            marketplace: "tools".into(),
            source: Some("owner/tools".into()),
        }
    );
    assert_eq!(tools.path, b.join("plugins"));
    assert_eq!(
        item(&drift, "marketplace:local").repair,
        Repair::AddMarketplace {
            marketplace: "local".into(),
            source: Some("/home/me/my-marketplace".into()),
        }
    );
    let review = item(&drift, "plugin:review@tools");
    assert_eq!(
        review.problem,
        Problem::MissingPlugin {
            plugin: "review@tools".into()
        }
    );
    assert_eq!(
        review.repair,
        Repair::InstallPlugin {
            plugin: "review@tools".into()
        }
    );
    assert!(review.explanation.contains("review@tools"));
    assert_eq!(drift.drift_count, 5);
}

#[test]
fn what_the_linked_instance_has_installed_for_the_user_is_not_drift() {
    let f = fixture();
    primary_plugins(&f.primary);
    let b = f.linked(".claude-b", Some(primary_global_config()));
    marketplaces(
        &b,
        &[
            ("tools", json!({"source": "github", "repo": "owner/tools"})),
            ("extra", json!({"source": "github", "repo": "owner/extra"})),
        ],
    );
    // deploy is installed only for a project here: it still has to be installed for the user.
    installed(
        &b,
        &[
            ("review@tools", "user"),
            ("deploy@tools", "project"),
            ("only-b@extra", "user"),
        ],
    );

    let drift = f.core.drift(&b).unwrap();

    assert_eq!(
        ids(&drift),
        vec![
            "marketplace:local",
            "plugin:deploy@tools",
            "plugin:notes@local"
        ]
    );
}

#[test]
fn marketplace_sources_become_what_claude_plugin_marketplace_add_takes() {
    let f = fixture();
    marketplaces(
        &f.primary,
        &[
            (
                "pinned",
                json!({"source": "github", "repo": "owner/pinned", "ref": "v2"}),
            ),
            (
                "git",
                json!({"source": "git", "url": "https://git.example.com/m.git"}),
            ),
            (
                "web",
                json!({"source": "url", "url": "https://example.com/marketplace.json"}),
            ),
            ("odd", json!({"source": "npm", "package": "@me/market"})),
        ],
    );
    let b = f.linked(".claude-b", Some(primary_global_config()));

    let drift = f.core.drift(&b).unwrap();

    let source = |id: &str| match &item(&drift, id).repair {
        Repair::AddMarketplace { source, .. } => source.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(source("marketplace:pinned"), Some("owner/pinned#v2".into()));
    assert_eq!(
        source("marketplace:git"),
        Some("https://git.example.com/m.git".into())
    );
    assert_eq!(
        source("marketplace:web"),
        Some("https://example.com/marketplace.json".into())
    );
    assert_eq!(source("marketplace:odd"), None);
}

#[test]
fn a_linked_instance_sharing_the_whole_plugins_folder_has_its_plugins_reported_too() {
    let f = fixture();
    primary_plugins(&f.primary);
    let b = f.linked(".claude-b", Some(primary_global_config()));
    #[cfg(unix)]
    std::os::unix::fs::symlink(f.primary.join("plugins"), b.join("plugins")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(f.primary.join("plugins"), b.join("plugins")).unwrap();

    let drift = f.core.drift(&b).unwrap();

    // Once migrated, the instance's own plugins folder is empty: everything is to install.
    assert_eq!(ids(&drift)[0], "migrate:plugins");
    assert!(ids(&drift).contains(&"plugin:review@tools"));
    assert!(ids(&drift).contains(&"marketplace:tools"));
}

// ---------------------------------------------------------------------------------------------
// Plugins and marketplaces: repair

#[test]
fn repair_adds_marketplaces_then_installs_plugins_with_claude_plugin_in_the_instance() {
    let f = fixture();
    primary_plugins(&f.primary);
    let b = f.linked(".claude-b", Some(primary_global_config()));
    working_claude(&f.env, &[]);
    let primary_plugins_before = read(f.primary.join("plugins/installed_plugins.json"));

    let report = f.core.repair(&b, None).unwrap();

    assert_eq!(report.failed(), 0, "{:?}", report.items);
    assert_eq!(report.drift.items, vec![]);
    let add = |source: &str| args(&["plugin", "marketplace", "add", source, "--json"]);
    let install = |id: &str| args(&["plugin", "install", id, "--scope", "user", "--json"]);
    assert_eq!(
        invocations(&f.env),
        vec![
            (b.clone(), add("/home/me/my-marketplace")),
            (b.clone(), add("owner/tools")),
            (b.clone(), install("deploy@tools")),
            (b.clone(), install("notes@local")),
            (b.clone(), install("review@tools")),
        ]
    );
    // No file is ever copied from the primary's plugins folder.
    assert_eq!(
        read(f.primary.join("plugins/installed_plugins.json")),
        primary_plugins_before
    );
    assert!(!b.join("plugins/cache").exists());
}

#[test]
fn a_failed_install_is_reported_per_plugin_and_the_rest_still_installed() {
    let f = fixture();
    primary_plugins(&f.primary);
    let b = f.linked(".claude-b", Some(primary_global_config()));
    working_claude(&f.env, &[("deploy@tools", "Plugin \"deploy\" not found")]);

    let report = f.core.repair(&b, None).unwrap();

    assert_eq!(report.failed(), 1);
    match outcome(&report, "plugin:deploy@tools") {
        RepairOutcome::Failed { reason } => {
            assert!(reason.contains("Plugin \"deploy\" not found"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        outcome(&report, "plugin:review@tools"),
        &RepairOutcome::Done { backup: None }
    );
    assert_eq!(
        outcome(&report, "plugin:notes@local"),
        &RepairOutcome::Done { backup: None }
    );
    assert_eq!(ids(&report.drift), vec!["plugin:deploy@tools"]);
}

#[test]
fn a_plugin_whose_marketplace_failed_to_add_fails_with_that_reason() {
    let f = fixture();
    primary_plugins(&f.primary);
    let b = f.linked(".claude-b", Some(primary_global_config()));
    working_claude(&f.env, &[("owner/tools", "repository not found")]);

    let report = f.core.repair(&b, None).unwrap();

    for id in [
        "marketplace:tools",
        "plugin:deploy@tools",
        "plugin:review@tools",
    ] {
        match outcome(&report, id) {
            RepairOutcome::Failed { reason } => {
                assert!(reason.contains("repository not found"), "{id}: {reason}")
            }
            other => panic!("{id}: {other:?}"),
        }
    }
    assert_eq!(
        outcome(&report, "plugin:notes@local"),
        &RepairOutcome::Done { backup: None }
    );
    // The failed marketplace is tried once, not once per plugin.
    let adds = invocations(&f.env)
        .into_iter()
        .filter(|(_, a)| a.contains(&"owner/tools".to_owned()))
        .count();
    assert_eq!(adds, 1);
}

#[test]
fn repairing_one_plugin_adds_its_marketplace_first() {
    let f = fixture();
    primary_plugins(&f.primary);
    let b = f.linked(".claude-b", Some(primary_global_config()));
    working_claude(&f.env, &[]);

    let report = f.core.repair(&b, Some("plugin:review@tools")).unwrap();

    assert_eq!(report.failed(), 0);
    assert_eq!(
        invocations(&f.env),
        vec![
            (
                b.clone(),
                args(&["plugin", "marketplace", "add", "owner/tools", "--json"])
            ),
            (
                b.clone(),
                args(&[
                    "plugin",
                    "install",
                    "review@tools",
                    "--scope",
                    "user",
                    "--json"
                ])
            ),
        ]
    );
    assert!(!ids(&report.drift).contains(&"marketplace:tools"));
}

#[test]
fn a_marketplace_mccboard_cant_add_fails_and_says_so() {
    let f = fixture();
    marketplaces(
        &f.primary,
        &[("odd", json!({"source": "npm", "package": "@me/market"}))],
    );
    let b = f.linked(".claude-b", Some(primary_global_config()));
    working_claude(&f.env, &[]);

    let report = f.core.repair(&b, None).unwrap();

    match outcome(&report, "marketplace:odd") {
        RepairOutcome::Failed { reason } => assert!(reason.contains("by hand"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(f.env.claude_invocations().is_empty());
}

#[test]
fn without_claude_code_every_plugin_fails_with_a_reason() {
    let f = fixture();
    primary_plugins(&f.primary);
    let b = f.linked(".claude-b", Some(linked_global_config()));
    f.env.on_claude(|_: &ClaudeInvocation| {
        Err(io::Error::new(io::ErrorKind::NotFound, "no claude"))
    });

    let report = f.core.repair(&b, None).unwrap();

    // Keys don't need Claude Code: they are repaired all the same.
    assert_eq!(
        outcome(&report, "config:diffTool"),
        &RepairOutcome::Done { backup: None }
    );
    match outcome(&report, "plugin:review@tools") {
        RepairOutcome::Failed { reason } => assert!(reason.contains("Claude Code"), "{reason}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn plugin_repair_refuses_while_claude_code_runs_in_the_instance() {
    let f = fixture();
    primary_plugins(&f.primary);
    let b = f.linked(".claude-b", Some(primary_global_config()));
    working_claude(&f.env, &[]);
    f.env.add_process(ClaudeProcess {
        pid: 7,
        name: "claude".into(),
        config_dir: ProcessConfigDir::Dir { path: b.clone() },
        cwd: None,
    });

    let error = f.core.repair(&b, Some("plugin:review@tools")).unwrap_err();

    assert!(matches!(error, Error::InstanceInUse { .. }), "{error}");
    assert!(f.env.claude_invocations().is_empty());
}

#[test]
fn whole_folder_plugins_are_migrated_then_reinstalled_in_one_repair() {
    let f = fixture();
    primary_plugins(&f.primary);
    let b = f.linked(".claude-b", Some(primary_global_config()));
    #[cfg(unix)]
    std::os::unix::fs::symlink(f.primary.join("plugins"), b.join("plugins")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(f.primary.join("plugins"), b.join("plugins")).unwrap();
    working_claude(&f.env, &[]);
    let primary_known = read(f.primary.join("plugins/known_marketplaces.json"));

    let report = f.core.repair(&b, None).unwrap();

    assert_eq!(report.failed(), 0, "{:?}", report.items);
    assert_eq!(report.items[0].id, "migrate:plugins");
    assert!(
        !fs::symlink_metadata(b.join("plugins"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(report.drift.items, vec![]);
    // The primary's plugins were never touched through the old link.
    assert_eq!(
        read(f.primary.join("plugins/known_marketplaces.json")),
        primary_known
    );
}

#[test]
fn a_plugin_is_not_installed_through_a_whole_folder_link() {
    let f = fixture();
    primary_plugins(&f.primary);
    let b = f.linked(".claude-b", Some(primary_global_config()));
    #[cfg(unix)]
    std::os::unix::fs::symlink(f.primary.join("plugins"), b.join("plugins")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(f.primary.join("plugins"), b.join("plugins")).unwrap();
    working_claude(&f.env, &[]);

    let report = f.core.repair(&b, Some("plugin:review@tools")).unwrap();

    match outcome(&report, "plugin:review@tools") {
        RepairOutcome::Failed { reason } => {
            assert!(reason.contains("migrate:plugins"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
    assert!(f.env.claude_invocations().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Aligning a new instance (the add flow, #11)

#[test]
fn align_brings_an_unregistered_instance_in_line_without_linking_anything() {
    let f = fixture();
    primary_plugins(&f.primary);
    write(f.primary.join("settings.json"), "{}");
    let new = f.home.path().join(".claude-new");
    fs::create_dir_all(&new).unwrap();
    working_claude(&f.env, &[]);

    let report = f.core.align(&new).unwrap();

    assert_eq!(report.config_dir, new);
    assert_eq!(report.failed(), 0, "{:?}", report.items);
    assert_eq!(
        read_json(new.join(".claude.json"))["diffTool"],
        json!("auto")
    );
    assert!(new.join("plugins/installed_plugins.json").is_file());
    // Links are the sharing table's job, not alignment's.
    assert!(!new.join("settings.json").exists());
    assert_eq!(
        report
            .drift
            .items
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        vec!["link:settings.json"]
    );
    assert!(f.core.registry().unwrap().instances.len() == 1);
}

#[test]
fn align_refuses_while_claude_code_runs_in_the_instance() {
    let f = fixture();
    let new = f.home.path().join(".claude-new");
    fs::create_dir_all(&new).unwrap();
    f.env.add_process(ClaudeProcess {
        pid: 9,
        name: "claude".into(),
        config_dir: ProcessConfigDir::Dir { path: new.clone() },
        cwd: None,
    });

    let error = f.core.align(&new).unwrap_err();

    assert!(matches!(error, Error::InstanceInUse { .. }), "{error}");
    assert!(!new.join(".claude.json").exists());
}

#[test]
fn aligning_the_primary_instance_changes_nothing() {
    let f = fixture();
    primary_plugins(&f.primary);
    working_claude(&f.env, &[]);
    let before = read(f.home.path().join(".claude.json"));

    let report = f.core.align(&f.primary).unwrap();

    assert_eq!(report.items, vec![]);
    assert_eq!(read(f.home.path().join(".claude.json")), before);
    assert!(f.env.claude_invocations().is_empty());
}
