//! Linux launchers: one small `sh` script per launcher in `~/.local/bin`, the folder where the
//! official installer of Claude Code puts `claude`. A script works in every shell (bash, zsh,
//! fish, nushell…) and in every program that runs commands from the PATH. An alias in the rc
//! file of one shell does not (ADR 0011).
//!
//! ```sh
//! #!/bin/sh
//! # mccboard launcher: overwritten by mccboard. Rename it with `mccboard account set`.
//! export CLAUDE_CONFIG_DIR=/home/me/.claude-b
//! exec claude "$@"
//! ```
//!
//! mccboard reads, rewrites and removes only the scripts that carry its marker line. A file of
//! the user's with the name of a launcher is never touched: writing that launcher fails instead.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use super::{Found, shell};
use crate::Error;

const SHEBANG: &str = "#!/bin/sh";
const MARKER: &str =
    "# mccboard launcher: overwritten by mccboard. Rename it with `mccboard account set`.";
const EXPORT: &str = "export CLAUDE_CONFIG_DIR=";

/// The most a launcher script can hold: longer files are not mccboard's.
const MAX_LEN: u64 = 4096;

pub(super) struct ScriptFolder {
    path: PathBuf,
}

impl ScriptFolder {
    /// `~/.local/bin` of `home`.
    pub fn of(home: &Path) -> Self {
        Self {
            path: home.join(".local").join("bin"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The launchers in the folder. A file mccboard did not write comes back unmanaged and
    /// without content: a wanted name it holds then reads as stale and never as an orphan.
    pub fn read(&self) -> Result<Vec<Found>, Error> {
        let entries = match fs::read_dir(&self.path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(Error::LauncherUnreadable {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let path = entry.path();
            if path.is_dir() {
                continue;
            }
            found.push(match managed_text(&path) {
                Some(text) => Found {
                    name,
                    config_dir: config_dir_of(&text),
                    content: text,
                    managed: true,
                },
                None => Found {
                    name,
                    config_dir: None,
                    content: String::new(),
                    managed: false,
                },
            });
        }
        found.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(found)
    }

    /// Writes the launcher `name`, creating the folder when needed. Refuses a file of the
    /// user's with that name.
    pub fn write(&self, name: &str, config_dir: &Path) -> Result<(), Error> {
        let target = self.path.join(name);
        if fs::symlink_metadata(&target).is_ok() && managed_text(&target).is_none() {
            return Err(Error::LauncherNameTaken { path: target });
        }
        let unwritable = |source| Error::LauncherUnwritable {
            path: self.path.clone(),
            source,
        };
        fs::create_dir_all(&self.path).map_err(unwritable)?;
        let temporary = self
            .path
            .join(format!(".{name}.mccboard-{}.tmp", std::process::id()));
        let result = fs::write(&temporary, content(config_dir))
            .and_then(|()| make_executable(&temporary))
            .and_then(|()| fs::rename(&temporary, &target));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(unwritable)
    }

    /// Removes the launcher `name` when mccboard wrote it.
    pub fn remove(&self, name: &str) -> Result<(), Error> {
        let target = self.path.join(name);
        if managed_text(&target).is_none() {
            return Ok(());
        }
        match fs::remove_file(&target) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::LauncherUnwritable {
                path: self.path.clone(),
                source,
            }),
        }
    }
}

/// The script mccboard writes for a launcher of `config_dir`. It passes every argument
/// through to `claude`.
pub(super) fn content(config_dir: &Path) -> String {
    let dir = shell::quote_if_needed(&config_dir.to_string_lossy());
    format!("{SHEBANG}\n{MARKER}\n{EXPORT}{dir}\nexec claude \"$@\"\n")
}

/// The text of `path` when it is a launcher mccboard wrote.
fn managed_text(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_LEN + 1).read_to_string(&mut text).ok()?;
    if text.len() as u64 > MAX_LEN {
        return None;
    }
    let mut lines = text.lines();
    (lines.next() == Some(SHEBANG) && lines.next() == Some(MARKER)).then_some(text)
}

/// The config directory a launcher script exports, if it exports one.
fn config_dir_of(text: &str) -> Option<PathBuf> {
    text.lines()
        .find_map(|line| line.strip_prefix(EXPORT))
        .and_then(shell::words)
        .and_then(|words| words.into_iter().next())
        .map(PathBuf::from)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}
