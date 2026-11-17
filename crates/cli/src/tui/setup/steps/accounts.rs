//! Step 3: the accounts, and a linked instance for each new one.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mccboard_core::{
    AddReport, AddRequest, Core, Error, InstanceStatus, LoginReport, LoginState, LoginTerminal,
    LoginWarning, Registry, Warning,
};

use super::{Action, Read, moved, read};
use crate::tui::setup::state::{Notice, Request};
use crate::tui::texts::Failure;

/// The registered instances with their account and login. `a` adds an account: its label,
/// then the folder of its instance as the core proposes it or as the user changes it, then the
/// add itself with Claude Code's login in this terminal. After an add the label of the next
/// account is asked, until the user leaves it empty. Enter on an instance that isn't logged
/// in opens its login.
pub struct AccountsStep {
    pub registry: Read<Registry>,
    /// The selected instance, an index into the registry's instances.
    pub selected: usize,
    /// The add being typed.
    pub form: Option<AddForm>,
}

/// An add being typed, one field at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddForm {
    /// The account's label.
    Label { text: String },
    /// The folder of its instance: `proposed` by the core, `text` as the user edits it.
    Folder {
        label: String,
        proposed: PathBuf,
        text: String,
    },
}

impl AddForm {
    fn text_mut(&mut self) -> &mut String {
        match self {
            AddForm::Label { text } | AddForm::Folder { text, .. } => text,
        }
    }
}

impl AccountsStep {
    pub fn load(core: &Core) -> Self {
        let mut step = Self {
            registry: Ok(Registry {
                accounts: Vec::new(),
                instances: Vec::new(),
            }),
            selected: 0,
            form: None,
        };
        step.reload(core);
        step
    }

    /// Reads the registry again. When the selected instance doesn't wait for its login, the
    /// first one that does is selected: after a login, the next one to log in.
    pub fn reload(&mut self, core: &Core) {
        self.registry = read(core.registry());
        if !self.selected_instance().is_some_and(needs_login)
            && let Some(at) = self.instances().iter().position(needs_login)
        {
            self.selected = at;
        }
        self.selected = self.selected.min(self.instances().len().saturating_sub(1));
    }

    /// The registered instances, primary first then by path.
    pub fn instances(&self) -> &[InstanceStatus] {
        match &self.registry {
            Ok(registry) => &registry.instances,
            Err(_) => &[],
        }
    }

    pub fn selected_instance(&self) -> Option<&InstanceStatus> {
        self.instances().get(self.selected)
    }

    /// Done once an instance is registered and none waits for its login.
    pub fn done(&self) -> bool {
        !self.instances().is_empty() && !self.instances().iter().any(needs_login)
    }

    /// Whether the keys go to the form being typed.
    pub fn typing(&self) -> bool {
        self.form.is_some()
    }

    pub fn key(&mut self, core: &Core, key: KeyEvent) -> Action {
        match self.form.take() {
            None => self.list_key(key.code),
            Some(form) => self.form_key(core, form, key),
        }
    }

    fn list_key(&mut self, code: KeyCode) -> Action {
        if let Some(selected) = moved(self.selected, self.instances().len(), code) {
            self.selected = selected;
            return Action::None;
        }
        match code {
            KeyCode::Char('a') => {
                self.form = Some(AddForm::Label {
                    text: String::new(),
                });
                Action::None
            }
            KeyCode::Enter => match self.selected_instance() {
                Some(instance) if needs_login(instance) => {
                    Action::Request(Request::Login(instance.config_dir.clone()))
                }
                _ => Action::Next,
            },
            _ => Action::None,
        }
    }

    /// A key while the form is typed. The form was taken out of the step: what stays of it is
    /// put back.
    fn form_key(&mut self, core: &Core, form: AddForm, key: KeyEvent) -> Action {
        match (form, key.code) {
            // Esc closes the form from the label, and goes back to the label from the folder.
            (AddForm::Label { .. }, KeyCode::Esc) => Action::None,
            (AddForm::Folder { label, .. }, KeyCode::Esc) => {
                self.form = Some(AddForm::Label { text: label });
                Action::None
            }
            (AddForm::Label { text }, KeyCode::Enter) => {
                let label = text.trim().to_owned();
                // No label: the user moves on.
                if label.is_empty() {
                    return Action::None;
                }
                match core.add_folder(&label) {
                    Ok(proposed) => {
                        self.form = Some(AddForm::Folder {
                            text: crate::tui::shown_path(core.home(), &proposed),
                            label,
                            proposed,
                        });
                        Action::None
                    }
                    Err(error) => {
                        self.form = Some(AddForm::Label { text });
                        Action::Notice(Notice::LabelRefused(Failure::from(&error)))
                    }
                }
            }
            (
                AddForm::Folder {
                    label,
                    proposed,
                    text,
                },
                KeyCode::Enter,
            ) => {
                let request = AddRequest {
                    config_dir: chosen_folder(core.home(), &proposed, &text),
                    ..AddRequest::new(label.clone(), LoginTerminal::CurrentTerminal)
                };
                // It stays on screen while Claude Code has the terminal, and after a failure.
                self.form = Some(AddForm::Folder {
                    label,
                    proposed,
                    text,
                });
                Action::Request(Request::Add(request))
            }
            (mut form, KeyCode::Backspace) => {
                form.text_mut().pop();
                self.form = Some(form);
                Action::None
            }
            (mut form, KeyCode::Char(c)) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                form.text_mut().push(c);
                self.form = Some(form);
                Action::None
            }
            (form, _) => {
                self.form = Some(form);
                Action::None
            }
        }
    }

    /// What a login did.
    pub fn logged_in(&mut self, config_dir: PathBuf, result: Result<LoginReport, Error>) -> Notice {
        match result {
            Ok(report) => login_notice(&report),
            Err(error) => Notice::LoginFailed {
                path: config_dir,
                failure: Failure::from(&error),
            },
        }
    }

    /// What an add did. Once its instance is logged in, the label of the next account is
    /// asked. After a failure the form stays as it was, to try again.
    pub fn added(&mut self, result: Result<AddReport, Error>) -> Notice {
        let report = match result {
            Ok(report) => report,
            Err(error) => return Notice::AddFailed(Failure::from(&error)),
        };
        let logged_in = report.login.account.is_some();
        self.form = logged_in.then(|| AddForm::Label {
            text: String::new(),
        });
        Notice::Added {
            path: report.config_dir.clone(),
            login: Box::new(login_notice(&report.login)),
            failed: report.setup.failed(),
        }
    }
}

/// Whether `instance` waits for its login: it has no credentials, or ones that can't be read.
/// An expired token needs no login (Claude Code renews it when it runs), and a folder that is
/// gone can't have one.
pub fn needs_login(instance: &InstanceStatus) -> bool {
    !instance
        .observation
        .warnings
        .contains(&Warning::ConfigDirMissing)
        && matches!(
            instance.observation.login,
            LoginState::NotLoggedIn | LoginState::Unknown
        )
}

/// The folder typed for a new instance: `None` for the one the core proposed.
fn chosen_folder(home: &Path, proposed: &Path, text: &str) -> Option<PathBuf> {
    let text = text.trim();
    let same = text.is_empty()
        || text == crate::tui::shown_path(home, proposed)
        || Path::new(text) == proposed;
    (!same).then(|| PathBuf::from(text))
}

/// How a login ended: the account it logged in with, or none.
fn login_notice(report: &LoginReport) -> Notice {
    let Some(account) = &report.account else {
        return Notice::NotLoggedIn(report.config_dir.clone());
    };
    let others = report
        .warnings
        .iter()
        .flat_map(|warning| match warning {
            LoginWarning::AccountHasOtherInstances { instances, .. } => instances.clone(),
        })
        .collect();
    Notice::LoggedIn {
        path: report.config_dir.clone(),
        label: account.label.clone(),
        others,
    }
}
