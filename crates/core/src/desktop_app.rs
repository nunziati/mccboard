//! The Claude desktop app's data folders, where mccboard never creates links: the MSIX app
//! virtualises writes there and links break it (spec story 50).
//!
//! Windows: `%APPDATA%\Claude` (older installs, and the MSIX app's virtualised view),
//! `%LOCALAPPDATA%\AnthropicClaude`, and the MSIX package folders
//! `%LOCALAPPDATA%\Packages\Claude_*`. The default `AppData` locations under the home are
//! assumed; the same names are checked on every OS, which costs nothing elsewhere.

use std::fs;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

use crate::paths;

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

/// Whether `path` is in (or is) one of the Claude desktop app's data folders. Checks the path
/// as given and with links resolved.
pub(crate) fn contains(home: &Path, path: &Path) -> bool {
    let homes = spellings(home);
    spellings(path)
        .iter()
        .any(|path| homes.iter().any(|home| inside(home, path)))
}

fn spellings(path: &Path) -> Vec<PathBuf> {
    let mut out = vec![paths::normalize(path)];
    if let Ok(real) = fs::canonicalize(path) {
        out.push(paths::normalize(&real));
    }
    out
}

fn inside(home: &Path, path: &Path) -> bool {
    let at = |relative: &str| {
        let mut dir = home.to_path_buf();
        dir.extend(relative.split('/'));
        dir
    };
    if DATA_DIRS.iter().any(|dir| under(path, &at(dir)).is_some()) {
        return true;
    }
    under(path, &at(PACKAGES))
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
