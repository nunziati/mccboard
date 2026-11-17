//! Step 1: Claude Code itself.

use crossterm::event::KeyCode;
use mccboard_core::{ClaudeCode, Core, Error, OsKind};

use super::{Action, is_no, is_yes};
use crate::tui::setup::state::{Notice, Request};
use crate::tui::texts::Failure;

/// Claude Code as [`Core::claude_code`] finds it: where, and its version. When it is missing,
/// Enter offers its official installer and runs it only after the user confirms. On Windows
/// the installer is a PowerShell command, and the step shows it for the user to run.
pub struct ClaudeCodeStep {
    pub claude_code: ClaudeCode,
    /// The OS of this machine: it decides whether the installer can run from here.
    pub os: OsKind,
    /// The installer waits for the user's answer.
    pub confirm: bool,
}

impl ClaudeCodeStep {
    pub fn load(core: &Core, os: OsKind) -> Self {
        Self {
            claude_code: core.claude_code(),
            os,
            confirm: false,
        }
    }

    pub fn reload(&mut self, core: &Core) {
        self.claude_code = core.claude_code();
    }

    pub fn done(&self) -> bool {
        self.claude_code.is_found()
    }

    /// Whether mccboard can run the official installer here: on Linux.
    pub fn can_install(&self) -> bool {
        self.os != OsKind::Windows
    }

    pub fn key(&mut self, code: KeyCode) -> Action {
        if self.confirm {
            if is_yes(code) {
                self.confirm = false;
                return Action::Request(Request::InstallClaudeCode);
            }
            if is_no(code) {
                self.confirm = false;
                return Action::Notice(Notice::InstallCancelled);
            }
            return Action::None;
        }
        match code {
            KeyCode::Enter if !self.done() && self.can_install() => {
                self.confirm = true;
                Action::None
            }
            KeyCode::Enter => Action::Next,
            _ => Action::None,
        }
    }

    /// What the installer did, then Claude Code as found afterwards.
    pub fn installed(&mut self, result: Result<ClaudeCode, Error>) -> Notice {
        match result {
            Ok(found) => {
                let notice = if found.is_found() {
                    Notice::Installed
                } else {
                    Notice::InstallerFoundNothing
                };
                self.claude_code = found;
                notice
            }
            Err(error) => Notice::InstallFailed(Failure::from(&error)),
        }
    }
}
