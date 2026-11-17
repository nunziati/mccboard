//! Step 1: Claude Code itself.

use crossterm::event::KeyCode;
use mccboard_core::{ClaudeCode, Error, OsKind};

use super::Action;
use crate::tui::app::{is_no, is_yes};
use crate::tui::setup::state::{Notice, Request};
use crate::tui::texts::Failure;

/// Claude Code as [`mccboard_core::Core::claude_code`] finds it: where, and its version. That
/// lookup runs `claude --version` and may take seconds: the runner does it on a thread of its
/// own when the setup starts, and again only through the installer. Until it answers, the step
/// says it is looking.
///
/// When Claude Code is missing, Enter offers its official installer and runs it only after
/// the user confirms. On Windows the installer is a PowerShell command, and the step shows it
/// for the user to run.
pub struct ClaudeCodeStep {
    /// `None` while the runner looks for it.
    pub claude_code: Option<ClaudeCode>,
    /// The OS the core works on: it decides whether the installer can run from here.
    pub os: OsKind,
    /// The installer waits for the user's answer.
    pub confirm: bool,
}

impl ClaudeCodeStep {
    pub fn new(os: OsKind) -> Self {
        Self {
            claude_code: None,
            os,
            confirm: false,
        }
    }

    /// What the runner found.
    pub fn found(&mut self, claude_code: ClaudeCode) {
        self.claude_code = Some(claude_code);
    }

    pub fn done(&self) -> bool {
        self.claude_code.as_ref().is_some_and(ClaudeCode::is_found)
    }

    /// Whether mccboard can run the official installer here: not on Windows.
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
        match (&self.claude_code, code) {
            // Still looking: nothing to offer yet.
            (None, _) => Action::None,
            (Some(ClaudeCode::Missing), KeyCode::Enter) if self.can_install() => {
                self.confirm = true;
                Action::None
            }
            (Some(_), KeyCode::Enter) => Action::Next,
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
                self.claude_code = Some(found);
                notice
            }
            Err(error) => Notice::InstallFailed(Failure::from(&error)),
        }
    }
}
