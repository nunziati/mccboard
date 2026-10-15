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
mod backup;
mod claude_files;
mod config;
mod desktop_app;
mod doctor;
mod environment;
mod error;
mod instance;
mod links;
mod paths;
mod registry;
mod repair;
mod scan;
mod session_guard;
mod sharing;
mod system;
#[cfg(feature = "testing")]
pub mod testing;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use account::{Account, AccountUpdate, AccountUuid, Color, ObservedAccount};
pub use doctor::{Doctor, DriftItem, InstanceDrift, Problem, Repair};
pub use environment::{Environment, SystemEnvironment};
pub use error::Error;
pub use instance::{InstanceKind, LoginState, Observation, Warning};
pub use paths::Paths;
pub use registry::{AccountStatus, InstanceStatus, Registration, Registry};
pub use repair::{RepairOutcome, RepairReport, RepairedItem};
pub use scan::Candidate;
pub use session_guard::{ClaudeProcess, ProcessConfigDir};
pub use sharing::{SharingAnswer, SharingQuestion};

/// The entry point to every core operation, bound to one home directory on the local machine.
#[derive(Clone)]
pub struct Core {
    home: PathBuf,
    env: Arc<dyn Environment>,
}

impl Core {
    /// A core working in `home` (the user's home directory, or a temporary one in tests).
    pub fn new(home: impl Into<PathBuf>, env: impl Environment + 'static) -> Self {
        Self {
            home: home.into(),
            env: Arc::new(env),
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
    pub fn repair(&self, config_dir: &Path, item: Option<&str>) -> Result<RepairReport, Error> {
        repair::repair(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            config_dir,
            item,
        )
    }
}
