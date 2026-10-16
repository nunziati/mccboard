//! What [`crate::SystemEnvironment`] asks the operating system: running Claude Code processes
//! and Claude desktop app processes, where the launcher shim is and, on Windows, whether
//! Developer Mode is on.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sysinfo::{Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::desktop_launch::DesktopLaunch;
use crate::desktop_process::ProcessListing;
use crate::session_guard::{ClaudeProcess, ProcessConfigDir};

const CONFIG_DIR_VAR: &str = "CLAUDE_CONFIG_DIR";

/// The launcher shim's bytes, from next to the running program: `cargo build` puts every
/// workspace program in one folder, and the app's installer puts the shim next to the app.
pub(crate) fn launcher_shim() -> io::Result<Vec<u8>> {
    let file = format!(
        "{}{}",
        mccboard_shim::PROGRAM_NAME,
        std::env::consts::EXE_SUFFIX
    );
    let path = std::env::current_exe()?.with_file_name(file);
    std::fs::read(&path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}

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

/// Every running process named `Claude` that mccboard can see (the desktop app's `Claude.exe`
/// and its helpers, and Claude Code's `claude`), with its program, parent and command line.
/// [`crate::desktop_process`] tells them apart.
pub(crate) fn desktop_processes() -> Vec<ProcessListing> {
    let mut system = System::new();
    let refresh = ProcessRefreshKind::nothing()
        .without_tasks()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet);
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
    let stem = |path: &Path| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    system
        .processes()
        .values()
        .filter(|p| p.thread_kind().is_none())
        .filter(|p| {
            stem(Path::new(p.name())) == "claude"
                || p.exe().is_some_and(|exe| stem(exe) == "claude")
        })
        .map(|p| ProcessListing {
            pid: p.pid().as_u32(),
            parent_pid: p.parent().map(|parent| parent.as_u32()),
            name: p.name().to_string_lossy().into_owned(),
            exe: p.exe().map(Path::to_path_buf),
            args: p
                .cmd()
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
        })
        .collect()
}

/// Starts the desktop app as `launch` says and returns its pid, without waiting: its input and
/// output go nowhere (Electron would log to an inherited console), and dropping the child
/// doesn't stop it.
pub(crate) fn launch_desktop_app(launch: &DesktopLaunch) -> io::Result<u32> {
    let mut command = if launch.console_window {
        Command::new(&launch.program)
    } else {
        crate::background::command(&launch.program)
    };
    command.args(&launch.args);
    for name in stripped_vars(std::env::vars_os().map(|(name, _)| name), launch) {
        command.env_remove(name);
    }
    for (name, value) in &launch.set_env {
        command.env(name, value);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = command
        .spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", launch.program.display())))?;
    Ok(child.id())
}

/// The inherited variables (`names`) that `launch` removes: those starting with one of its
/// prefixes, ASCII case ignored.
fn stripped_vars(names: impl Iterator<Item = OsString>, launch: &DesktopLaunch) -> Vec<OsString> {
    names
        .filter(|name| {
            let name = name.to_string_lossy();
            launch.remove_env_prefixes.iter().any(|prefix| {
                name.get(..prefix.len())
                    .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(names: &[&str]) -> impl Iterator<Item = OsString> {
        names
            .iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn the_launch_strips_every_claude_and_anthropic_variable_whatever_its_case() {
        let launch = crate::desktop_launch::launch(
            PathBuf::from("Claude.exe"),
            Some(Path::new("/p/b")),
            Some(Path::new("/h/.claude-b")),
        );
        let stripped = stripped_vars(
            names(&[
                "PATH",
                "CLAUDE_CONFIG_DIR",
                "CLAUDE_CODE_OAUTH_TOKEN",
                "CLAUDECODE",
                "ANTHROPIC_API_KEY",
                "anthropic_base_url",
                "Claude_Code_Entrypoint",
                "MY_CLAUDE",
                "USERPROFILE",
            ]),
            &launch,
        );
        assert_eq!(
            stripped,
            [
                "CLAUDE_CONFIG_DIR",
                "CLAUDE_CODE_OAUTH_TOKEN",
                "CLAUDECODE",
                "ANTHROPIC_API_KEY",
                "anthropic_base_url",
                "Claude_Code_Entrypoint",
            ]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn the_launched_program_gets_the_variable_set_and_the_others_removed() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("env.txt");
        let script = dir.path().join("fake-claude.sh");
        let o = out.display();
        std::fs::write(
            &script,
            format!("env > '{o}.part' && mv '{o}.part' '{o}'\n"),
        )
        .unwrap();
        let mut launch = crate::desktop_launch::launch(
            PathBuf::from("/bin/sh"),
            None,
            Some(Path::new("/h/.claude-b")),
        );
        launch.args = vec![script.display().to_string()];
        let pid = launch_desktop_app(&launch).unwrap();
        assert!(pid > 0);
        let mut text = String::new();
        for _ in 0..200 {
            text = std::fs::read_to_string(&out).unwrap_or_default();
            if !text.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let ours: Vec<&str> = text
            .lines()
            .filter(|line| {
                let upper = line.to_ascii_uppercase();
                upper.starts_with("CLAUDE") || upper.starts_with("ANTHROPIC")
            })
            .collect();
        // Whatever CLAUDE*/ANTHROPIC* variables the tests run with (CLAUDECODE under Claude
        // Code), only the one set is left.
        assert_eq!(ours, ["CLAUDE_CONFIG_DIR=/h/.claude-b"], "{text}");
    }
}
