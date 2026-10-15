//! The doctor: how each instance differs from what its sharing says (drift), item by item,
//! each with a plain-words explanation and the repair that would fix it.
//!
//! Drift is a list of [`DriftItem`]s. Each has a stable `id` naming the item it is about,
//! `<family>:<subject>` (`link:settings.json`, `link:skills/review`, `migrate:skills`), which
//! stays the same whatever is wrong with the item, so a repair can target it. Its [`Problem`]
//! says what is wrong and its [`Repair`] what a repair would do. Launchers have their own
//! family (`launcher:claude-b`), and so do aligned items: Global config keys
//! (`config:diffTool`), marketplaces (`marketplace:tools`) and plugins (`plugin:review@tools`),
//! see [`crate::alignment`].

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::ConfigFile;
use crate::instance::InstanceKind;
use crate::launcher::{LauncherPlace, LauncherState, LauncherStatus};
use crate::links::{self, Entry};
use crate::sharing::{self, Sharing, SharingAnswer, SharingQuestion};
use crate::{Error, alignment, paths};

/// The doctor's report on every registered instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Doctor {
    /// Registered instances, primary first then by path, each with its drift.
    pub instances: Vec<InstanceDrift>,
    /// Unknown items of the primary instance waiting for an answer ("share `gsd-core`?").
    pub questions: Vec<SharingQuestion>,
    /// Drift items across every instance, informational ones left out.
    pub drift_count: usize,
}

impl Doctor {
    /// Whether any instance has drift (informational items don't count).
    pub fn has_drift(&self) -> bool {
        self.drift_count > 0
    }
}

/// One instance's drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceDrift {
    pub config_dir: PathBuf,
    pub kind: InstanceKind,
    /// Every drift item, informational ones included, in a stable order.
    pub items: Vec<DriftItem>,
    /// Items that are drift, informational ones left out.
    pub drift_count: usize,
}

impl InstanceDrift {
    fn new(config_dir: PathBuf, kind: InstanceKind, items: Vec<DriftItem>) -> Self {
        let drift_count = items.iter().filter(|i| !i.informational).count();
        Self {
            config_dir,
            kind,
            items,
            drift_count,
        }
    }
}

/// One difference between an instance and what its sharing says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DriftItem {
    /// Stable identifier of the item, `<family>:<subject>`: the same across runs whatever is
    /// wrong with the item. Repairs target it.
    pub id: String,
    /// The item, relative to the config directory, with `/` separators (`skills/review`); for
    /// a launcher, its command name (`claude-b`).
    pub item: String,
    /// The item's full path in the instance; for a launcher, where launchers live (the shell
    /// rc file, or mccboard's bin folder).
    pub path: PathBuf,
    pub problem: Problem,
    /// What a repair would do.
    pub repair: Repair,
    /// Reported for information only: not counted as drift, nothing to repair.
    pub informational: bool,
    /// The problem in plain words (English; the app translates from `problem`).
    pub explanation: String,
    /// The repair in plain words (English; the app translates from `repair`).
    pub repair_explanation: String,
}

/// What is wrong with an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Problem {
    /// A linked item isn't there at all.
    MissingLink,
    /// A personal skill of the primary instance isn't linked in this instance yet.
    UnlinkedSkill { skill: String },
    /// A link leads nowhere. `target` is where it points, if readable.
    BrokenLink { target: Option<PathBuf> },
    /// A link points somewhere other than the primary instance's item.
    WrongTarget { target: PathBuf },
    /// A link points at another link: more than one hop to the primary's item.
    LinkChain { target: PathBuf },
    /// A separate copy (a real file or folder) where a link belongs.
    NotALink,
    /// `skills/` or `plugins/` linked as a whole folder: needs migrating to the safe layout.
    WholeFolderLink { target: Option<PathBuf> },
    /// The primary's `settings.json` has hooks, which may run twice in a linked instance
    /// (anthropics/claude-code#80791). Informational.
    DuplicateHooks,
    /// The instance has no launcher (`claude-b`) to start Claude Code in it.
    MissingLauncher,
    /// The instance's launcher isn't what mccboard writes: it starts Claude Code in another
    /// folder (`found`, when readable), was edited by hand, or is incomplete.
    StaleLauncher { found: Option<PathBuf> },
    /// A Global config key of `.claude.json` differs from the primary's: `primary` is the
    /// primary's value, `instance` this instance's (`None` when it doesn't set it).
    MisalignedKey {
        key: String,
        primary: serde_json::Value,
        instance: Option<serde_json::Value>,
    },
    /// A plugin marketplace added in the primary instance isn't added here.
    MissingMarketplace { marketplace: String },
    /// A plugin installed for the user in the primary instance isn't installed here
    /// (`name@marketplace`).
    MissingPlugin { plugin: String },
}

/// What a repair of an item would do. Link repairs make the item a single-hop link to
/// `target`, the real item in the primary instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Repair {
    /// Create a link to `target` where nothing is.
    CreateLink { target: PathBuf },
    /// Replace the link that is there with a link to `target`.
    ReplaceLink { target: PathBuf },
    /// Move the real file or folder that is there to a backup under `~/.mccboard/state/`,
    /// then link to `target`.
    BackupAndLink { target: PathBuf },
    /// Replace the whole-folder `skills/` link with a real folder holding one link per
    /// personal skill of the primary instance (`skills`, by name).
    MigrateSkills { skills: Vec<String> },
    /// Replace the whole-folder `plugins/` link with the instance's own folder; its plugins
    /// are then reinstalled with Claude Code's own commands.
    MigratePlugins,
    /// Make the launchers match the registered linked instances ([`crate::Core::sync_launchers`]).
    SyncLaunchers,
    /// Set the Global config key `key` to `value` in this instance's `.claude.json`, leaving
    /// every other key as it is.
    AlignKey {
        key: String,
        value: serde_json::Value,
    },
    /// Run `claude plugin marketplace add <source>` in this instance. `source` is `None` when
    /// mccboard can't tell it from the primary's: the marketplace is then added by hand.
    AddMarketplace {
        marketplace: String,
        source: Option<String>,
    },
    /// Run `claude plugin install <plugin> --scope user` in this instance, adding its
    /// marketplace first when missing.
    InstallPlugin { plugin: String },
    /// Nothing to repair.
    Nothing,
}

/// `launchers` is the launchers' status, or `None` when it couldn't be read: the doctor then
/// reports link drift only, and the launchers' own status says what is wrong.
pub(crate) fn doctor(
    home: &Path,
    config_file: &Path,
    launchers: Option<&LauncherStatus>,
) -> Result<Doctor, Error> {
    let config = ConfigFile::load(config_file)?.config;
    let answers = &config.sharing;

    let mut instances: Vec<InstanceDrift> = config
        .instances
        .iter()
        .map(|entry| drift(home, &entry.config_dir, answers, launchers))
        .collect();
    instances.sort_by(|a, b| {
        (a.kind != InstanceKind::Primary, paths::key(&a.config_dir))
            .cmp(&(b.kind != InstanceKind::Primary, paths::key(&b.config_dir)))
    });
    instances.dedup_by(|a, b| paths::same(&a.config_dir, &b.config_dir));

    let drift_count = instances.iter().map(|i| i.drift_count).sum();
    Ok(Doctor {
        questions: sharing::questions(&primary_dir(home), answers),
        instances,
        drift_count,
    })
}

pub(crate) fn primary_dir(home: &Path) -> PathBuf {
    home.join(".claude")
}

/// The drift of the instance in `config_dir`, against the primary instance in `home`, then
/// its launcher's, when `launchers` has one for it.
pub(crate) fn drift(
    home: &Path,
    config_dir: &Path,
    answers: &BTreeMap<String, SharingAnswer>,
    launchers: Option<&LauncherStatus>,
) -> InstanceDrift {
    let kind = InstanceKind::of(home, config_dir);
    let primary = primary_dir(home);
    let mut items = if kind == InstanceKind::Primary || !config_dir.is_dir() || !primary.is_dir() {
        // The primary holds the real items; a missing folder has nothing to compare.
        Vec::new()
    } else {
        let mut items = link_drift(&primary, config_dir, answers);
        items.extend(duplicate_hooks(&primary, config_dir));
        items.extend(alignment::drift(home, &primary, config_dir));
        items
    };
    items.extend(launchers.and_then(|status| launcher_drift(status, config_dir)));
    InstanceDrift::new(config_dir.to_path_buf(), kind, items)
}

/// The launcher of the instance in `config_dir`, when it is missing or stale. Only registered
/// linked instances have one.
fn launcher_drift(status: &LauncherStatus, config_dir: &Path) -> Option<DriftItem> {
    let launcher = status.instance(config_dir)?;
    let problem = match &launcher.state {
        LauncherState::Ok => return None,
        LauncherState::Missing => Problem::MissingLauncher,
        LauncherState::Stale { found } => Problem::StaleLauncher {
            found: found.clone(),
        },
    };
    let place = match &status.place {
        LauncherPlace::RcFile { path } | LauncherPlace::BinFolder { path, .. } => path,
    };
    Some(DriftItem::new(
        format!("launcher:{}", launcher.name),
        &launcher.name,
        place,
        problem,
        Repair::SyncLaunchers,
    ))
}

/// Linked items, then each personal skill, then whole-folder links of `skills/` and
/// `plugins/`.
fn link_drift(
    primary: &Path,
    linked: &Path,
    answers: &BTreeMap<String, SharingAnswer>,
) -> Vec<DriftItem> {
    let mut items = Vec::new();

    // The table's linked items in its order, then unknown items answered "link".
    let mut names: Vec<String> = sharing::LINKED_ITEMS
        .iter()
        .map(|s| s.to_string())
        .collect();
    names.extend(
        answers
            .iter()
            .filter(|(name, _)| sharing::classify(name, answers) == Sharing::Linked)
            .filter(|(name, _)| !sharing::LINKED_ITEMS.contains(&name.as_str()))
            .map(|(name, _)| name.clone()),
    );
    for name in names {
        let source = primary.join(&name);
        if links::inspect(&source) == Entry::Missing {
            continue;
        }
        items.extend(check_link(
            &name,
            &source,
            &linked.join(&name),
            Problem::MissingLink,
        ));
    }

    let skills_dir = linked.join(sharing::SKILLS);
    let personal = sharing::personal_skills(primary);
    match links::inspect(&skills_dir) {
        Entry::Link { target } => items.push(whole_folder(
            sharing::SKILLS,
            &skills_dir,
            target,
            Repair::MigrateSkills { skills: personal },
        )),
        _ => {
            for skill in personal {
                let item = format!("{}/{skill}", sharing::SKILLS);
                let source = primary.join(sharing::SKILLS).join(&skill);
                let missing = Problem::UnlinkedSkill {
                    skill: skill.clone(),
                };
                items.extend(check_link(
                    &item,
                    &source,
                    &skills_dir.join(&skill),
                    missing,
                ));
            }
        }
    }

    let plugins_dir = linked.join(sharing::PLUGINS);
    if let Entry::Link { target } = links::inspect(&plugins_dir) {
        items.push(whole_folder(
            sharing::PLUGINS,
            &plugins_dir,
            target,
            Repair::MigratePlugins,
        ));
    }
    items
}

/// Checks that `at` is a single-hop link to the real item behind `source`.
fn check_link(item: &str, source: &Path, at: &Path, missing: Problem) -> Option<DriftItem> {
    let real = links::real_item(source);
    let (problem, repair) = match links::inspect(at) {
        Entry::Missing => (missing, Repair::CreateLink { target: real }),
        Entry::Real => (Problem::NotALink, Repair::BackupAndLink { target: real }),
        Entry::Link { target } => {
            let replace = Repair::ReplaceLink {
                target: real.clone(),
            };
            match target {
                None => (Problem::BrokenLink { target: None }, replace),
                Some(target) => {
                    let destination = links::destination(at);
                    if destination.is_none() {
                        (
                            Problem::BrokenLink {
                                target: Some(target),
                            },
                            replace,
                        )
                    } else if links::is_link(&target) {
                        // More than one hop. If it ends somewhere else, that's what matters.
                        let ends_right = destination == links::destination(&real);
                        if ends_right {
                            (Problem::LinkChain { target }, replace)
                        } else {
                            (Problem::WrongTarget { target }, replace)
                        }
                    } else if links::same_place(&target, &real) {
                        return None;
                    } else {
                        (Problem::WrongTarget { target }, replace)
                    }
                }
            }
        }
    };
    Some(DriftItem::new(
        format!("link:{item}"),
        item,
        at,
        problem,
        repair,
    ))
}

fn whole_folder(item: &str, at: &Path, target: Option<PathBuf>, repair: Repair) -> DriftItem {
    let problem = Problem::WholeFolderLink { target };
    DriftItem::new(format!("migrate:{item}"), item, at, problem, repair)
}

/// The primary's `settings.json` with hooks also loads as project settings when Claude Code
/// runs from the home folder (anthropics/claude-code#80791), so in a linked instance sharing
/// it those hooks may run twice.
fn duplicate_hooks(primary: &Path, linked: &Path) -> Option<DriftItem> {
    let settings = primary.join("settings.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(settings).ok()?).ok()?;
    let has_hooks = value
        .get("hooks")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|hooks| !hooks.is_empty());
    has_hooks.then(|| {
        DriftItem::new(
            "hooks:duplicates".to_owned(),
            "settings.json",
            &linked.join("settings.json"),
            Problem::DuplicateHooks,
            Repair::Nothing,
        )
    })
}

impl DriftItem {
    pub(crate) fn new(
        id: String,
        item: &str,
        path: &Path,
        problem: Problem,
        repair: Repair,
    ) -> Self {
        let informational = matches!(problem, Problem::DuplicateHooks);
        Self {
            explanation: explain(item, &problem),
            repair_explanation: explain_repair(item, &repair),
            id,
            item: item.to_owned(),
            path: path.to_path_buf(),
            problem,
            repair,
            informational,
        }
    }
}

fn explain(item: &str, problem: &Problem) -> String {
    match problem {
        Problem::MissingLink => {
            format!("{item} is missing: it should be a link to the primary instance's {item}.")
        }
        Problem::UnlinkedSkill { skill } => format!(
            "The personal skill \"{skill}\" is in the primary instance but isn't linked here, \
             so this account can't use it."
        ),
        Problem::BrokenLink {
            target: Some(target),
        } => format!(
            "{item} is a link to {}, which doesn't exist.",
            target.display()
        ),
        Problem::BrokenLink { target: None } => {
            format!("{item} is a link that can't be read.")
        }
        Problem::WrongTarget { target } => format!(
            "{item} is a link to {}, not to the primary instance's {item}.",
            target.display()
        ),
        Problem::LinkChain { target } => format!(
            "{item} is a link to {}, which is a link itself: more than one hop. Claude Code \
             saves settings by following a single hop, so a chain can break.",
            target.display()
        ),
        Problem::NotALink => format!(
            "{item} is a separate copy, not a link to the primary instance's {item}, so \
             changes to one don't reach the other."
        ),
        Problem::WholeFolderLink { target } => {
            let to = target
                .as_ref()
                .map(|t| format!(" (to {})", t.display()))
                .unwrap_or_default();
            if item == sharing::SKILLS {
                format!(
                    "skills is linked as a whole folder{to}, so every account shares the same \
                     skills/synced folder, where Claude Code keeps each account's claude.ai \
                     skills."
                )
            } else {
                format!(
                    "{item} is linked as a whole folder{to}, which breaks plugin marketplace \
                     validation in both instances."
                )
            }
        }
        Problem::DuplicateHooks => "The primary instance's settings.json defines hooks. When \
             Claude Code runs in this instance from your home folder, it also loads that file \
             as project settings, so those hooks may run twice \
             (anthropics/claude-code#80791)."
            .to_owned(),
        Problem::MissingLauncher => format!(
            "The launcher {item} is missing, so no command starts Claude Code in this instance."
        ),
        Problem::StaleLauncher { found: Some(found) } => format!(
            "The launcher {item} starts Claude Code in {} instead of this instance.",
            found.display()
        ),
        Problem::StaleLauncher { found: None } => format!(
            "The launcher {item} isn't the one mccboard writes: it was edited by hand or is \
             incomplete."
        ),
        Problem::MisalignedKey {
            key,
            primary,
            instance: Some(instance),
        } => format!(
            "The Global config key {key} is {} here but {} in the primary instance.",
            json(instance),
            json(primary)
        ),
        Problem::MisalignedKey {
            key,
            primary,
            instance: None,
        } => format!(
            "The Global config key {key} isn't set here; the primary instance has {}.",
            json(primary)
        ),
        Problem::MissingMarketplace { marketplace } => format!(
            "The plugin marketplace {marketplace} is added in the primary instance but not here."
        ),
        Problem::MissingPlugin { plugin } => format!(
            "The plugin {plugin} is installed in the primary instance but not here, so this \
             account doesn't have it."
        ),
    }
}

fn explain_repair(item: &str, repair: &Repair) -> String {
    match repair {
        Repair::CreateLink { target } => {
            format!("Create {item} as a link to {}.", target.display())
        }
        Repair::ReplaceLink { target } => {
            format!("Replace it with a single link to {}.", target.display())
        }
        Repair::BackupAndLink { target } => format!(
            "Move this copy to a backup in ~/.mccboard/state/, then link {item} to {}.",
            target.display()
        ),
        Repair::MigrateSkills { skills } => format!(
            "Make skills a real folder in this instance with one link per personal skill \
             ({}), and keep this account's skills/synced private.",
            if skills.is_empty() {
                "none yet".to_owned()
            } else {
                skills.join(", ")
            }
        ),
        Repair::MigratePlugins => "Give this instance its own plugins folder; its plugins are \
             then reinstalled with Claude Code's own `claude plugin` commands."
            .to_owned(),
        Repair::SyncLaunchers => {
            "Fix the launchers (mccboard launcher sync), then open a new terminal.".to_owned()
        }
        Repair::AlignKey { key, value } => format!(
            "Set {key} to {} in this instance's .claude.json, leaving every other key as it is.",
            json(value)
        ),
        Repair::AddMarketplace {
            source: Some(source),
            ..
        } => format!("Add it with `claude plugin marketplace add {source}`, run in this instance."),
        Repair::AddMarketplace { source: None, .. } => "mccboard can't tell how to add it: add \
             it by hand with `claude plugin marketplace add`, run in this instance."
            .to_owned(),
        Repair::InstallPlugin { plugin } => format!(
            "Install it with `claude plugin install {plugin}`, run in this instance (adding its \
             marketplace first if needed)."
        ),
        Repair::Nothing => "Nothing to repair: this is for your information.".to_owned(),
    }
}

/// A JSON value as Claude Code writes it, on one line.
fn json(value: &serde_json::Value) -> String {
    value.to_string()
}
