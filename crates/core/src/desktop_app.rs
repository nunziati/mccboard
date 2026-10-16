//! The Claude desktop app's data folders, where mccboard never creates links: the MSIX app
//! virtualises writes there and links break it (spec story 50). Desktop profiles' data
//! folders are in the same set (spec #34, story 6).
//!
//! Windows: `%APPDATA%\Claude` (older installs, and the MSIX app's virtualised view),
//! `%LOCALAPPDATA%\AnthropicClaude`, and the MSIX package folders
//! `%LOCALAPPDATA%\Packages\Claude_*`. The default `AppData` locations under the home are
//! assumed; the same names are checked on every OS, which costs nothing elsewhere.

use std::fs;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

use crate::{OsKind, folder_guard, paths};

/// Data folders, relative to the home, `/`-separated.
const DATA_DIRS: &[&str] = &[
    "AppData/Roaming/Claude",
    "AppData/Local/AnthropicClaude",
    ".config/Claude",
    "Library/Application Support/Claude",
];

/// Where Windows keeps MSIX package data, relative to the home.
const PACKAGES: &str = "AppData/Local/Packages";

/// Package folder name prefixes of the Claude desktop app (lowercase).
const PACKAGE_PREFIXES: &[&str] = &["claude_", "anthropic.claude"];

/// Where the MSIX app's view of `%APPDATA%\Claude` lands inside its package folder.
const PACKAGE_DATA_DIR: &str = "LocalCache/Roaming/Claude";

/// Whether `path` is in (or is) one of the Claude desktop app's data folders, or one of the
/// desktop profiles' `profile_folders`. Checks the path as given and with links resolved.
pub(crate) fn contains(home: &Path, profile_folders: &[PathBuf], path: &Path) -> bool {
    let homes = spellings(home);
    spellings(path)
        .iter()
        .any(|path| homes.iter().any(|home| inside(home, path)))
        || profile_folders
            .iter()
            .any(|folder| folder_guard::holds(folder, path))
}

/// Whether `path` holds one of the app's own data folders, or the folder of every MSIX
/// package's data: deleting it would delete the app's data.
pub(crate) fn holds_app_data(home: &Path, path: &Path) -> bool {
    DATA_DIRS
        .iter()
        .chain(std::iter::once(&PACKAGES))
        .any(|dir| folder_guard::holds(path, &at(home, dir)))
}

/// The folders the app uses as its data directory when started normally (the default desktop
/// profile's), as found now: `%APPDATA%\Claude` and its virtualised copy in each of the app's
/// package folders, plus `~/.config/Claude` elsewhere than Windows. Lists the packages folder;
/// reads nothing inside any of them.
pub(crate) fn default_data_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![at(home, DATA_DIRS[0])];
    if let Ok(entries) = fs::read_dir(at(home, PACKAGES)) {
        let mut packages: Vec<PathBuf> = entries
            .flatten()
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                PACKAGE_PREFIXES.iter().any(|p| name.starts_with(p))
            })
            .map(|entry| {
                let mut dir = entry.path();
                dir.extend(PACKAGE_DATA_DIR.split('/'));
                dir
            })
            .collect();
        packages.sort();
        dirs.extend(packages);
    }
    dirs.push(at(home, ".config/Claude"));
    dirs
}

/// The default desktop profile's data folder as shown: the first of
/// [`default_data_dirs`] that exists, else `%APPDATA%\Claude` on Windows and
/// `~/.config/Claude` elsewhere.
pub(crate) fn default_data_dir(home: &Path, os: OsKind) -> PathBuf {
    default_data_dirs(home)
        .into_iter()
        .find(|dir| dir.is_dir())
        .unwrap_or_else(|| match os {
            OsKind::Windows => at(home, DATA_DIRS[0]),
            OsKind::Linux => at(home, ".config/Claude"),
        })
}

fn at(home: &Path, relative: &str) -> PathBuf {
    let mut dir = home.to_path_buf();
    dir.extend(relative.split('/'));
    dir
}

fn spellings(path: &Path) -> Vec<PathBuf> {
    let mut out = vec![paths::normalize(path)];
    if let Ok(real) = fs::canonicalize(path) {
        out.push(paths::normalize(&real));
    }
    out
}

fn inside(home: &Path, path: &Path) -> bool {
    if DATA_DIRS
        .iter()
        .any(|dir| under(path, &at(home, dir)).is_some())
    {
        return true;
    }
    under(path, &at(home, PACKAGES))
        .and_then(|rest| rest.split(MAIN_SEPARATOR).next().map(str::to_owned))
        .is_some_and(|package| {
            let package = package.to_lowercase();
            PACKAGE_PREFIXES.iter().any(|p| package.starts_with(p))
        })
}

/// When `path` is `dir` or inside it, what follows `dir` in it (compared as [`paths::key`]).
fn under(path: &Path, dir: &Path) -> Option<String> {
    let (path, dir) = (paths::key(path), paths::key(dir));
    if path == dir {
        return Some(String::new());
    }
    path.strip_prefix(&format!("{dir}{MAIN_SEPARATOR}"))
        .map(str::to_owned)
}
