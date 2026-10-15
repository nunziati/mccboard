//! mccboard's core: domain, data sources, setup operations and token rules.
//!
//! The CLI and the Tauri app are thin front ends over [`Core`]; all behavior lives here.
//! The core never depends on Tauri (ADR 0004). It reaches everything outside the filesystem
//! through one port, [`Environment`], and takes the home directory as a parameter, so tests
//! run against temporary homes with [`testing::FakeEnvironment`].
//!
//! Token rules (AGENTS.md, ADR 0002): credentials are read-only, and no token ever appears in
//! anything this crate returns, prints or persists.

mod account;
mod add;
mod alignment;
mod backup;
mod claude_cli;
mod claude_files;
mod config;
mod desktop_app;
mod doctor;
mod environment;
mod error;
mod instance;
mod language;
mod launcher;
mod links;
mod paths;
mod registry;
mod remove;
mod repair;
mod scan;
mod session_guard;
mod sharing;
mod system;
#[cfg(feature = "testing")]
pub mod testing;
mod tray;
mod usage;
#[cfg(windows)]
mod windows_user_path;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use account::{Account, AccountUpdate, AccountUuid, Color, ObservedAccount};
pub use add::{AddReport, AddRequest, LoginReport, LoginRun, LoginWarning};
pub use alignment::GLOBAL_CONFIG_KEYS;
pub use doctor::{Doctor, DriftItem, InstanceDrift, Problem, Repair};
pub use environment::{
    ClaudeInvocation, ClaudeRun, EndpointReply, Environment, LoginExit, LoginInvocation,
    LoginTerminal, OsKind, SystemEnvironment,
};
pub use error::Error;
pub use instance::{InstanceKind, LoginState, Observation, Warning};
pub use language::Language;
pub use launcher::{
    InstanceLauncher, Launcher, LauncherPlace, LauncherState, LauncherStatus, LauncherSync,
    UserPathChange,
};
pub use paths::Paths;
pub use registry::{AccountStatus, InstanceStatus, Registration, Registry};
pub use remove::{FolderOutcome, Removal, RemoveLevel};
pub use repair::{RepairOutcome, RepairReport, RepairedItem};
pub use scan::Candidate;
pub use session_guard::{ClaudeProcess, ProcessConfigDir};
pub use sharing::{SharingAnswer, SharingQuestion};
pub use tray::{
    LanguageChoice, TOOLTIP_MAX_UTF16, TrayAccount, TrayIcon, TrayMenu, TrayView, TrayWindow,
    WindowName,
};
pub use usage::{
    AccessToken, AccountUsage, ModelWindow, UsageFailure, UsageReading, UsageReport, UsageSource,
    UsageStatus, UsageWarning, UsageWindow,
};

/// The entry point to every core operation, bound to one home directory on the local machine.
#[derive(Clone)]
pub struct Core {
    home: PathBuf,
    env: Arc<dyn Environment>,
    /// Serializes usage refreshes within this process (rate limits are per account).
    usage_lock: Arc<Mutex<()>>,
}

impl Core {
    /// A core working in `home` (the user's home directory, or a temporary one in tests).
    pub fn new(home: impl Into<PathBuf>, env: impl Environment + 'static) -> Self {
        Self {
            home: home.into(),
            env: Arc::new(env),
            usage_lock: Arc::default(),
        }
    }

    /// The home directory this core works in.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where mccboard keeps its own files in this home.
    pub fn paths(&self) -> Paths {
        Paths::new(&self.home)
    }

    /// Finds instance candidates in the home: `~/.claude` (the primary instance) and every
    /// `~/.claude-*` directory (linked instances), primary first. Scanning only proposes
    /// instances: it registers nothing and writes nothing. Each candidate says whether it is
    /// registered already.
    pub fn scan(&self) -> Result<Vec<Candidate>, Error> {
        let registered = registry::registered_dirs(&self.paths().config_file())?;
        scan::scan(&self.home, self.env.as_ref(), &registered)
    }

    /// The registered accounts and instances, each instance observed now. Reads
    /// `config.toml` and writes nothing.
    pub fn registry(&self) -> Result<Registry, Error> {
        registry::registry(&self.home, &self.paths().config_file(), self.env.as_ref())
    }

    /// Registers the instance whose config directory is `path` (absolute, or starting with
    /// `~`), at any location, logged in or not. The instance is recorded with the account it
    /// is logged in with; an account seen for the first time is added with a suggested label
    /// and color. Registering a registered instance again is harmless and accepts the account
    /// it is now logged in with.
    ///
    /// A linked instance gets its launcher (see [`Core::sync_launchers`]). If the launchers
    /// can't be written, the error says so; the registration is saved all the same.
    pub fn register(&self, path: &Path) -> Result<Registration, Error> {
        let registration = registry::register(
            &self.home,
            &self.paths().config_file(),
            self.env.as_ref(),
            path,
        )?;
        self.sync_launchers()?;
        Ok(registration)
    }

    /// Forgets a registered instance and removes its launcher. Nothing else on disk changes
    /// but `config.toml`; the account keeps its label and color. The same as
    /// [`Core::remove`] with [`RemoveLevel::FromMccboard`]: the primary instance is refused.
    pub fn unregister(&self, path: &Path) -> Result<(), Error> {
        self.remove(path, RemoveLevel::FromMccboard).map(drop)
    }

    /// Removes the registered linked instance in `path` (absolute, or starting with `~`):
    /// - [`RemoveLevel::FromMccboard`] forgets it and removes its launcher; its folder stays
    ///   as it is;
    /// - [`RemoveLevel::WithFolder`] also deletes its folder, and so the account's login on
    ///   this machine. Front ends confirm this with the user first. It refuses while a Claude
    ///   Code process uses the folder ([`Error::InstanceInUse`]), and for a folder that holds
    ///   the home, the primary instance or mccboard's files, or is a link
    ///   ([`Error::FolderNotDeletable`]). Links inside are removed, never followed, so the
    ///   primary instance's items stay intact.
    ///
    /// The primary instance is never removed ([`Error::PrimaryInstance`]). The account keeps
    /// its label and color. If the launchers can't be written, the error says so; the
    /// instance is removed all the same.
    pub fn remove(&self, path: &Path, level: RemoveLevel) -> Result<Removal, Error> {
        remove::remove(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            path,
            level,
            &|| self.sync_launchers(),
        )
    }

    /// Changes an account's label, color or launcher name override. The launchers of its
    /// linked instances follow the new name.
    pub fn update_account(
        &self,
        uuid: &AccountUuid,
        update: AccountUpdate,
    ) -> Result<Account, Error> {
        let account = registry::update_account(&self.paths().config_file(), uuid, update)?;
        self.sync_launchers()?;
        Ok(account)
    }

    /// The launchers as they are, against what the registered linked instances need: one per
    /// instance (ok, missing or stale), plus launchers mccboard made that nothing needs any
    /// more. Reads only. The doctor reports a missing or stale launcher as drift.
    pub fn launchers(&self) -> Result<LauncherStatus, Error> {
        launcher::status(&self.home, &self.paths(), self.env.as_ref())
    }

    /// Makes the launchers match the registered linked instances: creates the missing ones,
    /// rewrites stale ones, removes the ones nothing needs, and on Windows puts the bin folder
    /// on the user PATH once. Touches only mccboard's block of the shell rc file (Linux) or
    /// mccboard's bin folder (Windows). Running it again changes nothing.
    pub fn sync_launchers(&self) -> Result<LauncherSync, Error> {
        launcher::sync(&self.home, &self.paths(), self.env.as_ref())
    }

    /// Observes the account and login state of the instance whose config directory is
    /// `config_dir`. Missing or unreadable files never fail: they show in the observation.
    pub fn observe(&self, config_dir: &Path) -> Observation {
        instance::observe(&self.home, config_dir, self.env.as_ref())
    }

    /// Whether `config_dir` is the primary instance's (Claude Code's default) or a linked one.
    pub fn kind_of(&self, config_dir: &Path) -> InstanceKind {
        InstanceKind::of(&self.home, config_dir)
    }

    /// The drift of every registered instance against the sharing table (ADR 0005), each
    /// item explained with the repair that would fix it, plus the unknown items of the
    /// primary instance waiting for an answer. Reads only; changes nothing.
    ///
    /// A registered linked instance whose launcher is missing or stale has a `launcher:<name>`
    /// item, repaired by [`Core::sync_launchers`]. If the launchers can't be read, the report
    /// leaves them out and [`Core::launchers`] says why.
    pub fn doctor(&self) -> Result<Doctor, Error> {
        let launchers = self.launchers().ok();
        doctor::doctor(&self.home, &self.paths().config_file(), launchers.as_ref())
    }

    /// The drift of the instance in `config_dir` (absolute, or starting with `~`), registered
    /// or not. Reads only; changes nothing.
    pub fn drift(&self, config_dir: &Path) -> Result<InstanceDrift, Error> {
        let config_dir = paths::resolve(&self.home, config_dir)?;
        let answers = config::ConfigFile::load(&self.paths().config_file())?
            .config
            .sharing;
        let launchers = self.launchers().ok();
        Ok(doctor::drift(
            &self.home,
            &config_dir,
            &answers,
            launchers.as_ref(),
        ))
    }

    /// Unknown top-level items of the primary instance the user hasn't answered about yet:
    /// "share `gsd-core`?". Each is asked once: see [`Core::answer_sharing`].
    pub fn sharing_questions(&self) -> Result<Vec<SharingQuestion>, Error> {
        let answers = config::ConfigFile::load(&self.paths().config_file())?
            .config
            .sharing;
        Ok(sharing::questions(
            &doctor::primary_dir(&self.home),
            &answers,
        ))
    }

    /// Remembers in `config.toml` whether the unknown item `item` (a name in the primary
    /// instance's folder) is linked or kept private, for every instance. Answering again
    /// changes the answer. Items in the sharing table take no answer.
    pub fn answer_sharing(&self, item: &str, answer: SharingAnswer) -> Result<(), Error> {
        sharing::validate_item(item)?;
        let mut file = config::ConfigFile::load(&self.paths().config_file())?;
        file.set_sharing(item, answer);
        file.save()
    }

    /// Every registered account's last known usage reading, without calling the endpoint.
    /// Windows whose reset time has passed read as zero.
    pub fn usage(&self) -> Result<UsageReport, Error> {
        usage::report(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            &self.usage_lock,
            false,
        )
    }

    /// Refreshes usage, then reports it like [`Core::usage`]. The endpoint is called once per
    /// account, with a token from one of its registered instances, and only where the rate
    /// limits allow it: a manual refresh follows the same rules as a periodic one. A failed
    /// call keeps the last reading, marked stale.
    pub fn refresh_usage(&self) -> Result<UsageReport, Error> {
        usage::report(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            &self.usage_lock,
            true,
        )
    }

    /// The Claude Code processes using the instance in `config_dir` (absolute, or starting
    /// with `~`). While there are any, operations that change the instance refuse.
    pub fn processes_using(&self, config_dir: &Path) -> Result<Vec<ClaudeProcess>, Error> {
        let config_dir = paths::resolve(&self.home, config_dir)?;
        Ok(session_guard::processes_using(
            &self.home,
            self.env.as_ref(),
            &config_dir,
        ))
    }

    /// Repairs the drift of the registered instance in `config_dir`: every item, or only the
    /// one whose id is `item` ([`DriftItem::id`]). Refuses before changing anything while a
    /// Claude Code process uses the instance ([`Error::InstanceInUse`]), when a file link is
    /// needed and Windows Developer Mode is off ([`Error::DeveloperModeOff`]), or when a link
    /// would land in the Claude desktop app's data folder. Real files and folders a link
    /// replaces are moved to a backup under `~/.mccboard/state/backups/` first. Repairing
    /// again changes nothing; the primary instance is never relinked.
    ///
    /// Aligned items are brought in line with the primary instance: a Global config key
    /// (`config:<key>`) by writing only that key into the instance's `.claude.json`, a
    /// marketplace or plugin (`marketplace:<name>`, `plugin:<id>`) by running Claude Code's
    /// own `claude plugin` commands in the instance. A failed install is reported in its item
    /// and doesn't stop the others.
    ///
    /// A missing or stale launcher (`launcher:<name>`) is repaired with
    /// [`Core::sync_launchers`]; launchers live outside the instance's folder, so repairing
    /// only them doesn't wait for its sessions to end.
    pub fn repair(&self, config_dir: &Path, item: Option<&str>) -> Result<RepairReport, Error> {
        repair::repair(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            config_dir,
            item,
            &|| self.launchers().ok(),
            &|| self.sync_launchers(),
        )
    }
    /// The app's language as chosen in `config.toml`; `None` when the user hasn't chosen one
    /// (front ends then follow the system: see [`Language::from_locale`]).
    pub fn language(&self) -> Result<Option<Language>, Error> {
        Ok(config::ConfigFile::load(&self.paths().config_file())?
            .config
            .language)
    }

    /// Saves the app's language in `config.toml`, keeping everything else in the file.
    pub fn set_language(&self, language: Language) -> Result<(), Error> {
        let mut file = config::ConfigFile::load(&self.paths().config_file())?;
        file.set_language(language);
        file.save()
    }

    /// What the tray shows for `report` (from [`Core::usage`] or [`Core::refresh_usage`]),
    /// as of now: the worst account's percentage and color for the icon, a tooltip that fits
    /// the Windows tray, every account's windows as menu lines, and "no instances" when none
    /// is registered. Texts are in `language`, with reset times shown in `tz`. Windows whose
    /// reset passed since the report read as zero. Reads `config.toml`; calls nothing.
    pub fn tray_view(
        &self,
        report: &UsageReport,
        language: Language,
        tz: &jiff::tz::TimeZone,
    ) -> Result<TrayView, Error> {
        let config = config::ConfigFile::load(&self.paths().config_file())?.config;
        Ok(tray::view(
            report,
            config.instances.is_empty(),
            language,
            tz,
            self.env.now(),
        ))
    }

    /// Aligns the linked instance in `config_dir` (absolute, or starting with `~`) with the
    /// primary instance, registered or not: its Global config keys, then its marketplaces and
    /// plugins, as [`Core::repair`] does for those items, and nothing else (no links, no
    /// launcher). For the add flow, before the instance is logged in and registered. Refuses
    /// while a Claude Code process uses the instance; a failed install is reported in its item.
    /// Aligning the primary instance changes nothing.
    pub fn align(&self, config_dir: &Path) -> Result<RepairReport, Error> {
        repair::align(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            config_dir,
            &|| self.launchers().ok(),
            &|| self.sync_launchers(),
        )
    }
}
