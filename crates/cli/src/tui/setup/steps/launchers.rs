//! Step 6: the launchers.

use crossterm::event::KeyCode;
use mccboard_core::{Core, LauncherStatus};

use super::{Action, Read, read};
use crate::tui::setup::state::Notice;
use crate::tui::texts::Failure;

/// The launchers (`claude-b`), one per registered linked instance, against what the
/// instances need. Enter makes them match with [`Core::sync_launchers`], or moves on when they
/// do. The view always shows the command that makes them work in the shell the setup runs in,
/// since a shell reads its rc file or its PATH only when it starts.
pub struct LaunchersStep {
    pub status: Read<LauncherStatus>,
}

impl LaunchersStep {
    pub fn load(core: &Core) -> Self {
        Self {
            status: read(core.launchers()),
        }
    }

    pub fn reload(&mut self, core: &Core) {
        self.status = read(core.launchers());
    }

    /// Done once nothing is left for a sync to fix.
    pub fn done(&self) -> bool {
        self.status
            .as_ref()
            .is_ok_and(|status| status.drift_count() == 0)
    }

    pub fn key(&mut self, core: &Core, code: KeyCode) -> Action {
        match code {
            KeyCode::Enter if self.done() => Action::Next,
            // Launchers that can't be read can't be written either: the sync says why.
            KeyCode::Enter => Action::Notice(match core.sync_launchers() {
                Ok(sync) => Notice::LaunchersSynced(sync),
                Err(error) => Notice::LaunchersFailed(Failure::from(&error)),
            }),
            _ => Action::None,
        }
    }
}
