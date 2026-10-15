//! The session guard: every operation that changes an instance (repair, alignment, removal
//! with its folder) first checks that no Claude Code process is using its config directory,
//! and refuses with the list of those processes if any is.
//!
//! The environment port lists Claude Code processes with the config directory each was
//! started with ([`ProcessConfigDir`]). A process whose environment can't be read (another
//! user's, an elevated one on Windows) counts when Claude Code's own session registry,
//! `<config dir>/sessions/<pid>.json`, names its pid in this instance.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{Environment, Error, paths};

/// A running Claude Code process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaudeProcess {
    pub pid: u32,
    /// The process name (`claude`, `claude.exe`, `node`).
    pub name: String,
    /// The config directory it was started with.
    pub config_dir: ProcessConfigDir,
    /// Its working directory, when readable: tells the user which session it is.
    pub cwd: Option<PathBuf>,
}

/// The config directory a Claude Code process uses, from its `CLAUDE_CONFIG_DIR`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessConfigDir {
    /// No `CLAUDE_CONFIG_DIR` and the process's home unknown: Claude Code's default, taken
    /// as the primary instance's `~/.claude`.
    Default,
    /// The folder it uses, absolute: its `CLAUDE_CONFIG_DIR`, or `.claude` in its own home.
    Dir { path: PathBuf },
    /// The process's environment can't be read.
    Unknown,
}

/// The Claude Code processes using the instance in `config_dir`.
pub(crate) fn processes_using(
    home: &Path,
    env: &dyn Environment,
    config_dir: &Path,
) -> Vec<ClaudeProcess> {
    let mut using: Vec<ClaudeProcess> = env
        .claude_processes()
        .into_iter()
        .filter(|process| uses(home, config_dir, process))
        .collect();
    using.sort_by_key(|p| p.pid);
    using
}

/// Refuses with [`Error::InstanceInUse`] while a Claude Code process uses `config_dir`.
/// Call it right before the first change to an instance.
pub(crate) fn ensure_idle(
    home: &Path,
    env: &dyn Environment,
    config_dir: &Path,
) -> Result<(), Error> {
    let processes = processes_using(home, env, config_dir);
    if processes.is_empty() {
        Ok(())
    } else {
        Err(Error::InstanceInUse {
            path: config_dir.to_path_buf(),
            processes,
        })
    }
}

fn uses(home: &Path, config_dir: &Path, process: &ClaudeProcess) -> bool {
    match &process.config_dir {
        ProcessConfigDir::Default => same_folder(&home.join(".claude"), config_dir),
        ProcessConfigDir::Dir { path } => same_folder(path, config_dir),
        ProcessConfigDir::Unknown => config_dir
            .join("sessions")
            .join(format!("{}.json", process.pid))
            .is_file(),
    }
}

/// Two spellings of one folder: equal once normalised, or once links (and on Windows short
/// names) are resolved.
fn same_folder(a: &Path, b: &Path) -> bool {
    if paths::same(a, b) {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => paths::same(&a, &b),
        _ => false,
    }
}
