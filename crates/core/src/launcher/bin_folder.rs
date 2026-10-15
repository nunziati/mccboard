//! Windows launchers, in mccboard's own bin folder, `~/.mccboard/bin`, which goes on the user
//! PATH once. A launcher is a copy of mccboard's shim, `<name>.exe`, next to its sidecar
//! `<name>.shim`, which names the instance (the `mccboard-shim` crate; ADR 0007). A program,
//! not a script: cmd.exe and PowerShell run it whatever the execution policy, and arguments
//! reach `claude` without a shell re-quoting them.
//!
//! Earlier versions wrote `<name>.cmd` and `<name>.ps1` scripts. Those mccboard wrote (they
//! carry its marker) make a launcher stale, and a sync removes them. Files in the folder that
//! mccboard didn't write are left alone.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use mccboard_shim::sidecar;

use super::Found;
use crate::Error;

const PROGRAM: &str = "exe";
/// The scripts earlier versions wrote, and the marker they carry.
const LEGACY: [&str; 2] = ["cmd", "ps1"];
const LEGACY_MARKER: &str = "Managed by mccboard";
/// A launcher program moved aside because it was running when it had to go
/// ([`remove_program`]): `claude-b.exe.old`, `claude-b.exe.old-2`… A later sync deletes it.
const ASIDE: &str = ".exe.old";

pub(super) struct BinFolder {
    path: PathBuf,
    /// The shim mccboard ships, or why it can't be read.
    shim: io::Result<Vec<u8>>,
}

/// What a name has in the folder.
#[derive(Default)]
struct Files {
    /// As found, the first time.
    name: String,
    program: Option<Program>,
    sidecar: Option<String>,
    /// The text of each `.cmd` and `.ps1` script.
    legacy: Vec<String>,
}

/// A `<name>.exe`, against the shim mccboard ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Program {
    Shim,
    Other,
    /// The shim can't be read, so no telling: taken as the shim, never as mccboard's to
    /// remove.
    Unknown,
}

impl BinFolder {
    pub fn new(path: PathBuf, shim: io::Result<Vec<u8>>) -> Self {
        Self { path, shim }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The launchers in the folder: one per name with a program, a sidecar or a script.
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
        // By name, ignoring case as Windows does.
        let mut by_name: BTreeMap<String, Files> = BTreeMap::new();
        for entry in entries {
            let path = entry.map_err(unreadable)?.path();
            let (Some(stem), Some(extension)) = (
                path.file_stem().and_then(|s| s.to_str()),
                path.extension().and_then(|s| s.to_str()),
            ) else {
                continue;
            };
            let extension = extension.to_ascii_lowercase();
            let known = extension == PROGRAM
                || extension == sidecar::EXTENSION
                || LEGACY.contains(&extension.as_str());
            if !known || !path.is_file() {
                continue;
            }
            let files = by_name.entry(stem.to_lowercase()).or_insert_with(|| Files {
                name: stem.to_owned(),
                ..Files::default()
            });
            if extension == PROGRAM {
                files.program = Some(self.program(&path).map_err(unreadable)?);
            } else {
                let text =
                    String::from_utf8_lossy(&fs::read(&path).map_err(unreadable)?).into_owned();
                if extension == sidecar::EXTENSION {
                    files.sidecar = Some(text);
                } else {
                    files.legacy.push(text);
                }
            }
        }
        Ok(by_name.into_values().map(found).collect())
    }

    /// Writes a launcher: its sidecar, then its program unless it is the shim already, and
    /// removes the scripts an earlier version wrote for it.
    pub fn write(&self, name: &str, config_dir: &Path) -> Result<(), Error> {
        let unwritable = |source| Error::LauncherUnwritable {
            path: self.path.clone(),
            source,
        };
        let program = self.file(name, PROGRAM);
        let shim = match &self.shim {
            Ok(shim) => Some(shim),
            // A program already there may do: only the sidecar can be checked.
            Err(_) if program.is_file() => None,
            Err(e) => {
                return Err(Error::LauncherShimMissing {
                    source: io::Error::new(e.kind(), e.to_string()),
                });
            }
        };
        fs::create_dir_all(&self.path).map_err(unwritable)?;
        let sidecar_path = self.file(name, sidecar::EXTENSION);
        let text = sidecar::render(config_dir);
        if fs::read_to_string(&sidecar_path).ok().as_deref() != Some(text.as_str()) {
            fs::write(&sidecar_path, text).map_err(unwritable)?;
        }
        if let Some(shim) = shim
            && !same_bytes(&program, shim).map_err(unwritable)?
        {
            remove_program(&program).map_err(unwritable)?;
            fs::write(&program, shim).map_err(unwritable)?;
        }
        self.remove_legacy(name).map_err(unwritable)
    }

    /// Removes a launcher mccboard wrote: its program when its sidecar is mccboard's or it is
    /// the shim, its sidecar when mccboard's, and an earlier version's scripts.
    pub fn remove(&self, name: &str) -> Result<(), Error> {
        let unwritable = |source| Error::LauncherUnwritable {
            path: self.path.clone(),
            source,
        };
        let sidecar_path = self.file(name, sidecar::EXTENSION);
        let sidecar_ours =
            fs::read_to_string(&sidecar_path).is_ok_and(|text| sidecar::is_managed(&text));
        let program = self.file(name, PROGRAM);
        let program_ours = sidecar_ours
            || (program.is_file() && self.program(&program).map_err(unwritable)? == Program::Shim);
        if program_ours {
            remove_program(&program).map_err(unwritable)?;
        }
        if sidecar_ours {
            fs::remove_file(&sidecar_path).map_err(unwritable)?;
        }
        self.remove_legacy(name).map_err(unwritable)
    }

    /// Deletes the programs moved aside by earlier syncs, now that they may have exited.
    /// Those still running stay for the next time.
    pub fn sweep(&self) {
        let Ok(entries) = fs::read_dir(&self.path) else {
            return;
        };
        for entry in entries.flatten() {
            let aside = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.to_ascii_lowercase().contains(ASIDE));
            if aside {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn file(&self, name: &str, extension: &str) -> PathBuf {
        self.path.join(format!("{name}.{extension}"))
    }

    fn program(&self, path: &Path) -> io::Result<Program> {
        Ok(match &self.shim {
            Ok(shim) if same_bytes(path, shim)? => Program::Shim,
            Ok(_) => Program::Other,
            Err(_) => Program::Unknown,
        })
    }

    /// Removes the `.cmd` and `.ps1` scripts of `name` that carry mccboard's marker.
    fn remove_legacy(&self, name: &str) -> io::Result<()> {
        let entries = match fs::read_dir(&self.path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let path = entry?.path();
            let ours = path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(name))
                && path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|ext| LEGACY.contains(&ext.to_ascii_lowercase().as_str()))
                && fs::read(&path)
                    .is_ok_and(|bytes| String::from_utf8_lossy(&bytes).contains(LEGACY_MARKER));
            if ours {
                fs::remove_file(&path)?;
            }
        }
        Ok(())
    }
}

/// A launcher as [`Found::content`] holds it, when mccboard writes it for `config_dir`.
pub(super) fn content(config_dir: &Path) -> String {
    joined(
        Some(Program::Shim),
        Some(&sidecar::render(config_dir)),
        false,
    )
}

/// What decides whether a launcher is as mccboard writes it: whether its program is the shim,
/// its sidecar's text, and whether an earlier version's scripts are still there.
fn joined(program: Option<Program>, sidecar: Option<&str>, legacy: bool) -> String {
    let program = match program {
        None => "none",
        Some(Program::Shim | Program::Unknown) => "shim",
        Some(Program::Other) => "other",
    };
    format!(
        "{program}\0{}\0legacy={legacy}",
        sidecar.unwrap_or_default()
    )
}

fn found(files: Files) -> Found {
    let legacy_ours = files.legacy.iter().any(|text| text.contains(LEGACY_MARKER));
    let sidecar_ours = files.sidecar.as_deref().is_some_and(sidecar::is_managed);
    Found {
        config_dir: files
            .sidecar
            .as_deref()
            .and_then(sidecar::parse)
            .or_else(|| files.legacy.iter().find_map(|text| legacy_config_dir(text))),
        content: joined(files.program, files.sidecar.as_deref(), legacy_ours),
        managed: sidecar_ours || legacy_ours || files.program == Some(Program::Shim),
        name: files.name,
    }
}

/// Whether the file at `path` holds exactly `bytes`; a missing file doesn't.
fn same_bytes(path: &Path, bytes: &[u8]) -> io::Result<bool> {
    match fs::metadata(path) {
        Ok(meta) if meta.len() != bytes.len() as u64 => Ok(false),
        Ok(_) => Ok(fs::read(path)? == bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Removes a launcher program, which may be running right now: Windows doesn't let a running
/// program be deleted or overwritten, but lets it be renamed. Then it is moved aside, out of
/// the PATH's reach (`claude-b.exe.old`), for a later sync to delete ([`BinFolder::sweep`]).
fn remove_program(path: &Path) -> io::Result<()> {
    let error = match fs::remove_file(path) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => e,
    };
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let aside = (1..)
        .map(|n| match n {
            1 => path.with_file_name(format!("{file_name}.old")),
            n => path.with_file_name(format!("{file_name}.old-{n}")),
        })
        .find(|aside| !aside.exists())
        .expect("an endless sequence has a free name");
    fs::rename(path, aside).map_err(|_| error)
}

/// The instance an earlier version's script starts Claude Code in.
fn legacy_config_dir(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let line = line.trim();
        if let Some(dir) = line.strip_prefix("set \"CLAUDE_CONFIG_DIR=") {
            // cmd.exe: `set "CLAUDE_CONFIG_DIR=<dir>"`, `%` doubled.
            return dir
                .strip_suffix('"')
                .map(|dir| PathBuf::from(dir.replace("%%", "%")));
        }
        // PowerShell: `$env:CLAUDE_CONFIG_DIR = '<dir>'`, `'` doubled.
        line.strip_prefix("$env:CLAUDE_CONFIG_DIR = '")?
            .strip_suffix('\'')
            .map(|dir| PathBuf::from(dir.replace("''", "'")))
    })
}
