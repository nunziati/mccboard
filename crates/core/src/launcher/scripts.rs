//! Windows launchers: `<name>.cmd` (cmd.exe) and `<name>.ps1` (PowerShell) in mccboard's own
//! bin folder, `~/.mccboard/bin`, which goes on the user PATH once.
//!
//! Both set `CLAUDE_CONFIG_DIR` for the one command and pass every argument through to
//! `claude`. Files in the folder that mccboard didn't write are left alone.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::Found;
use crate::Error;

/// Marks the scripts mccboard wrote; only those are ever removed.
const MARKER: &str = "Managed by mccboard";
const EXTENSIONS: [&str; 2] = ["cmd", "ps1"];

pub(super) struct BinFolder {
    path: PathBuf,
}

impl BinFolder {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The launchers in the folder: one per name with a `.cmd` or `.ps1` file.
    pub fn read(&self) -> Result<Vec<Found>, Error> {
        let unreadable = |source| Error::LauncherUnreadable {
            path: self.path.clone(),
            source,
        };
        let entries = match fs::read_dir(&self.path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(unreadable(e)),
        };
        // By name, ignoring case as Windows does: (name as found, cmd text, ps1 text).
        let mut by_name: BTreeMap<String, (String, Option<String>, Option<String>)> =
            BTreeMap::new();
        for entry in entries {
            let path = entry.map_err(unreadable)?.path();
            let (Some(stem), Some(extension)) = (
                path.file_stem().and_then(|s| s.to_str()),
                path.extension().and_then(|s| s.to_str()),
            ) else {
                continue;
            };
            let extension = extension.to_ascii_lowercase();
            if !EXTENSIONS.contains(&extension.as_str()) || !path.is_file() {
                continue;
            }
            let text = String::from_utf8_lossy(&fs::read(&path).map_err(unreadable)?).into_owned();
            let slot = by_name
                .entry(stem.to_lowercase())
                .or_insert_with(|| (stem.to_owned(), None, None));
            if extension == "cmd" {
                slot.1 = Some(text);
            } else {
                slot.2 = Some(text);
            }
        }
        Ok(by_name
            .into_values()
            .map(|(name, cmd, ps1)| {
                let config_dir = cmd
                    .as_deref()
                    .and_then(cmd_config_dir)
                    .or_else(|| ps1.as_deref().and_then(ps1_config_dir));
                let managed = [&cmd, &ps1]
                    .into_iter()
                    .flatten()
                    .any(|text| text.contains(MARKER));
                Found {
                    name,
                    config_dir,
                    content: joined(cmd.as_deref(), ps1.as_deref()),
                    managed,
                }
            })
            .collect())
    }

    /// Writes both scripts of a launcher.
    pub fn write(&self, name: &str, config_dir: &Path) -> Result<(), Error> {
        let unwritable = |source| Error::LauncherUnwritable {
            path: self.path.clone(),
            source,
        };
        fs::create_dir_all(&self.path).map_err(unwritable)?;
        fs::write(self.path.join(format!("{name}.cmd")), cmd(config_dir)).map_err(unwritable)?;
        fs::write(self.path.join(format!("{name}.ps1")), ps1(config_dir)).map_err(unwritable)?;
        Ok(())
    }

    /// Removes the scripts of a launcher that mccboard wrote.
    pub fn remove(&self, name: &str) -> Result<(), Error> {
        let unwritable = |source| Error::LauncherUnwritable {
            path: self.path.clone(),
            source,
        };
        let entries = fs::read_dir(&self.path).map_err(unwritable)?;
        for entry in entries {
            let path = entry.map_err(unwritable)?.path();
            let ours = path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(name))
                && path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
                && fs::read(&path)
                    .is_ok_and(|bytes| String::from_utf8_lossy(&bytes).contains(MARKER));
            if ours {
                fs::remove_file(&path).map_err(unwritable)?;
            }
        }
        Ok(())
    }
}

/// Both scripts as [`Found::content`] holds them, to compare with what is on disk.
pub(super) fn content(config_dir: &Path) -> String {
    joined(Some(&cmd(config_dir)), Some(&ps1(config_dir)))
}

fn joined(cmd: Option<&str>, ps1: Option<&str>) -> String {
    format!("{}\0{}", cmd.unwrap_or_default(), ps1.unwrap_or_default())
}

/// The cmd.exe script. `setlocal` keeps the variable from leaking into the calling shell.
fn cmd(config_dir: &Path) -> String {
    let dir = config_dir.to_string_lossy().replace('%', "%%");
    [
        "@echo off",
        &format!(
            "rem {MARKER}: starts Claude Code in the instance below. Changes here are overwritten."
        ),
        "setlocal",
        &format!("set \"CLAUDE_CONFIG_DIR={dir}\""),
        "claude %*",
    ]
    .map(|line| format!("{line}\r\n"))
    .concat()
}

/// The PowerShell script, UTF-8 with a byte order mark so Windows PowerShell 5 reads
/// non-ASCII paths right. It restores the variable after `claude` exits.
fn ps1(config_dir: &Path) -> String {
    let dir = config_dir.to_string_lossy().replace('\'', "''");
    let lines = [
        &format!(
            "# {MARKER}: starts Claude Code in the instance below. Changes here are overwritten."
        ) as &str,
        "$previous = $env:CLAUDE_CONFIG_DIR",
        &format!("$env:CLAUDE_CONFIG_DIR = '{dir}'"),
        "try {",
        "    & claude @args",
        "} finally {",
        "    $env:CLAUDE_CONFIG_DIR = $previous",
        "}",
        "exit $LASTEXITCODE",
    ];
    format!(
        "\u{feff}{}",
        lines.map(|line| format!("{line}\r\n")).concat()
    )
}

fn cmd_config_dir(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("set \"CLAUDE_CONFIG_DIR=")?
            .strip_suffix('"')
            .map(|dir| PathBuf::from(dir.replace("%%", "%")))
    })
}

fn ps1_config_dir(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("$env:CLAUDE_CONFIG_DIR = '")?
            .strip_suffix('\'')
            .map(|dir| PathBuf::from(dir.replace("''", "'")))
    })
}
