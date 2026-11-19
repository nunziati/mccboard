//! The eight steps of the setup. Each is a plain struct that reads from the core what is
//! already done, when it loads and again after every action, says whether its work is done,
//! and turns keys into an [`Action`]. Quick operations (registering, answering about an item,
//! the launchers, the preferences) run in the step, through the core. What needs the
//! terminal or a thread of its own goes out as a [`Request`].
//!
//! The registry, the doctor and the launchers observe every instance: the setup reads them once
//! per reload into one [`Snapshot`] for all the steps.

mod accounts;
mod claude_code;
mod drift;
mod instances;
mod launchers;
mod preferences;
mod sharing;
mod summary;

pub use accounts::{AccountsStep, AddForm, needs_login};
pub use claude_code::ClaudeCodeStep;
pub use drift::DriftStep;
pub use instances::InstancesStep;
pub use launchers::LaunchersStep;
pub use preferences::{Preference, PreferencesStep};
pub use sharing::SharingStep;
pub use summary::SummaryStep;

use crossterm::event::KeyCode;
use mccboard_core::{Core, Doctor, Error, Language, LauncherStatus, Registry};

use super::state::{Notice, Request};
use crate::tui::texts::Failure;

/// What a key did in a step, for the setup to carry out.
#[derive(Debug)]
pub enum Action {
    /// Nothing changed outside the step.
    None,
    /// Something was done or refused: the setup tells how it went and reads every step again.
    Notice(Notice),
    /// Something for the runner: hand the terminal to Claude Code, repair on a thread, or
    /// leave the setup.
    Request(Request),
    /// The step has nothing left to do, and the next one shows.
    Next,
    /// The user chose this language for mccboard.
    Language(Language),
}

/// The selection among `len` rows after the arrow `code`, or `None` when `code` isn't an
/// arrow up or down.
pub fn moved(selected: usize, len: usize, code: KeyCode) -> Option<usize> {
    match code {
        KeyCode::Up => Some(selected.saturating_sub(1)),
        KeyCode::Down => Some((selected + 1).min(len.saturating_sub(1))),
        _ => None,
    }
}

/// What the core read, or why it couldn't.
pub type Read<T> = Result<T, Failure>;

/// `result` with its error as a front end tells it.
pub fn read<T>(result: Result<T, Error>) -> Read<T> {
    result.map_err(|error| Failure::from(&error))
}

/// The registered instances as the core reports them now, read once for every step that shows
/// them.
pub struct Snapshot {
    pub registry: Read<Registry>,
    /// The drift of each instance, with the unknown items of the primary instance.
    pub doctor: Read<Doctor>,
    pub launchers: Read<LauncherStatus>,
}

impl Snapshot {
    pub fn read(core: &Core) -> Snapshot {
        Snapshot {
            registry: read(core.registry()),
            doctor: read(core.doctor()),
            launchers: read(core.launchers()),
        }
    }
}
