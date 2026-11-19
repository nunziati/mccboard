//! Step 4: the unknown items of the primary instance.

use crossterm::event::KeyCode;
use mccboard_core::{Core, Doctor, SharingAnswer, SharingQuestion};

use super::{Action, Read};
use crate::tui::setup::state::Notice;
use crate::tui::texts::Failure;

/// The items of the primary instance that mccboard doesn't know, one at a time, as `mccboard
/// doctor` asks about them: `l` links the item in every linked instance, `p` keeps it private.
/// The core keeps the answer in `config.toml`. `s` skips an item, which is asked again the
/// next time.
pub struct SharingStep {
    pub questions: Read<Vec<SharingQuestion>>,
    /// The items skipped in this run.
    pub skipped: Vec<String>,
}

impl SharingStep {
    pub fn new(doctor: &Read<Doctor>) -> Self {
        let mut step = Self {
            questions: Ok(Vec::new()),
            skipped: Vec::new(),
        };
        step.refresh(doctor);
        step
    }

    /// Takes the questions of the doctor's report as read now.
    pub fn refresh(&mut self, doctor: &Read<Doctor>) {
        self.questions = doctor
            .as_ref()
            .map(|doctor| doctor.questions.clone())
            .map_err(Clone::clone);
    }

    /// The questions waiting for an answer, none when they can't be read.
    pub fn questions(&self) -> &[SharingQuestion] {
        self.questions.as_deref().unwrap_or_default()
    }

    /// The question asked now: the first one not skipped.
    pub fn asked(&self) -> Option<&SharingQuestion> {
        self.questions()
            .iter()
            .find(|question| !self.skipped.contains(&question.item))
    }

    /// Done once no item waits for an answer.
    pub fn done(&self) -> bool {
        self.questions.as_ref().is_ok_and(Vec::is_empty)
    }

    pub fn key(&mut self, core: &Core, code: KeyCode) -> Action {
        let Some(question) = self.asked() else {
            return match code {
                KeyCode::Enter => Action::Next,
                _ => Action::None,
            };
        };
        let item = question.item.clone();
        let answer = match code {
            KeyCode::Char('l') => SharingAnswer::Link,
            KeyCode::Char('p') => SharingAnswer::Private,
            KeyCode::Char('s') => {
                self.skipped.push(item);
                return Action::None;
            }
            _ => return Action::None,
        };
        Action::Notice(match core.answer_sharing(&item, answer) {
            Ok(()) => Notice::Shared { item, answer },
            Err(error) => Notice::ShareFailed(Failure::from(&error)),
        })
    }
}
