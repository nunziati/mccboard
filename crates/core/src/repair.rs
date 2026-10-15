//! Repair: fixing an instance's drift, all of it or one item, as the doctor describes it.
//!
//! Before changing anything a repair checks, in order:
//! 1. the instance is registered and linked (the primary instance is never relinked);
//! 2. no link would land in the Claude desktop app's data folder;
//! 3. file links can be made (Developer Mode on Windows, ADR 0006);
//! 4. no Claude Code process uses the instance (the session guard).
//!
//! Then it repairs item by item: a failed item is reported and the others still repaired.
//! Every link is a single hop to the primary's real item; a real file or folder in the way
//! is moved to a backup first. Private items are never touched: repairs act only on the
//! items the doctor reports, which are never private.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::backup::Backup;
use crate::config::ConfigFile;
use crate::doctor::{self, DriftItem, InstanceDrift, Repair};
use crate::instance::{self, InstanceKind};
use crate::links::{self, Entry};
use crate::sharing;
use crate::{Environment, Error, Paths, desktop_app, paths, session_guard};

/// What a repair did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepairReport {
    pub config_dir: PathBuf,
    /// The items repaired, in the doctor's order, each with its outcome. Empty when there was
    /// nothing to repair.
    pub items: Vec<RepairedItem>,
    /// The instance's drift after the repair.
    pub drift: InstanceDrift,
}

impl RepairReport {
    /// How many items failed.
    pub fn failed(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.outcome, RepairOutcome::Failed { .. }))
            .count()
    }
}

/// One item a repair worked on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepairedItem {
    /// The drift item's id (`link:settings.json`).
    pub id: String,
    pub item: String,
    /// What the repair set out to do.
    pub repair: Repair,
    pub outcome: RepairOutcome,
}

/// How repairing one item went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RepairOutcome {
    /// Repaired. `backup` is where the real item it replaced went, if any.
    Done { backup: Option<PathBuf> },
    /// Not repaired; `reason` says why (paths and OS errors only). The item is left as it
    /// was, or missing when the link it had couldn't be replaced.
    Failed { reason: String },
}

pub(crate) fn repair(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
    config_dir: &Path,
    only: Option<&str>,
) -> Result<RepairReport, Error> {
    let config_dir = paths::resolve(home, config_dir)?;
    let config = ConfigFile::load(&paths.config_file())?.config;
    if config.instance(&config_dir).is_none() {
        return Err(Error::NotRegistered { path: config_dir });
    }
    if !config_dir.is_dir() {
        return Err(Error::NotADirectory { path: config_dir });
    }
    let answers = &config.sharing;
    let drift = doctor::drift(home, &config_dir, answers);

    let selected: Vec<&DriftItem> = match only {
        None => drift.items.iter().collect(),
        Some(id) => {
            let found = drift.items.iter().find(|i| i.id == id);
            vec![found.ok_or_else(|| Error::DriftItemNotFound {
                path: config_dir.clone(),
                id: id.to_owned(),
            })?]
        }
    };
    let selected: Vec<&DriftItem> = selected
        .into_iter()
        .filter(|i| !i.informational && i.repair != Repair::Nothing)
        .collect();
    if selected.is_empty() || drift.kind == InstanceKind::Primary {
        return Ok(RepairReport {
            config_dir,
            items: Vec::new(),
            drift,
        });
    }

    let into_desktop_app = std::iter::once(config_dir.as_path())
        .chain(selected.iter().map(|i| i.path.as_path()))
        .any(|path| desktop_app::contains(home, path));
    if into_desktop_app {
        return Err(Error::DesktopAppFolder { path: config_dir });
    }
    if selected.iter().any(|i| needs_file_link(&i.repair)) && !env.file_links_allowed() {
        return Err(Error::DeveloperModeOff);
    }
    session_guard::ensure_idle(home, env, &config_dir)?;

    let primary = doctor::primary_dir(home);
    let mut backup = Backup::new(paths.backups_dir(), env.now(), home, &config_dir);
    let mut items = Vec::new();
    for item in selected {
        let outcome = match apply(home, env, &primary, &config_dir, item, &mut backup) {
            Ok(backup) => RepairOutcome::Done { backup },
            Err(error) => RepairOutcome::Failed {
                reason: error.to_string(),
            },
        };
        items.push(RepairedItem {
            id: item.id.clone(),
            item: item.item.clone(),
            repair: item.repair.clone(),
            outcome,
        });
    }

    Ok(RepairReport {
        drift: doctor::drift(home, &config_dir, answers),
        config_dir,
        items,
    })
}

fn needs_file_link(repair: &Repair) -> bool {
    match repair {
        Repair::CreateLink { target }
        | Repair::ReplaceLink { target }
        | Repair::BackupAndLink { target } => links::is_file_link(target),
        // Skills are folders; plugins get no link.
        Repair::MigrateSkills { .. } | Repair::MigratePlugins | Repair::Nothing => false,
    }
}

/// Repairs one item; returns where a replaced real item was backed up, if any.
fn apply(
    home: &Path,
    env: &dyn Environment,
    primary: &Path,
    config_dir: &Path,
    item: &DriftItem,
    backup: &mut Backup,
) -> io::Result<Option<PathBuf>> {
    let at = &item.path;
    match &item.repair {
        Repair::CreateLink { target } => {
            expect(at, matches!(links::inspect(at), Entry::Missing), "free")?;
            if let Some(parent) = at.parent() {
                fs::create_dir_all(parent)?;
            }
            links::create_link(target, at)?;
            Ok(None)
        }
        Repair::ReplaceLink { target } => {
            expect(at, links::is_link(at), "a link")?;
            links::remove_link(at)?;
            links::create_link(target, at)?;
            Ok(None)
        }
        Repair::BackupAndLink { target } => {
            expect(
                at,
                links::inspect(at) == Entry::Real,
                "a real file or folder",
            )?;
            let kept = backup.keep(at, &item.item)?;
            if let Err(error) = links::create_link(target, at) {
                // Put the original back rather than leave a hole.
                let _ = backup.restore(&kept, at);
                return Err(error);
            }
            Ok(Some(kept))
        }
        Repair::MigrateSkills { skills } => {
            migrate_skills(home, env, primary, config_dir, at, skills)?;
            Ok(None)
        }
        Repair::MigratePlugins => {
            expect(at, links::is_link(at), "a link")?;
            links::remove_link(at)?;
            fs::create_dir(at)?;
            Ok(None)
        }
        Repair::Nothing => Ok(None),
    }
}

/// Checks that an item is still as the doctor saw it.
fn expect(at: &Path, ok: bool, what: &str) -> io::Result<()> {
    if ok {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} is no longer {what}; run the doctor again",
            at.display()
        )))
    }
}

/// Replaces the whole-folder `skills/` link with a real folder: the instance's own claude.ai
/// skills (`synced/`) copied from where the link led, then one link per personal skill.
fn migrate_skills(
    home: &Path,
    env: &dyn Environment,
    primary: &Path,
    config_dir: &Path,
    at: &Path,
    skills: &[String],
) -> io::Result<()> {
    expect(at, links::is_link(at), "a link")?;
    let shared = links::destination(at);
    let primary_skills = links::destination(&primary.join(sharing::SKILLS));
    let synced = shared.as_ref().map(|s| s.join(SYNCED));
    // Folder names of synced/ to keep: all of them when the folder was the instance's own;
    // when it was the primary's, only this account's (`<org>_<account>`, `.bucket-…`).
    let keep: Box<dyn Fn(&str) -> bool> = if shared.is_some() && shared == primary_skills {
        match instance::observe(home, config_dir, env).account {
            Some(account) => {
                let suffix = format!("_{}", account.uuid.as_str());
                Box::new(move |name: &str| name.ends_with(&suffix))
            }
            None => Box::new(|_: &str| false),
        }
    } else {
        Box::new(|_: &str| true)
    };

    links::remove_link(at)?;
    fs::create_dir(at)?;
    if let Some(synced) = synced.filter(|s| s.is_dir()) {
        for entry in fs::read_dir(&synced)? {
            let entry = entry?;
            let name = entry.file_name();
            if keep(&name.to_string_lossy()) {
                let to = at.join(SYNCED);
                fs::create_dir_all(&to)?;
                copy_all(&entry.path(), &to.join(&name))?;
            }
        }
    }
    for skill in skills {
        let target = links::real_item(&primary.join(sharing::SKILLS).join(skill));
        links::create_link(&target, &at.join(skill))?;
    }
    Ok(())
}

const SYNCED: &str = "synced";

/// Copies a file or folder; links inside are skipped (synced skills hold none).
fn copy_all(from: &Path, to: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_all(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if meta.is_file() {
        fs::copy(from, to)?;
    }
    Ok(())
}
