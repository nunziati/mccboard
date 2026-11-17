//! Step 8: the summary.

use crossterm::event::KeyCode;
use mccboard_core::{Doctor, LauncherStatus, Registry};

use super::{Action, Read, Snapshot};
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
    pub fn new(snapshot: &Snapshot) -> Self {
        Self {
            registry: snapshot.registry.clone(),
            doctor: snapshot.doctor.as_ref().ok().cloned(),
            launchers: snapshot.launchers.as_ref().ok().cloned(),
        }
    }

    /// Takes the instances as read now.
    pub fn refresh(&mut self, snapshot: &Snapshot) {
        *self = Self::new(snapshot);
    }

    pub fn key(&mut self, code: KeyCode) -> Action {
        match code {
            KeyCode::Enter => Action::Request(Request::Dashboard),
            _ => Action::None,
        }
    }
}
