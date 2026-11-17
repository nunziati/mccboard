//! Step 7: mccboard's preferences.

use crossterm::event::KeyCode;
use mccboard_core::{Core, Language};

use super::{Action, Read, read};
use crate::tui::setup::state::Notice;
use crate::tui::texts::Failure;

/// The language of mccboard, and whether Claude Code may renew an expired token (ADR 0002).
/// Space changes the selected one, and the core saves it in `config.toml` at once.
pub struct PreferencesStep {
    /// Whether a usage refresh may ask Claude Code to renew an expired token.
    pub renew: Read<bool>,
    pub selected: Preference,
}

/// A preference of the step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preference {
    Language,
    Renewal,
}

impl PreferencesStep {
    pub fn load(core: &Core) -> Self {
        Self {
            renew: read(core.renew_expired_tokens()),
            selected: Preference::Language,
        }
    }

    pub fn reload(&mut self, core: &Core) {
        self.renew = read(core.renew_expired_tokens());
    }

    /// The preferences always have a value: nothing is left to do.
    pub fn done(&self) -> bool {
        true
    }

    /// `language` is the one in use.
    pub fn key(&mut self, core: &Core, language: Language, code: KeyCode) -> Action {
        match code {
            KeyCode::Up => self.selected = Preference::Language,
            KeyCode::Down => self.selected = Preference::Renewal,
            KeyCode::Char(' ') => return self.change(core, language),
            KeyCode::Enter => return Action::Next,
            _ => {}
        }
        Action::None
    }

    fn change(&mut self, core: &Core, language: Language) -> Action {
        match self.selected {
            Preference::Language => {
                let all = Language::ALL;
                let at = all.iter().position(|l| *l == language).unwrap_or(0);
                Action::Language(all[(at + 1) % all.len()])
            }
            Preference::Renewal => {
                // Unreadable, config.toml can't be changed either: saving says why.
                let on = !self.renew.as_ref().is_ok_and(|on| *on);
                Action::Notice(match core.set_renew_expired_tokens(on) {
                    Ok(()) => Notice::Renewal(on),
                    Err(error) => Notice::PreferenceNotSaved(Failure::from(&error)),
                })
            }
        }
    }
}
