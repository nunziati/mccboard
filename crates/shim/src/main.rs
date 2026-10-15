//! The shim, run as a launcher (`claude-b.exe`): see the library for what it does.
//!
//! Arguments go to `claude` as `std` passes them: for `claude.exe` (the native installer) with
//! the quoting every Windows program parses back; for `claude.cmd` (an npm install) through
//! `cmd.exe /e:ON /v:OFF /d /c`, with `std`'s escaping for batch files (Rust 1.77+), which
//! quotes `&`, `^`, `!`, spaces and quotes and stops `%` expansion. What can't pass through
//! cmd.exe at all, a line break, `std` refuses instead of passing it on altered: the shim then
//! says so and starts nothing.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, ExitCode};

use mccboard_shim::{find_program, is_batch_file, sidecar};

/// The exit code when `claude` couldn't be started at all.
const FAILED: u8 = 1;

fn main() -> ExitCode {
    // Its own path says which sidecar to read: without it, no instance to start.
    let exe = match env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!(
                "mccboard launcher: cannot find its own program ({e}), so not which instance \
                 to start Claude Code in."
            );
            return ExitCode::from(FAILED);
        }
    };
    let name = exe
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    match run(&exe) {
        // The whole code: on Windows it is 32 bits, more than `ExitCode` holds.
        Ok(code) => std::process::exit(code),
        Err(message) => {
            eprintln!("{name}: {message}");
            ExitCode::from(FAILED)
        }
    }
}

/// Starts `claude` in the launcher's instance and waits for it: its exit code.
fn run(exe: &Path) -> Result<i32, String> {
    let sidecar_path = exe.with_extension(sidecar::EXTENSION);
    let text = fs::read_to_string(&sidecar_path).map_err(|e| {
        format!(
            "cannot read {} ({e}). Run `mccboard launcher sync` to repair the launchers.",
            sidecar_path.display()
        )
    })?;
    let config_dir = sidecar::parse(&text).ok_or_else(|| {
        format!(
            "{} names no instance. Run `mccboard launcher sync` to repair the launchers.",
            sidecar_path.display()
        )
    })?;
    let path = env::var_os("PATH").unwrap_or_default();
    let pathext = env::var_os("PATHEXT");
    let claude = find_program("claude", &path, pathext.as_deref(), exe.parent())
        .ok_or("cannot find Claude Code (claude) on the PATH. Is it installed?")?;

    outlive_console_events();
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let status = Command::new(&claude)
        .args(&args)
        .env("CLAUDE_CONFIG_DIR", &config_dir)
        .status()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::InvalidInput && is_batch_file(&claude) {
                format!(
                    "{} is a batch file, and cmd.exe can't pass these arguments to it \
                     unchanged (a line break, for one).",
                    claude.display()
                )
            } else {
                format!("cannot start {}: {e}", claude.display())
            }
        })?;
    Ok(status.code().unwrap_or(i32::from(FAILED)))
}

/// Ctrl+C and Ctrl+Break reach every program on the console: `claude` handles them, the shim
/// only waits for it to exit and returns its exit code. A handler is used, not
/// `SetConsoleCtrlHandler(NULL, TRUE)`: that one is inherited and would make `claude` ignore
/// Ctrl+C too.
#[cfg(windows)]
fn outlive_console_events() {
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler,
    };

    unsafe extern "system" fn handler(event: u32) -> windows_sys::core::BOOL {
        windows_sys::core::BOOL::from(event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT)
    }
    // SAFETY: `handler` is a plain function that lives as long as the program.
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

#[cfg(not(windows))]
fn outlive_console_events() {}
