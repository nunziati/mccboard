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
mod claude_files;
mod config;
mod doctor;
mod environment;
mod error;
mod instance;
mod language;
mod links;
mod paths;
mod registry;
mod scan;
mod sharing;
#[cfg(feature = "testing")]
pub mod testing;
mod tray;
mod usage;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use account::{Account, AccountUpdate, AccountUuid, Color, ObservedAccount};
pub use doctor::{Doctor, DriftItem, InstanceDrift, Problem, Repair};
pub use environment::{EndpointReply, Environment, SystemEnvironment};
pub use error::Error;
pub use instance::{InstanceKind, LoginState, Observation, Warning};
pub use language::Language;
pub use paths::Paths;
pub use registry::{AccountStatus, InstanceStatus, Registration, Registry};
pub use scan::Candidate;
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
    pub fn register(&self, path: &Path) -> Result<Registration, Error> {
        registry::register(
            &self.home,
            &self.paths().config_file(),
            self.env.as_ref(),
            path,
        )
    }

    /// Forgets a registered instance. Nothing on disk changes but `config.toml`; the account
    /// keeps its label and color.
    pub fn unregister(&self, path: &Path) -> Result<(), Error> {
        registry::unregister(&self.home, &self.paths().config_file(), path)
    }

    /// Changes an account's label, color or launcher name override.
    pub fn update_account(
        &self,
        uuid: &AccountUuid,
        update: AccountUpdate,
    ) -> Result<Account, Error> {
        registry::update_account(&self.paths().config_file(), uuid, update)
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
    pub fn doctor(&self) -> Result<Doctor, Error> {
        doctor::doctor(&self.home, &self.paths().config_file())
    }

    /// The drift of the instance in `config_dir` (absolute, or starting with `~`), registered
    /// or not. Reads only; changes nothing.
    pub fn drift(&self, config_dir: &Path) -> Result<InstanceDrift, Error> {
        let config_dir = paths::resolve(&self.home, config_dir)?;
        let answers = config::ConfigFile::load(&self.paths().config_file())?
            .config
            .sharing;
        Ok(doctor::drift(&self.home, &config_dir, &answers))
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
}
