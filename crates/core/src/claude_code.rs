//! Claude Code itself on this machine, as a setup needs it: whether it is installed, its
//! version, and its official installer (#62).
//!
//! Claude Code is looked for on the PATH, then in `~/.local/bin`, where its official installer
//! puts it before the PATH knows that folder. The login, the background runs and the token
//! renewal find it in the same two places.
//! Its version comes from `claude --version`, run in the background. Nothing here opens a
//! file of any instance: no credential is ever read.
//!
//! The installer is Claude Code's own, run in the current terminal through the environment
//! port, and only after the user has confirmed. On Windows it is a PowerShell command, which
//! mccboard leaves to the user.

use std::io;
use std::path::PathBuf;

use serde::Serialize;

use crate::{ClaudeRun, Core, Error, InstallerInvocation, OsKind};

/// Claude Code's official installer on Linux, as its documentation gives it. A front end shows
/// it before asking to run it.
pub const CLAUDE_CODE_INSTALL_COMMAND: &str = "curl -fsSL https://claude.ai/install.sh | bash";

/// Claude Code's official installer on Windows, a PowerShell command. mccboard never runs it:
/// the user does.
pub const CLAUDE_CODE_WINDOWS_INSTALL_COMMAND: &str = "irm https://claude.ai/install.ps1 | iex";

/// Claude Code on this machine, as [`Core::claude_code`] finds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClaudeCode {
    /// Found on the PATH, or in `~/.local/bin`.
    Found {
        /// `claude` on the PATH (on Windows `claude.exe`, or an npm install's `claude.cmd`),
        /// else `~/.local/bin/claude` (`claude.exe` on Windows).
        program: PathBuf,
        /// Whether it is on the PATH, where the launchers and the user's own shell find it.
        /// `false` when it is only in `~/.local/bin` and the PATH lacks that folder: the login,
        /// the background runs and the token renewal find it there all the same, and a new
        /// terminal, or the line the installer gave for the shell profile, adds it.
        on_path: bool,
        /// The first line of `claude --version` (`2.1.301 (Claude Code)`), `None` when it
        /// couldn't be read.
        version: Option<String>,
    },
    /// Neither on the PATH nor in `~/.local/bin`.
    Missing,
}

impl ClaudeCode {
    /// Whether Claude Code was found.
    pub fn is_found(&self) -> bool {
        matches!(self, ClaudeCode::Found { .. })
    }
}

impl Core {
    /// Whether Claude Code is installed, where, and its version. `claude` is looked for on
    /// the PATH, then in `.local/bin` of this core's home (`claude.exe` on Windows), where the
    /// official installer puts it: the login and Claude Code's background runs find it the
    /// same way.
    /// The version is the first line of `claude --version`, run hidden, with no input, and
    /// stopped after a few seconds. A version that can't be read leaves Claude Code found,
    /// without one. Opens no file of any instance.
    pub fn claude_code(&self) -> ClaudeCode {
        let (program, on_path) = match self.env.find_program("claude") {
            Some(program) => (program, true),
            None => {
                let local = crate::claude_cli::installed_claude(&self.home, self.env.os());
                if !crate::claude_cli::is_program(&local) {
                    return ClaudeCode::Missing;
                }
                (local, false)
            }
        };
        let version = self
            .env
            .claude_version(&program)
            .ok()
            .and_then(|run| version(&run));
        ClaudeCode::Found {
            program,
            on_path,
            version,
        }
    }

    /// Runs Claude Code's official installer ([`CLAUDE_CODE_INSTALL_COMMAND`]) attached to
    /// the current terminal, with `curl` and `bash` from the PATH and this core's home as
    /// `HOME`, then looks for Claude Code again ([`Core::claude_code`]). Front ends call it
    /// only after the user has confirmed. Running it with Claude Code installed installs it
    /// again, as the official command does.
    ///
    /// Refuses, before running anything, on Windows, naming the PowerShell command
    /// ([`Error::ClaudeInstallerOnWindows`]), and without `curl` or `bash` on the PATH
    /// ([`Error::ClaudeInstallerNeedsProgram`]). Fails with the exit code when the script
    /// can't be downloaded ([`Error::ClaudeInstallerNotDownloaded`]) or the installer fails
    /// ([`Error::ClaudeInstallerFailed`]): its own messages in the terminal say why.
    pub fn install_claude_code(&self) -> Result<ClaudeCode, Error> {
        if self.env.os() == OsKind::Windows {
            return Err(Error::ClaudeInstallerOnWindows);
        }
        let find = |name: &str| {
            self.env
                .find_program(name)
                .ok_or_else(|| Error::ClaudeInstallerNeedsProgram {
                    program: name.to_owned(),
                })
        };
        let invocation = InstallerInvocation {
            curl: find("curl")?,
            bash: find("bash")?,
            home: self.home.clone(),
        };
        let exit =
            self.env
                .install_claude_code(&invocation)
                .map_err(|error| match error.kind() {
                    io::ErrorKind::Unsupported => Error::ClaudeInstallerOnWindows,
                    _ => Error::ClaudeInstallerNotStarted {
                        reason: error.to_string(),
                    },
                })?;
        // The installer's own failure first: when bash stops early, curl may fail writing to it.
        if exit.exit_code != Some(0) {
            return Err(Error::ClaudeInstallerFailed {
                exit_code: exit.exit_code,
            });
        }
        if exit.download_exit_code != Some(0) {
            return Err(Error::ClaudeInstallerNotDownloaded {
                exit_code: exit.download_exit_code,
            });
        }
        Ok(self.claude_code())
    }
}

/// The version `claude --version` printed: its first line, trimmed, when it exited with 0.
fn version(run: &ClaudeRun) -> Option<String> {
    if !run.success() {
        return None;
    }
    let line = run.stdout.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}
