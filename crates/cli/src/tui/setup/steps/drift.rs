//! Step 5: the drift of each instance.

use std::path::PathBuf;

use crossterm::event::KeyCode;
use mccboard_core::{Core, Doctor, Error, InstanceDrift, RepairReport};

use super::{Action, Read, is_no, is_yes, moved, read};
use crate::tui::setup::state::{Notice, Request};
use crate::tui::texts::Failure;

/// The drift of each registered instance, as the doctor reports it, with what a repair would
/// do for each item. Enter asks before repairing the selected instance, and the runner
/// repairs it on a thread of its own: installing a plugin runs Claude Code and takes a while.
pub struct DriftStep {
    pub doctor: Read<Doctor>,
    /// The selected instance, an index into the doctor's instances.
    pub selected: usize,
    /// The repair of the selected instance waits for the user's answer.
    pub confirm: bool,
    /// The instance being repaired.
    pub repairing: Option<PathBuf>,
}

impl DriftStep {
    pub fn load(core: &Core) -> Self {
        let mut step = Self {
            doctor: read(core.doctor()),
            selected: 0,
            confirm: false,
            repairing: None,
        };
        step.follow_drift();
        step
    }

    pub fn reload(&mut self, core: &Core) {
        self.doctor = read(core.doctor());
        self.follow_drift();
    }

    /// When the selected instance has no drift, selects the first instance with drift: after
    /// a repair, the next one to repair.
    fn follow_drift(&mut self) {
        let has_drift = |instance: &InstanceDrift| instance.drift_count > 0;
        if !self.selected_instance().is_some_and(has_drift)
            && let Some(at) = self.with_drift()
        {
            self.selected = at;
        }
        self.selected = self.selected.min(self.instances().len().saturating_sub(1));
    }

    /// Every registered instance with its drift, primary first then by path.
    pub fn instances(&self) -> &[InstanceDrift] {
        match &self.doctor {
            Ok(doctor) => &doctor.instances,
            Err(_) => &[],
        }
    }

    pub fn selected_instance(&self) -> Option<&InstanceDrift> {
        self.instances().get(self.selected)
    }

    /// The first instance with drift.
    fn with_drift(&self) -> Option<usize> {
        self.instances()
            .iter()
            .position(|instance| instance.drift_count > 0)
    }

    /// Done once no instance has drift.
    pub fn done(&self) -> bool {
        self.doctor.as_ref().is_ok_and(|doctor| !doctor.has_drift())
    }

    pub fn key(&mut self, code: KeyCode) -> Action {
        if self.confirm {
            return self.answer(code);
        }
        if let Some(selected) = moved(self.selected, self.instances().len(), code) {
            self.selected = selected;
            return Action::None;
        }
        if code != KeyCode::Enter {
            return Action::None;
        }
        // Enter asks about the selected instance, or goes to the next one with drift.
        if self
            .selected_instance()
            .is_some_and(|instance| instance.drift_count > 0)
        {
            self.confirm = true;
            return Action::None;
        }
        match self.with_drift() {
            Some(at) => {
                self.selected = at;
                Action::None
            }
            None => Action::Next,
        }
    }

    fn answer(&mut self, code: KeyCode) -> Action {
        if is_yes(code) {
            self.confirm = false;
            let Some(instance) = self.selected_instance() else {
                return Action::None;
            };
            let dir = instance.config_dir.clone();
            self.repairing = Some(dir.clone());
            return Action::Request(Request::Repair(dir));
        }
        if is_no(code) {
            self.confirm = false;
            return Action::Notice(Notice::RepairCancelled);
        }
        Action::None
    }

    /// What a repair did.
    pub fn repaired(&mut self, config_dir: PathBuf, result: Result<RepairReport, Error>) -> Notice {
        self.repairing = None;
        match result {
            Ok(report) => Notice::Repaired(report),
            Err(error) => Notice::RepairFailed {
                path: config_dir,
                failure: Failure::from(&error),
            },
        }
    }
}
