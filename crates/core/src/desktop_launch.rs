//! Finding and starting the Claude desktop app (spec #34, ticket #36, ADR 0008).
//!
//! - **The app** is the MSIX package of family [`PACKAGE_FAMILY`]; families of other
//!   packages named `Claude` are found from their data folders under
//!   `%LOCALAPPDATA%\Packages`. The environment port asks Windows' packaging API for their
//!   install folders; the core picks the newest version whose `app\Claude.exe` is there. It
//!   is looked up at **every** launch, because the folder changes with each update.
//! - **The launch** the core asks the port for ([`DesktopLaunch`]): the program, its
//!   arguments, the variables to set and the prefixes of the inherited ones to remove, and no
//!   console window. The port starts it and doesn't wait.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{Environment, Error, OsKind};

/// The package family of the Claude desktop app from the Microsoft Store and claude.ai.
pub const PACKAGE_FAMILY: &str = "Claude_pzs8sxrjxfjjc";

/// The variable that tells a profile's Code tab which instance to use.
pub(crate) const CONFIG_DIR_VAR: &str = "CLAUDE_CONFIG_DIR";

/// Inherited variables starting with these (ASCII case ignored) are removed from the app's
/// environment: mccboard's own `CLAUDE_CONFIG_DIR`, a token, an API key would otherwise
/// reach the profile's Code sessions.
pub(crate) const STRIPPED_PREFIXES: &[&str] = &["CLAUDE", "ANTHROPIC"];

/// Where Windows keeps each package's data, one folder per package family, under the home.
const PACKAGES: &[&str] = &["AppData", "Local", "Packages"];

/// The old Squirrel install of the app, under the home: out of scope (ADR 0008).
const SQUIRREL: &[&str] = &["AppData", "Local", "AnthropicClaude"];

/// An installed MSIX package of the desktop app, as the environment port finds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopPackage {
    /// The package full name (`Claude_1.0.2342.0_x64__pzs8sxrjxfjjc`).
    pub full_name: String,
    /// Its install folder (`C:\Program Files\WindowsApps\Claude_…`).
    pub install_dir: PathBuf,
}

/// The desktop app to start, as the core asks the environment port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopLaunch {
    /// `<install folder>\app\Claude.exe`.
    pub program: PathBuf,
    /// `--user-data-dir=<data folder>` for a profile; none for the default desktop profile.
    pub args: Vec<String>,
    /// Variables to set in its environment: `CLAUDE_CONFIG_DIR` for a profile.
    pub set_env: Vec<(String, PathBuf)>,
    /// Inherited variables whose names start with one of these, ASCII case ignored, are
    /// removed before `set_env` applies.
    pub remove_env_prefixes: Vec<String>,
    /// Whether a console window may show: never for the desktop app.
    pub console_window: bool,
}

/// `<install folder>\app\Claude.exe` of the newest installed package of the desktop app.
pub(crate) fn find_app(home: &Path, env: &dyn Environment) -> Result<PathBuf, Error> {
    if env.os() != OsKind::Windows {
        return Err(Error::DesktopAppNeedsWindows);
    }
    let packages =
        env.desktop_app_packages(&families(home))
            .map_err(|e| Error::DesktopAppNotOpened {
                reason: e.to_string(),
            })?;
    if let Some(program) = newest_program(&packages) {
        return Ok(program);
    }
    let squirrel = under(home, SQUIRREL);
    if squirrel.is_dir() {
        return Err(Error::DesktopAppNotMsix { path: squirrel });
    }
    Err(Error::DesktopAppNotInstalled)
}

/// The launch of the app `program` with data folder `data_folder` (none: the default desktop
/// profile) and its Code tab in `config_dir`.
pub(crate) fn launch(
    program: PathBuf,
    data_folder: Option<&Path>,
    config_dir: Option<&Path>,
) -> DesktopLaunch {
    DesktopLaunch {
        program,
        args: data_folder
            .map(|folder| format!("--user-data-dir={}", folder.display()))
            .into_iter()
            .collect(),
        set_env: config_dir
            .map(|dir| (CONFIG_DIR_VAR.to_owned(), dir.to_path_buf()))
            .into_iter()
            .collect(),
        remove_env_prefixes: STRIPPED_PREFIXES.iter().map(|p| (*p).to_owned()).collect(),
        console_window: false,
    }
}

/// The package families to look up: the app's own, then any other whose data folder under
/// `%LOCALAPPDATA%\Packages` is named `Claude_…`.
fn families(home: &Path) -> Vec<String> {
    const PREFIX: &str = "claude_";
    let mut families = vec![PACKAGE_FAMILY.to_owned()];
    if let Ok(entries) = fs::read_dir(under(home, PACKAGES)) {
        let mut found: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| {
                name.len() > PREFIX.len()
                    && name
                        .get(..PREFIX.len())
                        .is_some_and(|p| p.eq_ignore_ascii_case(PREFIX))
            })
            .filter(|name| !families.iter().any(|f| f.eq_ignore_ascii_case(name)))
            .collect();
        found.sort();
        families.extend(found);
    }
    families
}

/// The program of the newest package whose `app\Claude.exe` is there.
fn newest_program(packages: &[DesktopPackage]) -> Option<PathBuf> {
    packages
        .iter()
        .map(|p| {
            (
                version(&p.full_name),
                p.install_dir.join("app").join("Claude.exe"),
            )
        })
        .filter(|(_, program)| is_there(program))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, program)| program)
}

/// The version in a package full name (`Name_1.0.2342.0_x64__publisher`), compared part by
/// part; an unreadable one sorts first.
fn version(full_name: &str) -> Vec<u32> {
    full_name
        .split('_')
        .nth(1)
        .and_then(|v| v.split('.').map(|n| n.parse().ok()).collect())
        .unwrap_or_default()
}

/// Whether `program` is a file. Files in `WindowsApps` can be read by path; a refusal still
/// counts as there, and launching tells.
fn is_there(program: &Path) -> bool {
    match fs::metadata(program) {
        Ok(meta) => meta.is_file(),
        Err(e) => e.kind() == io::ErrorKind::PermissionDenied,
    }
}

fn under(home: &Path, parts: &[&str]) -> PathBuf {
    let mut path = home.to_path_buf();
    path.extend(parts);
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_read_from_the_full_name() {
        assert_eq!(
            version("Claude_1.0.2342.0_x64__pzs8sxrjxfjjc"),
            [1, 0, 2342, 0]
        );
        assert!(version("Claude_1.0.10.0_x64__p") > version("Claude_1.0.9.0_x64__p"));
        assert_eq!(version("Claude"), Vec::<u32>::new());
        assert_eq!(version("Claude_x.y_x64__p"), Vec::<u32>::new());
    }

    #[test]
    fn the_launch_of_a_profile_and_of_the_default_one() {
        let profile = launch(
            PathBuf::from("/app/Claude.exe"),
            Some(Path::new("/p/b")),
            Some(Path::new("/h/.claude-b")),
        );
        assert_eq!(profile.args, ["--user-data-dir=/p/b"]);
        assert_eq!(
            profile.set_env,
            [(
                "CLAUDE_CONFIG_DIR".to_owned(),
                PathBuf::from("/h/.claude-b")
            )]
        );
        assert_eq!(profile.remove_env_prefixes, ["CLAUDE", "ANTHROPIC"]);
        assert!(!profile.console_window);

        let default = launch(PathBuf::from("/app/Claude.exe"), None, None);
        assert!(default.args.is_empty());
        assert!(default.set_env.is_empty());
        assert_eq!(default.remove_env_prefixes, ["CLAUDE", "ANTHROPIC"]);
    }
}
