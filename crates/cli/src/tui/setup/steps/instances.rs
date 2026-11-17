//! Step 2: the instances in the home.

use std::path::PathBuf;

use crossterm::event::KeyCode;
use mccboard_core::{Candidate, Core, InstanceKind};

use super::{Action, Read, moved, read};
use crate::tui::setup::state::Notice;
use crate::tui::texts::Failure;

/// The instances a scan finds in the home (`~/.claude`, `~/.claude-*`), the primary instance
/// first. Space checks the ones to register, and the primary instance starts checked when it
/// isn't registered yet. Enter registers the checked ones, or moves on when none is.
pub struct InstancesStep {
    pub candidates: Read<Vec<Candidate>>,
    /// The folders checked for registering, all of them unregistered.
    pub checked: Vec<PathBuf>,
    pub selected: usize,
}

impl InstancesStep {
    pub fn load(core: &Core) -> Self {
        let mut step = Self {
            candidates: Ok(Vec::new()),
            checked: Vec::new(),
            selected: 0,
        };
        step.reload(core);
        step.checked = step
            .list()
            .iter()
            .filter(|c| !c.registered && c.kind == InstanceKind::Primary)
            .map(|c| c.config_dir.clone())
            .collect();
        step
    }

    pub fn reload(&mut self, core: &Core) {
        self.candidates = read(core.scan());
        let candidates = self.candidates.as_deref().unwrap_or_default();
        self.checked.retain(|dir| {
            candidates
                .iter()
                .any(|c| !c.registered && &c.config_dir == dir)
        });
        self.selected = self.selected.min(candidates.len().saturating_sub(1));
    }

    /// The candidates found, none when the home can't be read.
    pub fn list(&self) -> &[Candidate] {
        self.candidates.as_deref().unwrap_or_default()
    }

    pub fn is_checked(&self, candidate: &Candidate) -> bool {
        self.checked.contains(&candidate.config_dir)
    }

    /// The primary instance, when the scan found its folder.
    pub fn primary(&self) -> Option<&Candidate> {
        self.list().iter().find(|c| c.kind == InstanceKind::Primary)
    }

    /// Done once the primary instance is registered and nothing checked waits.
    pub fn done(&self) -> bool {
        self.primary().is_some_and(|primary| primary.registered) && self.checked.is_empty()
    }

    pub fn key(&mut self, core: &Core, code: KeyCode) -> Action {
        if let Some(selected) = moved(self.selected, self.list().len(), code) {
            self.selected = selected;
            return Action::None;
        }
        match code {
            KeyCode::Char(' ') => {
                self.toggle();
                Action::None
            }
            KeyCode::Enter if self.checked.is_empty() => Action::Next,
            KeyCode::Enter => Action::Notice(self.register(core)),
            _ => Action::None,
        }
    }

    /// Checks the selected candidate, or unchecks it. A registered one stays as it is.
    fn toggle(&mut self) {
        let Some(candidate) = self.list().get(self.selected) else {
            return;
        };
        if candidate.registered {
            return;
        }
        let dir = candidate.config_dir.clone();
        match self.checked.iter().position(|checked| *checked == dir) {
            Some(at) => {
                self.checked.remove(at);
            }
            None => self.checked.push(dir),
        }
    }

    /// Registers the checked candidates in the order the scan lists them, stopping at the
    /// first that fails.
    fn register(&mut self, core: &Core) -> Notice {
        let dirs: Vec<PathBuf> = self
            .list()
            .iter()
            .filter(|c| self.is_checked(c))
            .map(|c| c.config_dir.clone())
            .collect();
        let mut registered = Vec::new();
        for dir in dirs {
            match core.register(&dir) {
                Ok(registration) => {
                    self.checked.retain(|checked| *checked != dir);
                    registered.push(registration.instance.config_dir);
                }
                Err(error) => {
                    return Notice::RegisterFailed {
                        path: dir,
                        failure: Failure::from(&error),
                        registered,
                    };
                }
            }
        }
        Notice::Registered(registered)
    }
}
