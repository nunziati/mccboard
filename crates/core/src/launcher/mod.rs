//! Launchers: the command that starts Claude Code in a linked instance (`claude-b`), passing
//! every argument through. The primary instance keeps plain `claude`.
//!
//! What launchers should exist is derived from `config.toml`, never stored: one per registered
//! linked instance, named by its account ([`crate::Account::launcher_name`]) or, without a
//! known account, after its folder. Where they live depends on the OS:
//! - Linux: alias lines in a marked block of the user's shell rc file ([`rc_file`]);
//! - Windows: a copy of mccboard's shim program, `claude-b.exe`, with a sidecar file naming the
//!   instance, in `~/.mccboard/bin`, a folder on the user PATH ([`bin_folder`]).
//!
//! [`status`] compares what is there with what should be (for the doctor); [`sync`] makes it so.

mod bin_folder;
mod rc_file;
mod shell;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::account;
use crate::config::{Config, ConfigFile};
use crate::instance::InstanceKind;
use crate::{Environment, Error, OsKind, Paths, paths};

/// A launcher as found: its command name and the config directory it starts Claude Code in,
/// when that can be read from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Launcher {
    pub name: String,
    pub config_dir: Option<PathBuf>,
}

/// Where this machine's launchers live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LauncherPlace {
    /// Linux: a block marked as mccboard's in the user's shell rc file.
    RcFile { path: PathBuf },
    /// Windows: mccboard's bin folder, which must be on the user PATH.
    BinFolder { path: PathBuf, on_user_path: bool },
}

/// A registered linked instance's launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceLauncher {
    pub config_dir: PathBuf,
    /// The command name it should have.
    pub name: String,
    #[serde(flatten)]
    pub state: LauncherState,
}

/// How an instance's launcher compares with what it should be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LauncherState {
    Ok,
    /// There is no launcher with its name.
    Missing,
    /// A launcher with its name exists but isn't what mccboard writes: it points to another
    /// folder (`found`, when it does and that can be read), was edited by hand, is incomplete,
    /// or was written by an earlier version (on Windows, scripts or an older shim).
    Stale {
        found: Option<PathBuf>,
    },
}

/// The launchers as they are, against what the registered linked instances need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LauncherStatus {
    pub place: LauncherPlace,
    /// One per registered linked instance, in path order.
    pub instances: Vec<InstanceLauncher>,
    /// Launchers mccboard made that no registered instance needs (after a rename or an
    /// unregistration done by hand).
    pub orphans: Vec<Launcher>,
}

impl LauncherStatus {
    /// How many things a [`sync`](crate::Core::sync_launchers) would fix: launchers missing or
    /// stale, orphans, and on Windows a bin folder that isn't on the user PATH.
    pub fn drift_count(&self) -> usize {
        let off_path = match self.place {
            LauncherPlace::BinFolder { on_user_path, .. } => {
                !on_user_path && !self.instances.is_empty()
            }
            LauncherPlace::RcFile { .. } => false,
        };
        self.instances
            .iter()
            .filter(|i| i.state != LauncherState::Ok)
            .count()
            + self.orphans.len()
            + usize::from(off_path)
    }

    /// The launcher of the instance whose config directory is `config_dir`, if it is a
    /// registered linked instance.
    pub fn instance(&self, config_dir: &Path) -> Option<&InstanceLauncher> {
        self.instances
            .iter()
            .find(|i| paths::same(&i.config_dir, config_dir))
    }
}

/// What a sync changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LauncherSync {
    pub created: Vec<Launcher>,
    pub updated: Vec<Launcher>,
    pub removed: Vec<Launcher>,
    /// On Windows, when there are launchers: whether the bin folder is on the user PATH.
    pub user_path: Option<UserPathChange>,
}

/// The bin folder and the user PATH, after a sync on Windows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UserPathChange {
    AlreadyOn,
    /// Added now: terminals opened from now on find the launchers.
    Added,
    /// The user PATH couldn't be read or changed; the user can add the folder by hand.
    NotAdded {
        reason: String,
    },
}

/// A launcher a registered linked instance needs.
#[derive(Debug, Clone)]
struct Wanted {
    name: String,
    config_dir: PathBuf,
}

/// A launcher found in its place, with the text it has, to compare with what mccboard writes.
#[derive(Debug, Clone)]
struct Found {
    name: String,
    config_dir: Option<PathBuf>,
    content: String,
    /// Whether mccboard made it (always, inside the rc block). Only those can be orphans.
    managed: bool,
}

/// Where launchers live on this machine.
enum Store {
    RcFile(rc_file::RcFile),
    BinFolder(bin_folder::BinFolder),
}

impl Store {
    fn of(home: &Path, paths: &Paths, env: &dyn Environment) -> Self {
        match env.os() {
            OsKind::Windows => Store::BinFolder(bin_folder::BinFolder::new(
                paths.bin_dir(),
                env.launcher_shim(),
            )),
            OsKind::Linux => Store::RcFile(rc_file::RcFile::of(home, env.login_shell())),
        }
    }

    fn read(&self) -> Result<Vec<Found>, Error> {
        match self {
            Store::RcFile(rc) => rc.read(),
            Store::BinFolder(bin) => bin.read(),
        }
    }

    /// The text a launcher has when mccboard wrote it, as [`Found::content`] holds it.
    fn render(&self, name: &str, config_dir: &Path) -> String {
        match self {
            Store::RcFile(_) => rc_file::alias_line(name, config_dir),
            Store::BinFolder(_) => bin_folder::content(config_dir),
        }
    }
}

pub(crate) fn status(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
) -> Result<LauncherStatus, Error> {
    let config = ConfigFile::load(&paths.config_file())?.config;
    let store = Store::of(home, paths, env);
    let comparison = compare(&store, &wanted(home, &config), store.read()?);
    Ok(LauncherStatus {
        place: place(&store, env),
        instances: comparison.instances,
        orphans: comparison.orphans.into_iter().map(launcher).collect(),
    })
}

pub(crate) fn sync(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
) -> Result<LauncherSync, Error> {
    let config = ConfigFile::load(&paths.config_file())?.config;
    let store = Store::of(home, paths, env);
    let wanted = wanted(home, &config);
    let comparison = compare(&store, &wanted, store.read()?);

    match &store {
        Store::RcFile(rc) => rc.write(&wanted)?,
        Store::BinFolder(bin) => {
            bin.sweep();
            for (want, status) in wanted.iter().zip(&comparison.instances) {
                if status.state != LauncherState::Ok {
                    bin.write(&want.name, &want.config_dir)?;
                }
            }
            for orphan in &comparison.orphans {
                bin.remove(&orphan.name)?;
            }
        }
    }

    let changed = |pick: fn(&LauncherState) -> bool| {
        comparison
            .instances
            .iter()
            .filter(|i| pick(&i.state))
            .map(|i| Launcher {
                name: i.name.clone(),
                config_dir: Some(i.config_dir.clone()),
            })
            .collect()
    };
    let user_path = match &store {
        Store::BinFolder(bin) if !wanted.is_empty() => Some(put_on_user_path(env, bin.path())),
        _ => None,
    };
    Ok(LauncherSync {
        created: changed(|s| *s == LauncherState::Missing),
        updated: changed(|s| matches!(s, LauncherState::Stale { .. })),
        removed: comparison.orphans.into_iter().map(launcher).collect(),
        user_path,
    })
}

fn place(store: &Store, env: &dyn Environment) -> LauncherPlace {
    match store {
        Store::RcFile(rc) => LauncherPlace::RcFile {
            path: rc.path().to_path_buf(),
        },
        Store::BinFolder(bin) => LauncherPlace::BinFolder {
            path: bin.path().to_path_buf(),
            on_user_path: env.user_path_contains(bin.path()).unwrap_or(false),
        },
    }
}

fn put_on_user_path(env: &dyn Environment, dir: &Path) -> UserPathChange {
    match env.user_path_contains(dir) {
        Ok(true) => UserPathChange::AlreadyOn,
        Ok(false) => match env.add_to_user_path(dir) {
            Ok(()) => UserPathChange::Added,
            Err(e) => UserPathChange::NotAdded {
                reason: e.to_string(),
            },
        },
        Err(e) => UserPathChange::NotAdded {
            reason: e.to_string(),
        },
    }
}

fn launcher(found: Found) -> Launcher {
    Launcher {
        name: found.name,
        config_dir: found.config_dir,
    }
}

struct Comparison {
    /// In the order of the wanted launchers.
    instances: Vec<InstanceLauncher>,
    orphans: Vec<Found>,
}

fn compare(store: &Store, wanted: &[Wanted], found: Vec<Found>) -> Comparison {
    let key = |name: &str| name.to_lowercase();
    let instances = wanted
        .iter()
        .map(|want| {
            let state = match found.iter().find(|f| key(&f.name) == key(&want.name)) {
                None => LauncherState::Missing,
                Some(f) if f.content == store.render(&want.name, &want.config_dir) => {
                    LauncherState::Ok
                }
                Some(f) => LauncherState::Stale {
                    found: f
                        .config_dir
                        .clone()
                        .filter(|found| !paths::same(found, &want.config_dir)),
                },
            };
            InstanceLauncher {
                config_dir: want.config_dir.clone(),
                name: want.name.clone(),
                state,
            }
        })
        .collect();
    let wanted_names: HashSet<String> = wanted.iter().map(|w| key(&w.name)).collect();
    let orphans = found
        .into_iter()
        .filter(|f| f.managed && !wanted_names.contains(&key(&f.name)))
        .collect();
    Comparison { instances, orphans }
}

/// The launchers the registered linked instances need, in path order. Names are unique,
/// ignoring case: a name wanted twice (two instances of one account) gets `-2`, `-3`… on the
/// later instances.
fn wanted(home: &Path, config: &Config) -> Vec<Wanted> {
    let mut instances: Vec<_> = config
        .instances
        .iter()
        .filter(|i| InstanceKind::of(home, &i.config_dir) == InstanceKind::Linked)
        .collect();
    instances.sort_by_key(|i| paths::key(&i.config_dir));
    instances.dedup_by(|a, b| paths::same(&a.config_dir, &b.config_dir));

    let mut taken = HashSet::new();
    instances
        .into_iter()
        .map(|instance| {
            let base = instance
                .account
                .as_ref()
                .and_then(|uuid| config.account(uuid))
                .map_or_else(|| folder_name(&instance.config_dir), |a| a.launcher_name());
            let name = (1..)
                .map(|n| {
                    if n == 1 {
                        base.clone()
                    } else {
                        format!("{base}-{n}")
                    }
                })
                .find(|name| taken.insert(name.to_lowercase()))
                .expect("an endless sequence has a free name");
            Wanted {
                name,
                config_dir: instance.config_dir.clone(),
            }
        })
        .collect()
}

/// The launcher name of an instance with no known account, from its folder:
/// `.claude-work` → `claude-work`, `Team X` → `claude-team-x`.
fn folder_name(config_dir: &Path) -> String {
    let folder = config_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = folder.trim_start_matches('.');
    let slug = account::slug(folder.strip_prefix("claude-").unwrap_or(folder));
    if slug.is_empty() {
        "claude-instance".to_owned()
    } else {
        format!("claude-{slug}")
    }
}
