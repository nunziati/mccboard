//! Removing an instance: from mccboard only (unregister it, which removes its launcher), or
//! with its folder too.
//!
//! The primary instance is never removed at either level. Deleting a folder:
//! - refuses a folder that holds the home, the primary instance or mccboard's own files, one
//!   inside the primary instance, and an instance folder that is itself a link (its target is
//!   never followed);
//! - passes the session guard first: on Windows, deleting a folder under a running
//!   `claude.exe` pins a core at 100% (anthropics/claude-code#101027);
//! - removes every link inside as a link, never following it, so a linked instance's links
//!   leave the primary instance's real items intact.
//!
//! The folder goes first and the registration after, so a deletion that stops half-way leaves
//! the instance registered, and removing it again finishes the job.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::ConfigFile;
use crate::instance::InstanceKind;
use crate::links::{self, Entry};
use crate::{Environment, Error, LauncherSync, Paths, paths, registry, session_guard};

/// How far removing an instance goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoveLevel {
    /// mccboard forgets the instance and removes its launcher; the folder stays as it is.
    FromMccboard,
    /// Also deletes the instance's folder, and with it the account's login on this machine.
    /// Front ends ask the user to confirm this explicitly first.
    WithFolder,
}

/// What removing an instance did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Removal {
    /// The instance's config directory, as it was registered.
    pub config_dir: PathBuf,
    pub folder: FolderOutcome,
}

/// What happened to the instance's folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderOutcome {
    /// Left as it was ([`RemoveLevel::FromMccboard`]).
    Kept,
    /// Deleted, links inside removed without following them.
    Deleted,
    /// It no longer existed.
    AlreadyGone,
}

pub(crate) fn remove(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
    path: &Path,
    level: RemoveLevel,
    sync_launchers: &dyn Fn() -> Result<LauncherSync, Error>,
) -> Result<Removal, Error> {
    let config_dir = paths::resolve(home, path)?;
    let primary = home.join(".claude");
    if InstanceKind::of(home, &config_dir) == InstanceKind::Primary {
        return Err(Error::PrimaryInstance { path: primary });
    }
    let config_file = paths.config_file();
    // The instance keeps the spelling of its path in config.toml.
    let config_dir = ConfigFile::load(&config_file)?
        .config
        .instance(&config_dir)
        .map(|entry| entry.config_dir.clone())
        .ok_or(Error::NotRegistered { path: config_dir })?;

    let folder = match level {
        RemoveLevel::FromMccboard => FolderOutcome::Kept,
        RemoveLevel::WithFolder => match links::inspect(&config_dir) {
            Entry::Missing => FolderOutcome::AlreadyGone,
            Entry::Link { .. } => return Err(Error::FolderNotDeletable { path: config_dir }),
            Entry::Real => {
                if !config_dir.is_dir() {
                    return Err(Error::NotADirectory { path: config_dir });
                }
                let protected = [home, primary.as_path(), paths.root()];
                if protected.iter().any(|p| holds(&config_dir, p)) || holds(&primary, &config_dir) {
                    return Err(Error::FolderNotDeletable { path: config_dir });
                }
                session_guard::ensure_idle(home, env, &config_dir)?;
                delete_folder(&config_dir).map_err(|source| Error::FolderNotDeleted {
                    path: config_dir.clone(),
                    source,
                })?;
                FolderOutcome::Deleted
            }
        },
    };

    registry::unregister(home, &config_file, &config_dir)?;
    sync_launchers()?;
    Ok(Removal { config_dir, folder })
}

/// Whether `inner` is `outer` or somewhere inside it, by spelling or once links are resolved.
fn holds(outer: &Path, inner: &Path) -> bool {
    let inside = |outer: &Path, inner: &Path| {
        Path::new(&paths::key(inner)).starts_with(Path::new(&paths::key(outer)))
    };
    if inside(outer, inner) {
        return true;
    }
    match (fs::canonicalize(outer), fs::canonicalize(inner)) {
        (Ok(outer), Ok(inner)) => inside(&paths::normalize(&outer), &paths::normalize(&inner)),
        _ => false,
    }
}

/// Deletes a real folder and everything in it. Links (symlinks, junctions) are removed as
/// links: what they point to is never touched.
fn delete_folder(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        match links::inspect(&path) {
            Entry::Missing => {}
            Entry::Link { .. } => links::remove_link(&path)?,
            Entry::Real if fs::symlink_metadata(&path)?.is_dir() => delete_folder(&path)?,
            Entry::Real => remove_file(&path)?,
        }
    }
    fs::remove_dir(dir)
}

fn remove_file(path: &Path) -> io::Result<()> {
    let removed = fs::remove_file(path);
    #[cfg(windows)]
    if let Err(error) = &removed {
        // Windows refuses to delete a read-only file (git objects in plugin marketplaces).
        let mut permissions = fs::symlink_metadata(path)?.permissions();
        if error.kind() == io::ErrorKind::PermissionDenied && permissions.readonly() {
            // Only on Windows, where this clears the read-only attribute and nothing else.
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
            return fs::remove_file(path);
        }
    }
    removed
}
