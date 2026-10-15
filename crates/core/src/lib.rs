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
mod launcher;
mod links;
mod paths;
mod registry;
mod scan;
mod sharing;
#[cfg(feature = "testing")]
pub mod testing;
mod usage;
#[cfg(windows)]
mod windows_user_path;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use account::{Account, AccountUpdate, AccountUuid, Color, ObservedAccount};
pub use doctor::{Doctor, DriftItem, InstanceDrift, Problem, Repair};
pub use environment::{EndpointReply, Environment, OsKind, SystemEnvironment};
pub use error::Error;
pub use instance::{InstanceKind, LoginState, Observation, Warning};
pub use launcher::{
    InstanceLauncher, Launcher, LauncherPlace, LauncherState, LauncherStatus, LauncherSync,
    UserPathChange,
};
pub use paths::Paths;
pub use registry::{AccountStatus, InstanceStatus, Registration, Registry};
pub use scan::Candidate;
pub use sharing::{SharingAnswer, SharingQuestion};
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
    /// but `config.toml`; the account keeps its label and color.
    pub fn unregister(&self, path: &Path) -> Result<(), Error> {
        registry::unregister(&self.home, &self.paths().config_file(), path)?;
        self.sync_launchers()?;
        Ok(())
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
}
