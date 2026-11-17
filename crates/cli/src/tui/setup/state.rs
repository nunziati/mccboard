//! The setup's state: the eight steps, the one shown, and what each key does, with no
//! terminal. Keys come in through [`Setup::key`], what the runner did through
//! [`Setup::message`]. What needs the terminal or a thread of its own (the installer, a login,
//! an add, a repair) goes out as a [`Request`].
//!
//! After every action each step reads again from the core what is done, and the list of steps
//! marks the ones whose work is done as they are now.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use mccboard_core::{
    AddReport, AddRequest, ClaudeCode, Core, Error, Language, LauncherSync, LoginReport, OsKind,
    RepairOutcome, RepairReport, SharingAnswer,
};

use super::steps::{
    AccountsStep, Action, ClaudeCodeStep, DriftStep, InstancesStep, LaunchersStep, PreferencesStep,
    SharingStep, SummaryStep,
};
use crate::tui::Show;
use crate::tui::texts::{self, Failure, Texts};

/// A step of the setup, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepId {
    ClaudeCode,
    Instances,
    Accounts,
    Sharing,
    Drift,
    Launchers,
    Preferences,
    Summary,
}

impl StepId {
    pub const ALL: [StepId; 8] = [
        StepId::ClaudeCode,
        StepId::Instances,
        StepId::Accounts,
        StepId::Sharing,
        StepId::Drift,
        StepId::Launchers,
        StepId::Preferences,
        StepId::Summary,
    ];

    /// Its number, from 1.
    pub fn number(self) -> usize {
        Self::ALL.iter().position(|step| *step == self).unwrap_or(0) + 1
    }

    pub fn next(self) -> Option<StepId> {
        Self::ALL.get(self.number()).copied()
    }

    pub fn previous(self) -> Option<StepId> {
        self.number().checked_sub(2).map(|at| Self::ALL[at])
    }
}

/// What the setup asks its runner to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Run Claude Code's official installer, in the terminal: the user has confirmed.
    InstallClaudeCode,
    /// Open Claude Code's login of the registered instance in this folder, in the terminal.
    Login(PathBuf),
    /// Open Claude Code's first login in the primary instance, whose folder Claude Code
    /// hasn't made yet, in the terminal.
    LoginPrimary,
    /// Add a linked instance, with its login in the terminal.
    Add(AddRequest),
    /// Repair every drift item of the instance in this folder: the user has confirmed.
    Repair(PathBuf),
    /// Leave the setup for the dashboard.
    Dashboard,
    Quit,
}

/// What the runner tells the setup once a request is done.
#[derive(Debug)]
pub enum Message {
    Installed(Result<ClaudeCode, Error>),
    LoggedIn {
        config_dir: PathBuf,
        result: Result<LoginReport, Error>,
    },
    Added(Result<AddReport, Error>),
    Repaired {
        config_dir: PathBuf,
        result: Result<RepairReport, Error>,
    },
}

/// The line under the steps: how the last action went. Kept as data and written at draw time,
/// so it follows a change of language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    InstallCancelled,
    Installed,
    /// The installer ended well, but Claude Code is neither on the PATH nor in `~/.local/bin`.
    InstallerFoundNothing,
    InstallFailed(Failure),
    Registered(Vec<PathBuf>),
    RegisterFailed {
        path: PathBuf,
        failure: Failure,
        /// The ones registered before it.
        registered: Vec<PathBuf>,
    },
    LabelRefused(Failure),
    LoggedIn {
        path: PathBuf,
        label: String,
        /// The account's other instances here: perhaps the browser used the wrong account.
        others: Vec<PathBuf>,
    },
    NotLoggedIn(PathBuf),
    LoginFailed {
        path: PathBuf,
        failure: Failure,
    },
    Added {
        path: PathBuf,
        /// How its login went: [`Notice::LoggedIn`] or [`Notice::NotLoggedIn`].
        login: Box<Notice>,
        /// Setup items (links, plugins) that failed. Adding the instance again retries them.
        failed: usize,
    },
    AddFailed(Failure),
    Shared {
        item: String,
        answer: SharingAnswer,
    },
    ShareFailed(Failure),
    RepairCancelled,
    Repairing(PathBuf),
    Repaired(RepairReport),
    RepairFailed {
        path: PathBuf,
        failure: Failure,
    },
    LaunchersSynced(LauncherSync),
    LaunchersFailed(Failure),
    LanguageSet,
    Renewal(bool),
    PreferenceNotSaved(Failure),
}

impl Notice {
    /// Whether it tells of something that went wrong.
    pub fn is_error(&self) -> bool {
        match self {
            Notice::InstallerFoundNothing
            | Notice::InstallFailed(_)
            | Notice::RegisterFailed { .. }
            | Notice::LabelRefused(_)
            | Notice::NotLoggedIn(_)
            | Notice::LoginFailed { .. }
            | Notice::AddFailed(_)
            | Notice::ShareFailed(_)
            | Notice::RepairFailed { .. }
            | Notice::LaunchersFailed(_)
            | Notice::PreferenceNotSaved(_) => true,
            Notice::Added { login, failed, .. } => *failed > 0 || login.is_error(),
            Notice::Repaired(report) => report.failed() > 0,
            _ => false,
        }
    }
}

/// The setup's state.
pub struct Setup {
    pub(super) core: Core,
    pub(super) language: Language,
    pub(super) step: StepId,
    pub(super) claude_code: ClaudeCodeStep,
    pub(super) instances: InstancesStep,
    pub(super) accounts: AccountsStep,
    pub(super) sharing: SharingStep,
    pub(super) drift: DriftStep,
    pub(super) launchers: LaunchersStep,
    pub(super) preferences: PreferencesStep,
    pub(super) summary: SummaryStep,
    pub(super) notice: Option<Notice>,
    /// Where `q` leads: out of mccboard, or back to the dashboard that opened the setup.
    pub(super) back: Show,
}

impl Setup {
    /// The setup of `core`'s home in `language`, on a machine running `os`, at its first
    /// step. Every step reads what is already done. `q` quits mccboard.
    pub fn new(core: Core, language: Language, os: OsKind) -> Setup {
        Setup {
            claude_code: ClaudeCodeStep::load(&core, os),
            instances: InstancesStep::load(&core),
            accounts: AccountsStep::load(&core),
            sharing: SharingStep::load(&core),
            drift: DriftStep::load(&core),
            launchers: LaunchersStep::load(&core),
            preferences: PreferencesStep::load(&core),
            summary: SummaryStep::load(&core),
            core,
            language,
            step: StepId::ClaudeCode,
            notice: None,
            back: Show::Quit,
        }
    }

    /// The setup whose `q` leads to `back`: the dashboard, when the dashboard opened it.
    pub fn returning_to(self, back: Show) -> Setup {
        Setup { back, ..self }
    }

    pub fn texts(&self) -> Texts {
        Texts::new(self.language)
    }

    /// The step shown.
    pub fn step(&self) -> StepId {
        self.step
    }

    /// Whether the work of `step` is done. `None` for the summary, a step with no work of its own.
    pub fn done(&self, step: StepId) -> Option<bool> {
        Some(match step {
            StepId::ClaudeCode => self.claude_code.done(),
            StepId::Instances => self.instances.done(),
            StepId::Accounts => self.accounts.done(),
            StepId::Sharing => self.sharing.done(),
            StepId::Drift => self.drift.done(),
            StepId::Launchers => self.launchers.done(),
            StepId::Preferences => self.preferences.done(),
            StepId::Summary => return None,
        })
    }

    /// Whether the keys belong to the step shown: a field is typed, or a question waits for
    /// its answer. Then they don't move between steps or quit.
    pub fn holds_keys(&self) -> bool {
        match self.step {
            StepId::ClaudeCode => self.claude_code.confirm,
            StepId::Accounts => self.accounts.typing(),
            StepId::Drift => self.drift.confirm,
            _ => false,
        }
    }

    /// Whether a repair runs: the setup waits for it, and every key but Ctrl+C waits too.
    pub fn busy(&self) -> bool {
        self.drift.repairing.is_some()
    }

    /// Handles a key, and returns what the runner must do, if anything.
    pub fn key(&mut self, key: KeyEvent) -> Option<Request> {
        if self.busy() {
            return None;
        }
        if !self.holds_keys() {
            let to = match key.code {
                KeyCode::Char('q') => return Some(Request::Quit),
                KeyCode::Right | KeyCode::Tab => self.step.next(),
                KeyCode::Left | KeyCode::BackTab => self.step.previous(),
                KeyCode::Char(digit @ '1'..='8') => {
                    StepId::ALL.get(digit as usize - '1' as usize).copied()
                }
                _ => None,
            };
            if let Some(step) = to {
                self.go(step);
                return None;
            }
        }
        let action = match self.step {
            StepId::ClaudeCode => self.claude_code.key(key.code),
            StepId::Instances => self.instances.key(&self.core, key.code),
            StepId::Accounts => self.accounts.key(&self.core, key),
            StepId::Sharing => self.sharing.key(&self.core, key.code),
            StepId::Drift => self.drift.key(key.code),
            StepId::Launchers => self.launchers.key(&self.core, key.code),
            StepId::Preferences => self.preferences.key(&self.core, self.language, key.code),
            StepId::Summary => self.summary.key(key.code),
        };
        self.act(action)
    }

    /// Handles what the runner did.
    pub fn message(&mut self, message: Message) {
        let notice = match message {
            Message::Installed(result) => self.claude_code.installed(result),
            Message::LoggedIn { config_dir, result } => self.accounts.logged_in(config_dir, result),
            Message::Added(result) => self.accounts.added(result),
            Message::Repaired { config_dir, result } => self.drift.repaired(config_dir, result),
        };
        self.notice = Some(notice);
        self.reload();
    }

    fn act(&mut self, action: Action) -> Option<Request> {
        match action {
            Action::None => None,
            Action::Notice(notice) => {
                self.notice = Some(notice);
                self.reload();
                None
            }
            Action::Request(request) => {
                if let Request::Repair(dir) = &request {
                    self.notice = Some(Notice::Repairing(dir.clone()));
                }
                Some(request)
            }
            Action::Next => {
                if let Some(next) = self.step.next() {
                    self.go(next);
                }
                None
            }
            Action::Language(language) => {
                self.language = language;
                self.notice = Some(match self.core.set_language(language) {
                    Ok(()) => Notice::LanguageSet,
                    Err(error) => Notice::PreferenceNotSaved(Failure::from(&error)),
                });
                None
            }
        }
    }

    /// Shows `step`, as it is now.
    fn go(&mut self, step: StepId) {
        self.step = step;
        self.notice = None;
        match step {
            // Looking for Claude Code runs `claude --version`: only when its step shows.
            StepId::ClaudeCode => self.claude_code.reload(&self.core),
            _ => self.reload(),
        }
    }

    /// Reads every step again, but Claude Code's.
    fn reload(&mut self) {
        let core = &self.core;
        self.instances.reload(core);
        self.accounts.reload(core);
        self.sharing.reload(core);
        self.drift.reload(core);
        self.launchers.reload(core);
        self.preferences.reload(core);
        self.summary.reload(core);
    }

    /// `path` as shown: under the home, starting with `~`.
    pub fn shown_path(&self, path: &Path) -> String {
        crate::tui::shown_path(self.core.home(), path)
    }

    /// The notice in the language in use.
    pub fn notice_text(&self) -> Option<String> {
        Some(self.describe(self.notice.as_ref()?))
    }

    fn describe(&self, notice: &Notice) -> String {
        let texts = self.texts();
        let path = |dir: &Path| self.shown_path(dir);
        let paths = |dirs: &[PathBuf]| {
            dirs.iter()
                .map(|dir| path(dir))
                .collect::<Vec<_>>()
                .join(", ")
        };
        match notice {
            Notice::InstallCancelled => texts.install_cancelled().to_owned(),
            Notice::Installed => texts.installed().to_owned(),
            Notice::InstallerFoundNothing => {
                texts.installer_found_nothing(&path(&self.core.home().join(".local").join("bin")))
            }
            Notice::InstallFailed(failure) => texts.install_failed(failure),
            Notice::Registered(dirs) => texts.registered(&paths(dirs), dirs.len()),
            Notice::RegisterFailed {
                path: dir,
                failure,
                registered,
            } => texts.register_failed(&path(dir), failure, &paths(registered)),
            Notice::LabelRefused(failure) => texts.failure(failure),
            Notice::LoggedIn {
                path: dir,
                label,
                others,
            } => texts.logged_in_as(&path(dir), label, &paths(others)),
            Notice::NotLoggedIn(dir) => texts.still_not_logged_in(&path(dir)),
            Notice::LoginFailed { path: dir, failure } => texts.login_failed(&path(dir), failure),
            Notice::Added {
                path: dir,
                login,
                failed,
            } => texts.added(&path(dir), &self.describe(login), *failed),
            Notice::AddFailed(failure) => texts.add_failed(failure),
            Notice::Shared { item, answer } => texts.shared(item, *answer),
            Notice::ShareFailed(failure) => texts.failure(failure),
            Notice::RepairCancelled => texts.repair_cancelled().to_owned(),
            Notice::Repairing(dir) => texts.repairing(&path(dir)),
            Notice::Repaired(report) if report.items.is_empty() => {
                texts.nothing_to_repair(&path(&report.config_dir))
            }
            Notice::Repaired(report) => {
                texts.repaired(&repair_outcome(&path(&report.config_dir), report))
            }
            Notice::RepairFailed { path: dir, failure } => texts.repair_failed(&path(dir), failure),
            Notice::LaunchersSynced(sync) => texts.launchers_synced(sync),
            Notice::LaunchersFailed(failure) => texts.launchers_failed(failure),
            Notice::LanguageSet => texts.language_set(),
            Notice::Renewal(on) => texts.renewal_set(*on).to_owned(),
            Notice::PreferenceNotSaved(failure) => texts.preference_not_saved(failure),
        }
    }
}

/// What a repair did, as the dashboard tells it too.
fn repair_outcome(path: &str, report: &RepairReport) -> texts::RepairOutcome {
    let failures: Vec<&str> = report
        .items
        .iter()
        .filter_map(|item| match &item.outcome {
            RepairOutcome::Failed { reason } => Some(reason.as_str()),
            RepairOutcome::Done { .. } => None,
        })
        .collect();
    texts::RepairOutcome {
        path: path.to_owned(),
        done: report.items.len() - failures.len(),
        failed: failures.len(),
        first_failure: failures.first().map(|reason| (*reason).to_owned()),
        left: report.drift.drift_count,
        logged_out: report.items.iter().any(|item| item.logged_out),
    }
}
