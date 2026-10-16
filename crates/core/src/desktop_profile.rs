//! Desktop profiles (ADR 0008, spec #34): copies of the Claude desktop app, each with its own
//! data directory and sign-in, bound to one account.
//!
//! - **Stored** in `config.toml` (`[[desktop_profiles]]`): the account, the data folder and
//!   whether it opens when mccboard starts. The instance its Code tab uses is derived: the
//!   account's local (linked) instance.
//! - **The default desktop profile** is implicit, never stored and never removed: the app's
//!   own data directory, paired with the primary instance. Its account shows Claude's orange
//!   in every view model ([`crate::CLAUDE_ORANGE`]).
//! - **Rules**, each with its typed error: a profile needs a registered account with a local
//!   instance; one profile per account (the primary instance's account has the default one);
//!   the data folder stays out of instances, mccboard's files, the app's own data and other
//!   profiles' folders, and holds none of them.
//! - **Observation**: a profile's account is read from the names of the folders under
//!   `<data folder>/claude-code-sessions/` (`<account uuid>/<org uuid>/…`), and from nothing
//!   else. mccboard never opens a file in a data folder: `config.json` holds the app's token
//!   cache (ADR 0002).
//! - **Removal**: from mccboard only, or with the data folder, refused while a process of the
//!   app uses it (the environment port lists them, as the session guard does for Claude Code).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

use crate::account::{self, Account, AccountUuid, Color};
use crate::config::{Config, ConfigFile, DesktopProfileEntry};
use crate::instance::InstanceKind;
use crate::links::{self, Entry};
use crate::{
    Environment, Error, FolderOutcome, IconCache, OsKind, Paths, RemoveLevel, desktop_app,
    folder_guard, paths, remove, session_guard,
};

/// The folder under a data folder whose subfolders are named after the accounts signed in.
const SESSIONS: &str = "claude-code-sessions";

/// The color of a profile whose account isn't in `config.toml` (a hand-edited entry).
const NEUTRAL_COLOR: &str = "#6b7280";

/// A desktop profile as the front ends show it (the UI, the tray and the CLI share it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopProfile {
    /// The default desktop profile: the app as it starts normally. It comes first.
    pub default: bool,
    /// The account it is bound to; for the default profile, the primary instance's account
    /// (`None` while the primary instance isn't registered with one).
    pub account: Option<AccountUuid>,
    /// The account's label, when the account is registered.
    pub label: Option<String>,
    /// The account's color; Claude's orange for the default profile, always.
    pub color: Color,
    /// Its data directory. The default profile's is the app's own, as found now.
    pub data_folder: PathBuf,
    /// The instance whose config directory its Code tab uses: the account's local instance,
    /// the primary instance for the default profile. `None` when there is none any more.
    pub instance: Option<PathBuf>,
    /// Whether it opens when mccboard starts; always `false` for the default profile.
    pub open_at_start: bool,
    pub observed: DesktopObservation,
    pub warnings: Vec<DesktopWarning>,
}

/// The account a desktop profile is signed in with, from its `claude-code-sessions` folders.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct DesktopObservation {
    /// The most recently used account; `None`: not signed in yet.
    pub account: Option<AccountUuid>,
    /// Other accounts signed in before in the same data folder, by UUID.
    pub others: Vec<AccountUuid>,
}

/// Something wrong with a desktop profile that the user should see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopWarning {
    /// The profile is signed in with another account than its own: a wrong sign-in.
    ObservedAccountDiffers {
        observed: AccountUuid,
        observed_label: Option<String>,
    },
    /// The default profile is signed in with another account than the primary instance's:
    /// its Code sessions would write that account into `~/.claude.json`
    /// (anthropics/claude-code#99240).
    DefaultProfileNotPrimaryAccount {
        observed: AccountUuid,
        observed_label: Option<String>,
    },
    /// The profile's account has no local instance any more, so its Code tab has none to use.
    NoLocalInstance,
}

/// Changes to a desktop profile. `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopProfileUpdate {
    pub open_at_start: Option<bool>,
}

/// What removing a desktop profile did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopProfileRemoval {
    pub account: AccountUuid,
    pub data_folder: PathBuf,
    pub folder: FolderOutcome,
}

/// A running process of the Claude desktop app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopProcess {
    pub pid: u32,
    /// The process name (`Claude.exe`).
    pub name: String,
    pub data_dir: DesktopDataDir,
}

/// The data directory a process of the desktop app uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopDataDir {
    /// Started without `--user-data-dir`: the app's own data directory (the default desktop
    /// profile).
    Default,
    /// Its `--user-data-dir`, absolute.
    Dir { path: PathBuf },
    /// Its command line can't be read (another user's process, an elevated one).
    Unknown,
}

pub(crate) fn list(
    home: &Path,
    config_file: &Path,
    env: &dyn Environment,
) -> Result<Vec<DesktopProfile>, Error> {
    let config = ConfigFile::load(config_file)?.config;
    Ok(views(home, &config, env))
}

/// The data folder a new profile for `account` gets by default: `%LOCALAPPDATA%\Claude-<label>`
/// on Windows; elsewhere, where the app doesn't run, `~/.config/Claude-<label>`.
pub(crate) fn default_folder(
    home: &Path,
    config_file: &Path,
    env: &dyn Environment,
    account: &AccountUuid,
) -> Result<PathBuf, Error> {
    let config = ConfigFile::load(config_file)?.config;
    let account = registered(&config, account)?;
    folder_for(home, env.os(), &account.label)
}

pub(crate) fn add(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
    account: &AccountUuid,
    data_folder: Option<&Path>,
) -> Result<DesktopProfile, Error> {
    let mut file = ConfigFile::load(&paths.config_file())?;
    let config = &file.config;
    let label = registered(config, account)?.label.clone();
    if config.default_profile_account(home) == Some(account) {
        return Err(Error::DefaultDesktopProfileAccount { label });
    }
    if config.desktop_profile(account).is_some() {
        return Err(Error::DesktopProfileExists { label });
    }
    if local_instance(home, config, account).is_none() {
        return Err(Error::NoLocalInstance { label });
    }
    let data_folder = match data_folder {
        Some(folder) => paths::resolve(home, folder)?,
        None => folder_for(home, env.os(), &label)?,
    };
    if data_folder.exists() && !data_folder.is_dir() {
        return Err(Error::NotADirectory { path: data_folder });
    }
    if !folder_allowed(home, paths, config, &data_folder, account) {
        return Err(Error::DesktopProfileFolderNotAllowed { path: data_folder });
    }

    file.add_desktop_profile(&DesktopProfileEntry {
        account: account.clone(),
        data_folder,
        open_at_start: false,
    });
    file.save()?;
    Ok(view_of(home, &file.config, env, account))
}

pub(crate) fn update(
    home: &Path,
    config_file: &Path,
    env: &dyn Environment,
    account: &AccountUuid,
    update: DesktopProfileUpdate,
) -> Result<DesktopProfile, Error> {
    let mut file = ConfigFile::load(config_file)?;
    stored(home, &file.config, account)?;
    if let Some(on) = update.open_at_start {
        file.set_desktop_profile_open_at_start(account, on);
    }
    file.save()?;
    Ok(view_of(home, &file.config, env, account))
}

/// Deleting the data folder follows the rules of removing an instance's folder: the folder
/// first and the entry after, so a deletion that stops half-way leaves the profile stored and
/// removing it again finishes; links inside are removed, never followed.
pub(crate) fn remove(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
    account: &AccountUuid,
    level: RemoveLevel,
) -> Result<DesktopProfileRemoval, Error> {
    let mut file = ConfigFile::load(&paths.config_file())?;
    let entry = stored(home, &file.config, account)?.clone();
    let data_folder = entry.data_folder;

    let folder = match level {
        RemoveLevel::FromMccboard => FolderOutcome::Kept,
        RemoveLevel::WithFolder => {
            if links::inspect(&data_folder) == Entry::Missing {
                FolderOutcome::AlreadyGone
            } else if !folder_allowed(home, paths, &file.config, &data_folder, account) {
                return Err(Error::FolderNotDeletable { path: data_folder });
            } else if !data_folder.is_dir() {
                return Err(Error::NotADirectory { path: data_folder });
            } else {
                let processes = processes_using(home, env, &data_folder);
                if !processes.is_empty() {
                    return Err(Error::DesktopProfileInUse {
                        path: data_folder,
                        processes,
                    });
                }
                remove::delete_folder(&data_folder).map_err(|source| Error::FolderNotDeleted {
                    path: data_folder.clone(),
                    source,
                })?;
                FolderOutcome::Deleted
            }
        }
    };

    file.remove_desktop_profile(account);
    file.save()?;
    // Leftover icons are harmless (a new color or profile gets new file names).
    let _ = IconCache::new(paths).remove(account);
    Ok(DesktopProfileRemoval {
        account: account.clone(),
        data_folder,
        folder,
    })
}

/// The desktop app's processes using the data folder `data_folder`, by pid. A process whose
/// command line can't be read doesn't count: which folder it uses can't be told.
pub(crate) fn processes_using(
    home: &Path,
    env: &dyn Environment,
    data_folder: &Path,
) -> Vec<DesktopProcess> {
    let mut using: Vec<DesktopProcess> = env
        .desktop_processes()
        .into_iter()
        .filter(|process| match &process.data_dir {
            DesktopDataDir::Dir { path } => session_guard::same_folder(path, data_folder),
            DesktopDataDir::Default => desktop_app::default_data_dirs(home)
                .iter()
                .any(|dir| session_guard::same_folder(dir, data_folder)),
            DesktopDataDir::Unknown => false,
        })
        .collect();
    using.sort_by_key(|p| p.pid);
    using
}

/// A registered account, or [`Error::AccountNotFound`].
fn registered<'a>(config: &'a Config, account: &AccountUuid) -> Result<&'a Account, Error> {
    config
        .account(account)
        .ok_or_else(|| Error::AccountNotFound {
            account: account.to_string(),
        })
}

/// The stored profile of `account`, or why there is none: the default profile is never
/// stored, nor changed.
fn stored<'a>(
    home: &Path,
    config: &'a Config,
    account: &AccountUuid,
) -> Result<&'a DesktopProfileEntry, Error> {
    if let Some(entry) = config.desktop_profile(account) {
        return Ok(entry);
    }
    if config.default_profile_account(home) == Some(account) {
        return Err(Error::DefaultDesktopProfile);
    }
    let label = registered(config, account)?.label.clone();
    Err(Error::DesktopProfileNotFound { label })
}

/// `~/AppData/Local/Claude-<label>` (Windows) or `~/.config/Claude-<label>`, the label kept
/// as typed but for characters a folder name can't safely hold.
fn folder_for(home: &Path, os: OsKind, label: &str) -> Result<PathBuf, Error> {
    let label = account::validate_label(label)?;
    let mut name = String::new();
    for c in label.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            name.push(c);
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    let name = name.trim_end_matches('-');
    if name.is_empty() {
        return Err(Error::NoDefaultFolder { label });
    }
    let parent = match os {
        OsKind::Windows => home.join("AppData").join("Local"),
        OsKind::Linux => home.join(".config"),
    };
    Ok(parent.join(format!("Claude-{name}")))
}

/// Whether `folder` may be the data folder of `account`'s profile: not protected
/// ([`folder_guard`]), and neither inside nor holding a registered instance, the app's own
/// data, or another profile's folder.
fn folder_allowed(
    home: &Path,
    paths: &Paths,
    config: &Config,
    folder: &Path,
    account: &AccountUuid,
) -> bool {
    let overlaps =
        |other: &Path| folder_guard::holds(other, folder) || folder_guard::holds(folder, other);
    !(folder_guard::is_protected(home, paths.root(), folder)
        || config.instances.iter().any(|i| overlaps(&i.config_dir))
        || config
            .desktop_profiles
            .iter()
            .filter(|p| &p.account != account)
            .any(|p| overlaps(&p.data_folder))
        || desktop_app::contains(home, &[], folder)
        || desktop_app::holds_app_data(home, folder))
}

/// The account's local instance: its first registered linked instance, by path.
fn local_instance(home: &Path, config: &Config, account: &AccountUuid) -> Option<PathBuf> {
    let mut dirs: Vec<&PathBuf> = config
        .instances
        .iter()
        .filter(|i| i.account.as_ref() == Some(account))
        .filter(|i| InstanceKind::of(home, &i.config_dir) == InstanceKind::Linked)
        .map(|i| &i.config_dir)
        .collect();
    dirs.sort_by_key(|dir| paths::key(dir));
    dirs.first().map(|dir| (*dir).clone())
}

/// Every desktop profile: the default one first, then the others by label.
fn views(home: &Path, config: &Config, env: &dyn Environment) -> Vec<DesktopProfile> {
    let accounts = config.shown_accounts(home);
    let label_of = |uuid: &AccountUuid| {
        accounts
            .iter()
            .find(|a| &a.uuid == uuid)
            .map(|a| a.label.clone())
    };

    let primary_account = config.default_profile_account(home).cloned();
    let observed = observe(&desktop_app::default_data_dirs(home));
    let mut warnings = Vec::new();
    if let (Some(seen), Some(primary)) = (&observed.account, &primary_account)
        && seen != primary
    {
        warnings.push(DesktopWarning::DefaultProfileNotPrimaryAccount {
            observed: seen.clone(),
            observed_label: label_of(seen),
        });
    }
    let default = DesktopProfile {
        default: true,
        label: primary_account.as_ref().and_then(label_of),
        account: primary_account,
        color: account::claude_orange(),
        data_folder: desktop_app::default_data_dir(home, env.os()),
        instance: config.primary_instance(home).map(|i| i.config_dir.clone()),
        open_at_start: false,
        observed,
        warnings,
    };

    let mut others: Vec<DesktopProfile> = config
        .desktop_profiles
        .iter()
        .map(|entry| {
            let account = accounts.iter().find(|a| a.uuid == entry.account);
            let instance = local_instance(home, config, &entry.account);
            let observed = observe(std::slice::from_ref(&entry.data_folder));
            let mut warnings = Vec::new();
            if let Some(seen) = &observed.account
                && seen != &entry.account
            {
                warnings.push(DesktopWarning::ObservedAccountDiffers {
                    observed: seen.clone(),
                    observed_label: label_of(seen),
                });
            }
            if instance.is_none() {
                warnings.push(DesktopWarning::NoLocalInstance);
            }
            DesktopProfile {
                default: false,
                account: Some(entry.account.clone()),
                label: account.map(|a| a.label.clone()),
                color: account.map_or_else(
                    || NEUTRAL_COLOR.parse().expect("a valid color"),
                    |a| a.color.clone(),
                ),
                data_folder: entry.data_folder.clone(),
                instance,
                open_at_start: entry.open_at_start,
                observed,
                warnings,
            }
        })
        .collect();
    others.sort_by(|a, b| {
        let key = |p: &DesktopProfile| {
            (
                p.label.as_ref().map(|l| l.to_lowercase()),
                p.account.clone(),
            )
        };
        key(a).cmp(&key(b))
    });

    let mut all = vec![default];
    all.extend(others);
    all
}

/// The view of `account`'s stored profile.
fn view_of(
    home: &Path,
    config: &Config,
    env: &dyn Environment,
    account: &AccountUuid,
) -> DesktopProfile {
    views(home, config, env)
        .into_iter()
        .find(|p| !p.default && p.account.as_ref() == Some(account))
        .expect("the profile is stored")
}

/// The accounts signed in across `data_folders`, from the names of the folders under each
/// `claude-code-sessions` and their modification times; the most recently modified wins.
/// Lists folders only: no file in a data folder is ever opened.
fn observe(data_folders: &[PathBuf]) -> DesktopObservation {
    let mut seen: BTreeMap<String, SystemTime> = BTreeMap::new();
    for folder in data_folders {
        let Ok(entries) = fs::read_dir(folder.join(SESSIONS)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_uuid(&name) || !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let modified = last_modified(&entry.path());
            let at = seen.entry(name).or_insert(modified);
            *at = (*at).max(modified);
        }
    }
    let latest = seen
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(uuid, _)| uuid.clone());
    let others = seen
        .keys()
        .filter(|uuid| Some(*uuid) != latest.as_ref())
        .map(AccountUuid::new)
        .collect();
    DesktopObservation {
        account: latest.map(AccountUuid::new),
        others,
    }
}

/// The latest modification time of an account's folder and of the folders right inside it
/// (one per organization), where the app adds sessions.
fn last_modified(account_dir: &Path) -> SystemTime {
    let modified = |path: &Path| {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    };
    let mut latest = modified(account_dir);
    if let Ok(entries) = fs::read_dir(account_dir) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                latest = latest.max(modified(&entry.path()));
            }
        }
    }
    latest
}

/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, hexadecimal.
fn is_uuid(name: &str) -> bool {
    name.len() == 36
        && name.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}
