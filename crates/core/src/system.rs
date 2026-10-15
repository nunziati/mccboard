//! What [`crate::SystemEnvironment`] asks the operating system: running Claude Code processes
//! and, on Windows, whether Developer Mode is on.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use sysinfo::{Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::session_guard::{ClaudeProcess, ProcessConfigDir};

const CONFIG_DIR_VAR: &str = "CLAUDE_CONFIG_DIR";

/// Every running Claude Code process of this machine that mccboard can see, mccboard's own
/// process left out.
pub(crate) fn claude_processes() -> Vec<ClaudeProcess> {
    let mut system = System::new();
    let refresh = ProcessRefreshKind::nothing()
        .without_tasks()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_environ(UpdateKind::OnlyIfNotSet)
        .with_cwd(UpdateKind::OnlyIfNotSet);
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
    let own = std::process::id();
    system
        .processes()
        .values()
        .filter(|p| p.pid().as_u32() != own && p.thread_kind().is_none() && is_claude(p))
        .map(|p| ClaudeProcess {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().into_owned(),
            config_dir: config_dir(p),
            cwd: p.cwd().map(Path::to_path_buf),
        })
        .collect()
}

/// The native build runs as `claude` (`claude.exe`); an npm install runs `node` on Claude
/// Code's `cli.js`. Child processes (shells, hooks) are other programs and don't count.
fn is_claude(p: &Process) -> bool {
    let stem = |path: &Path| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    let name = stem(Path::new(p.name()));
    let exe = p.exe().map(stem).unwrap_or_default();
    let argv0 = p
        .cmd()
        .first()
        .map(|a| stem(Path::new(a)))
        .unwrap_or_default();
    if [&name, &exe, &argv0].iter().any(|n| n.as_str() == "claude") {
        return true;
    }
    let node = name == "node" || exe == "node";
    node && p.cmd().iter().skip(1).any(|arg| {
        let arg = arg.to_string_lossy().replace('\\', "/");
        arg.contains("@anthropic-ai/claude-code") || arg.contains("claude-code/cli")
    })
}

fn config_dir(p: &Process) -> ProcessConfigDir {
    let environ = p.environ();
    if environ.is_empty() {
        // Every process has some environment: an empty one is one we can't read.
        return ProcessConfigDir::Unknown;
    }
    let var = |name: &str| environ.iter().find_map(|entry| env_value(entry, name));
    let absolute = |value: PathBuf| {
        if value.is_absolute() {
            Some(value)
        } else {
            p.cwd().map(|cwd| crate::paths::normalize(&cwd.join(value)))
        }
    };
    match var(CONFIG_DIR_VAR).filter(|v| !v.as_os_str().is_empty()) {
        Some(value) => match absolute(value) {
            Some(path) => ProcessConfigDir::Dir { path },
            None => ProcessConfigDir::Unknown,
        },
        // Claude Code's default is `.claude` in the process's own home, which may differ
        // from the home mccboard works in.
        None => match var(HOME_VAR).filter(|v| v.is_absolute()) {
            Some(home) => ProcessConfigDir::Dir {
                path: home.join(".claude"),
            },
            None => ProcessConfigDir::Default,
        },
    }
}

/// The variable holding the home directory Claude Code uses (Node's `os.homedir()`).
const HOME_VAR: &str = if cfg!(windows) { "USERPROFILE" } else { "HOME" };

/// The value of variable `name` in one `NAME=value` entry (names are case-insensitive on
/// Windows).
fn env_value(entry: &OsStr, name: &str) -> Option<PathBuf> {
    let entry = entry.to_string_lossy();
    let (key, value) = entry.split_once('=')?;
    let matches = if cfg!(windows) {
        key.eq_ignore_ascii_case(name)
    } else {
        key == name
    };
    matches.then(|| PathBuf::from(value))
}

/// Whether file symlinks can be made without elevation: Developer Mode on Windows
/// (`HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock`,
/// `AllowDevelopmentWithoutDevLicense` = 1).
#[cfg(windows)]
pub(crate) fn file_links_allowed() -> bool {
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};

    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock",
            KEY_READ,
        )
        .and_then(|key| key.get_value::<u32, _>("AllowDevelopmentWithoutDevLicense"))
        .is_ok_and(|value| value == 1)
}

/// Elsewhere anyone can make a symlink.
#[cfg(not(windows))]
pub(crate) fn file_links_allowed() -> bool {
    true
}
