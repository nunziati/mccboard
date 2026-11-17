//! The dashboard's state: what it shows and what each key does, with no terminal. Keys come
//! in through [`Dashboard::key`], what the workers did through [`Dashboard::message`]. What
//! must run elsewhere (a refresh, a repair) goes out as a [`Request`].
//!
//! Everything shown comes from the core: the tray's and the taskbar widget's view models for
//! usage, the registry, the doctor and the launchers for instances. Nothing is computed here.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use mccboard_core::{
    Core, Doctor, Error, InstanceStatus, Language, LauncherStatus, Registry, RepairOutcome,
    RepairReport, TaskbarWidget, TrayView, UsageReport, UsageWarning,
};

use crate::tui::texts::{self, Failure, Texts};

/// What the dashboard asks its runner to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Refresh usage now, within the rate limits.
    Refresh,
    /// Repair every drift item of the instance in this folder.
    Repair(PathBuf),
    Quit,
}

/// What the runner's workers tell the dashboard.
#[derive(Debug)]
pub enum Message {
    /// A usage refresh started.
    RefreshStarted,
    /// A usage refresh ended.
    Usage(Result<UsageReport, Error>),
    /// A repair ended.
    Repaired {
        config_dir: PathBuf,
        result: Result<RepairReport, Error>,
    },
}

/// A repair waiting for the user's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub config_dir: PathBuf,
    /// The ids of the drift items it would repair (`link:settings.json`).
    pub items: Vec<String>,
}

/// The line under the instances: what the last action did, or what went wrong. Kept as data
/// and written at draw time: it follows a language switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Refreshed,
    /// No account could be called yet: the rate limits allow the next call then.
    NextCallAt(Timestamp),
    RefreshFailed(Failure),
    UsageWarning(UsageWarning),
    Repairing(PathBuf),
    RepairCancelled,
    NothingToRepair(PathBuf),
    Repaired {
        config_dir: PathBuf,
        done: usize,
        failed: usize,
        first_failure: Option<String>,
        left: usize,
        logged_out: bool,
    },
    RepairFailed {
        config_dir: PathBuf,
        failure: Failure,
    },
    LanguageSet,
    LanguageNotSaved(Failure),
}

impl Notice {
    /// Whether it tells of something that went wrong.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Notice::RefreshFailed(_)
                | Notice::UsageWarning(_)
                | Notice::RepairFailed { .. }
                | Notice::LanguageNotSaved(_)
        ) || matches!(self, Notice::Repaired { failed, .. } if *failed > 0)
    }
}

/// The dashboard's state.
pub struct Dashboard {
    pub(super) core: Core,
    pub(super) tz: TimeZone,
    pub(super) language: Language,
    /// The last usage report: the saved readings at start, then each refresh's.
    pub(super) usage: Result<UsageReport, Error>,
    pub(super) tray: TrayView,
    pub(super) widget: TaskbarWidget,
    /// Counts the times the usage views were computed: the runner times the next redraw of the
    /// times to reset from the last one.
    pub(super) views: u64,
    pub(super) registry: Result<Registry, Failure>,
    /// `None` when the drift can't be read: the instances then show no drift count.
    pub(super) doctor: Option<Doctor>,
    /// `None` when the launchers can't be read.
    pub(super) launchers: Option<LauncherStatus>,
    /// The selected instance, an index into the registry's instances.
    pub(super) selected: usize,
    pub(super) confirm: Option<Confirm>,
    pub(super) refreshing: bool,
    /// The user asked for a refresh that hasn't started yet.
    asked_refresh: bool,
    /// The refresh under way started after the user asked for one: its outcome is told.
    telling: bool,
    pub(super) repairing: Option<PathBuf>,
    pub(super) notice: Option<Notice>,
}

impl Dashboard {
    /// The dashboard of `core`'s home in `language`, with reset times shown in `tz`. Shows the
    /// last known readings at once, without calling the endpoint.
    pub fn new(core: Core, tz: TimeZone, language: Language) -> Dashboard {
        let usage = core.usage();
        let (tray, widget) = views(&core, &usage, language, &tz);
        let mut dashboard = Dashboard {
            core,
            tz,
            language,
            usage,
            tray,
            widget,
            views: 1,
            registry: Ok(Registry {
                accounts: Vec::new(),
                instances: Vec::new(),
            }),
            doctor: None,
            launchers: None,
            selected: 0,
            confirm: None,
            refreshing: false,
            asked_refresh: false,
            telling: false,
            repairing: None,
            notice: None,
        };
        dashboard.reload_instances();
        dashboard
    }

    pub fn texts(&self) -> Texts {
        Texts::new(self.language)
    }

    /// How many times the usage views were computed.
    pub fn views(&self) -> u64 {
        self.views
    }

    /// How long until a time to reset reads differently, from the last time the views were
    /// computed.
    pub fn redraw_in(&self) -> std::time::Duration {
        self.widget.redraw_in
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

    /// Handles a key, and returns what the runner must do, if anything.
    pub fn key(&mut self, key: KeyEvent) -> Option<Request> {
        if self.confirm.is_some() {
            return self.answer(key.code);
        }
        match key.code {
            KeyCode::Char('q') => Some(Request::Quit),
            KeyCode::Char('r') => {
                self.refreshing = true;
                self.asked_refresh = true;
                Some(Request::Refresh)
            }
            KeyCode::Char('p') => {
                self.ask_repair();
                None
            }
            KeyCode::Char('l') => {
                self.switch_language();
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let last = self.instances().len().saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
                None
            }
            _ => None,
        }
    }

    /// Handles what a worker sent.
    pub fn message(&mut self, message: Message) {
        match message {
            Message::RefreshStarted => {
                self.refreshing = true;
                // The first refresh to start after the user asked: its outcome is the one to
                // tell, not that of a refresh already under way.
                self.telling = std::mem::take(&mut self.asked_refresh);
            }
            Message::Usage(result) => self.refreshed(result),
            Message::Repaired { config_dir, result } => self.repaired(config_dir, result),
        }
    }

    /// Computes the usage views again, as of now: a time to reset changed.
    pub fn tick(&mut self) {
        self.compute_views();
    }

    /// Answers the repair question.
    fn answer(&mut self, code: KeyCode) -> Option<Request> {
        match code {
            KeyCode::Char('y' | 'Y' | 's' | 'S') | KeyCode::Enter => {
                let confirm = self.confirm.take()?;
                self.repairing = Some(confirm.config_dir.clone());
                self.notice = Some(Notice::Repairing(confirm.config_dir.clone()));
                Some(Request::Repair(confirm.config_dir))
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                self.confirm = None;
                self.notice = Some(Notice::RepairCancelled);
                None
            }
            _ => None,
        }
    }

    /// Asks to repair the selected instance, when it has drift and no repair is running.
    fn ask_repair(&mut self) {
        if self.repairing.is_some() {
            return;
        }
        let Some(instance) = self.selected_instance() else {
            return;
        };
        let config_dir = instance.config_dir.clone();
        // Without the doctor's report the question lists nothing, and the repair says what it
        // did.
        let items = match self.drift_of(&config_dir) {
            Some(items) if items.is_empty() => {
                self.notice = Some(Notice::NothingToRepair(config_dir));
                return;
            }
            Some(items) => items,
            None => Vec::new(),
        };
        self.confirm = Some(Confirm { config_dir, items });
    }

    /// The ids of the drift items of the instance in `config_dir`, or `None` when unknown.
    fn drift_of(&self, config_dir: &Path) -> Option<Vec<String>> {
        let doctor = self.doctor.as_ref()?;
        let drift = doctor
            .instances
            .iter()
            .find(|drift| drift.config_dir == config_dir)?;
        Some(drift.items.iter().map(|item| item.id.clone()).collect())
    }

    /// Switches to the next language and saves it in `config.toml`, as the tray does.
    fn switch_language(&mut self) {
        let all = Language::ALL;
        let at = all.iter().position(|l| *l == self.language).unwrap_or(0);
        self.language = all[(at + 1) % all.len()];
        self.notice = Some(match self.core.set_language(self.language) {
            Ok(()) => Notice::LanguageSet,
            Err(error) => Notice::LanguageNotSaved(Failure::from(&error)),
        });
        self.compute_views();
    }

    fn refreshed(&mut self, result: Result<UsageReport, Error>) {
        self.refreshing = false;
        let asked = std::mem::take(&mut self.telling);
        let notice = match &result {
            Ok(report) => match report.warnings.first() {
                Some(warning) => Some(Notice::UsageWarning(*warning)),
                None if asked => outcome(report),
                None => None,
            },
            Err(error) if asked => Some(Notice::RefreshFailed(Failure::from(error))),
            Err(_) => None,
        };
        if notice.is_some() {
            self.notice = notice;
        }
        self.usage = result;
        self.compute_views();
        // Claude Code may have renewed a token, or an instance logged in or out.
        self.reload_instances();
    }

    fn repaired(&mut self, config_dir: PathBuf, result: Result<RepairReport, Error>) {
        self.repairing = None;
        self.notice = Some(match result {
            Err(error) => Notice::RepairFailed {
                config_dir,
                failure: Failure::from(&error),
            },
            Ok(report) if report.items.is_empty() => Notice::NothingToRepair(config_dir),
            Ok(report) => {
                let failures: Vec<&str> = report
                    .items
                    .iter()
                    .filter_map(|item| match &item.outcome {
                        RepairOutcome::Failed { reason } => Some(reason.as_str()),
                        RepairOutcome::Done { .. } => None,
                    })
                    .collect();
                Notice::Repaired {
                    config_dir,
                    done: report.items.len() - failures.len(),
                    failed: failures.len(),
                    first_failure: failures.first().map(|reason| (*reason).to_owned()),
                    left: report.drift.drift_count,
                    logged_out: report.items.iter().any(|item| item.logged_out),
                }
            }
        });
        self.reload_instances();
    }

    /// The tray's and the widget's views of the last report, in the language in use, as of
    /// now.
    fn compute_views(&mut self) {
        (self.tray, self.widget) = views(&self.core, &self.usage, self.language, &self.tz);
        self.views += 1;
    }

    /// Reads the instances again: the registry, their drift and their launchers.
    fn reload_instances(&mut self) {
        self.registry = self.core.registry().map_err(|error| Failure::from(&error));
        self.doctor = self.core.doctor().ok();
        self.launchers = self.core.launchers().ok();
        let last = self.instances().len().saturating_sub(1);
        self.selected = self.selected.min(last);
    }

    /// The instance's folder as shown: under the home, starting with `~`.
    pub fn shown_path(&self, path: &Path) -> String {
        shown_path(self.core.home(), path)
    }

    /// The notice in the language in use.
    pub fn notice_text(&self) -> Option<String> {
        let texts = self.texts();
        let path = |dir: &Path| self.shown_path(dir);
        Some(match self.notice.as_ref()? {
            Notice::Refreshed => texts.refreshed().to_owned(),
            Notice::NextCallAt(at) => texts.next_call_at(&texts.clock(*at, &self.tz)),
            Notice::RefreshFailed(failure) => texts.refresh_failed(failure),
            Notice::UsageWarning(warning) => texts.usage_warning(*warning).to_owned(),
            Notice::Repairing(dir) => texts.repairing(&path(dir)),
            Notice::RepairCancelled => texts.repair_cancelled().to_owned(),
            Notice::NothingToRepair(dir) => texts.nothing_to_repair(&path(dir)),
            Notice::Repaired {
                config_dir,
                done,
                failed,
                first_failure,
                left,
                logged_out,
            } => texts.repaired(&texts::RepairOutcome {
                path: path(config_dir),
                done: *done,
                failed: *failed,
                first_failure: first_failure.clone(),
                left: *left,
                logged_out: *logged_out,
            }),
            Notice::RepairFailed {
                config_dir,
                failure,
            } => texts.repair_failed(&path(config_dir), failure),
            Notice::LanguageSet => texts.language_set(),
            Notice::LanguageNotSaved(failure) => texts.language_not_saved(failure),
        })
    }
}

/// What the tray and the taskbar widget show for `usage`, as of now.
fn views(
    core: &Core,
    usage: &Result<UsageReport, Error>,
    language: Language,
    tz: &TimeZone,
) -> (TrayView, TaskbarWidget) {
    match usage {
        Ok(report) => (
            core.tray_view(report, language, tz)
                .unwrap_or_else(|error| TrayView::unavailable(&error, language)),
            core.taskbar_widget(report, language, tz)
                .unwrap_or_else(|_| TaskbarWidget::unavailable(language)),
        ),
        Err(error) => (
            TrayView::unavailable(error, language),
            TaskbarWidget::unavailable(language),
        ),
    }
}

/// What a refresh the user asked for did: refreshed when an account was called, else when the
/// rate limits allow the next call, if they held it back.
fn outcome(report: &UsageReport) -> Option<Notice> {
    if report.accounts.iter().any(|usage| usage.called) {
        return Some(Notice::Refreshed);
    }
    report
        .accounts
        .iter()
        .filter_map(|usage| usage.next_call_at)
        .min()
        .map(Notice::NextCallAt)
}

/// `path` as shown: `~/…` under `home`, else whole.
pub fn shown_path(home: &Path, path: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}
