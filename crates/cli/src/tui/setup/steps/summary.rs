//! Step 8: the summary.

use crossterm::event::KeyCode;
use mccboard_core::{Core, Doctor, LauncherStatus, Registry};

use super::{Action, Read, read};
use crate::tui::setup::state::Request;

/// The accounts and instances as they now are, each instance with its login, drift and
/// launcher. Enter opens the dashboard.
pub struct SummaryStep {
    pub registry: Read<Registry>,
    /// `None` when the drift can't be read.
    pub doctor: Option<Doctor>,
    /// `None` when the launchers can't be read.
    pub launchers: Option<LauncherStatus>,
}

impl SummaryStep {
    pub fn load(core: &Core) -> Self {
        Self {
            registry: read(core.registry()),
            doctor: core.doctor().ok(),
            launchers: core.launchers().ok(),
        }
    }

    pub fn reload(&mut self, core: &Core) {
        *self = Self::load(core);
    }

    pub fn key(&mut self, code: KeyCode) -> Action {
        match code {
            KeyCode::Enter => Action::Request(Request::Dashboard),
            _ => Action::None,
        }
    }
}
