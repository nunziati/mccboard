//! Adding a linked instance, and logging an instance in.
//!
//! Adding composes the other operations, in order:
//! 1. checks, before anything changes: the folder isn't the primary instance's (or a link to
//!    it), a file, a protected folder ([`crate::folder_guard`]: one holding the home, the
//!    primary or mccboard's files, one inside them, or a link) or in the Claude desktop app's
//!    data folder or a desktop profile's; it doesn't belong to an account with another label; file links can be
//!    made (Developer Mode on Windows) when one is missing (the doctor's link drift); no
//!    Claude Code session uses the folder;
//! 2. creates the folder (`~/.claude-<label>` by default);
//! 3. registers it, without an account until it is logged in: this creates its launcher;
//! 4. repairs its drift: the sharing table's links and the aligned items (Global config
//!    keys, plugins and marketplaces);
//! 5. opens Claude Code's official login through the environment port, unless the instance
//!    is logged in already (its credentials say so);
//! 6. observes the account and registers the instance again, under it.
//!
//! The label asked for waits in `config.toml`, on the instance's entry, until an account is
//! observed in it: a login that happens later (`mccboard login`, "Log in" in the app, the
//! instance registered again) gives it to the account all the same.
//!
//! Each step is idempotent, so running the add again after a failure at any step completes
//! what's missing. mccboard never reads or writes credentials here: Claude Code's login
//! writes them, and mccboard reads `oauthAccount` from `.claude.json` afterwards.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::account::{self, Account, AccountUuid};
use crate::claude_files;
use crate::config::ConfigFile;
use crate::instance::InstanceKind;
use crate::{
    Core, Error, InstanceStatus, LoginInvocation, LoginState, LoginTerminal, OsKind, RepairReport,
    SharingQuestion, desktop_app, doctor, folder_guard, paths, repair, session_guard,
};

/// What to add: a linked instance for the account the user calls `label`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddRequest {
    /// The account label ("B"). It names the default folder and, when the login brings an
    /// account mccboard didn't know, becomes that account's label.
    pub label: String,
    /// The instance's config directory (absolute, or starting with `~`); `None` for
    /// `~/.claude-<label>` (see [`Core::add_folder`]).
    pub config_dir: Option<PathBuf>,
    /// Whether to open the login; without it the add stops once the instance is set up and
    /// registered, and [`Core::login`] finishes it later.
    pub login: bool,
    /// Where the login opens: the front end decides (the app in a new window, the CLI in its
    /// own terminal).
    pub terminal: LoginTerminal,
}

impl AddRequest {
    /// Adds `label`'s instance in the default folder, logging in in `terminal`.
    pub fn new(label: impl Into<String>, terminal: LoginTerminal) -> Self {
        Self {
            label: label.into(),
            config_dir: None,
            login: true,
            terminal,
        }
    }
}

/// What adding an instance did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AddReport {
    pub config_dir: PathBuf,
    /// Whether the folder was created now (it may exist from an earlier run, or by hand).
    pub created: bool,
    /// The links and alignment made: a repair of the new instance. A failed item (a plugin
    /// that couldn't be installed) doesn't stop the add; running it again retries.
    pub setup: RepairReport,
    /// Unknown items of the primary instance nobody answered about yet: they aren't linked.
    /// Once answered ([`Core::answer_sharing`]), adding again links those answered "link".
    pub sharing_questions: Vec<SharingQuestion>,
    pub login: LoginReport,
}

/// What a login did, and the instance after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoginReport {
    pub config_dir: PathBuf,
    pub run: LoginRun,
    /// The instance as registered now.
    pub instance: InstanceStatus,
    /// The account the instance is logged in with, as registered; `None` when no account
    /// can be observed (the login was closed, or failed).
    pub account: Option<Account>,
    /// Whether the account was new to mccboard, and added now.
    pub new_account: bool,
    pub warnings: Vec<LoginWarning>,
}

/// Whether the login ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "run", rename_all = "snake_case")]
pub enum LoginRun {
    /// It ran and ended with `exit_code` (`None` when unknown). Only the account observed
    /// afterwards says whether it worked.
    Ran { exit_code: Option<i32> },
    /// Not opened: the instance was logged in already (an add run again), perhaps with an
    /// expired token, which Claude Code renews the next time it runs.
    AlreadyLoggedIn,
    /// Not opened: the add was asked not to.
    NotRequested,
}

/// Something the user should know after a login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoginWarning {
    /// The account logged in already has other instances on this machine: perhaps the wrong
    /// account was used in the browser. The instance is registered all the same.
    AccountHasOtherInstances {
        account: AccountUuid,
        /// The account's label, when registered.
        label: Option<String>,
        instances: Vec<PathBuf>,
    },
}

impl Core {
    /// The folder a linked instance for `label` gets by default: `~/.claude-<label>`, the
    /// label lowercased and reduced to ASCII letters, digits and dashes (`"Personal Max"` →
    /// `~/.claude-personal-max`). [`Error::NoDefaultFolder`] when nothing is left of it.
    pub fn add_folder(&self, label: &str) -> Result<PathBuf, Error> {
        let label = account::validate_label(label)?;
        let slug = account::slug(&label);
        if slug.is_empty() {
            return Err(Error::NoDefaultFolder { label });
        }
        Ok(self.home.join(format!(".claude-{slug}")))
    }

    /// Adds a linked instance for the account the user calls `request.label`: creates its
    /// folder, registers it (which creates its launcher), links and aligns it like
    /// [`Core::repair`], opens Claude Code's official login unless it is logged in already,
    /// then registers it under the account it logged in with. An account new to mccboard
    /// gets the label, when no other account has it. A login with an account that already
    /// has an instance here is registered with a warning.
    ///
    /// Refuses before changing anything for the primary instance's folder or a path leading
    /// there through links ([`Error::PrimaryInstance`]), a file, a folder holding the home, the
    /// primary instance or mccboard's files, one inside them, or a link
    /// ([`Error::FolderNotAddable`]), a folder in the Claude desktop app's data, a folder that
    /// belongs to an account with another label ([`Error::FolderHasAnotherAccount`]), when
    /// file links are needed and Windows Developer Mode is off, and while a Claude Code
    /// process uses the folder. Idempotent and resumable: after a failure at any step, adding
    /// again completes what's missing; an instance set up but not logged in is registered,
    /// with the label waiting for its account, and [`Core::login`] can finish it too.
    pub fn add(&self, request: &AddRequest) -> Result<AddReport, Error> {
        let label = account::validate_label(&request.label)?;
        let config_dir = match &request.config_dir {
            Some(path) => paths::resolve(&self.home, path)?,
            None => self.add_folder(&label)?,
        };
        self.check_addable(&config_dir)?;
        self.check_account(&config_dir, &label)?;

        let created = !config_dir.exists();
        fs::create_dir_all(&config_dir).map_err(|source| Error::FolderNotCreated {
            path: config_dir.clone(),
            source,
        })?;
        // The label waits in config.toml for the account, so a login that happens later (a
        // failed or skipped one, finished with `login`) still gives it.
        let registration = self.register_labelled(&config_dir, Some(&label))?;
        // An instance registered already keeps the spelling of its path in config.toml.
        let config_dir = registration.instance.config_dir;
        let setup = self.repair(&config_dir, None)?;
        let sharing_questions = self.sharing_questions()?;

        let run = if !request.login {
            LoginRun::NotRequested
        } else if self.logged_in_already(&config_dir) {
            LoginRun::AlreadyLoggedIn
        } else {
            self.open_login(&config_dir, request.terminal)?
        };
        let mut login = self.after_login(&config_dir, run)?;
        login.new_account |= registration.new_account.is_some();
        Ok(AddReport {
            config_dir,
            created,
            setup,
            sharing_questions,
            login,
        })
    }

    /// Opens Claude Code's official login in the registered instance in `config_dir`
    /// (absolute, or starting with `~`), whatever its login state, in `terminal`, and waits
    /// for it to end. Then registers the instance again, under the account it is now logged in
    /// with (giving it the label an add asked for, if one is waiting), warning when that
    /// account has other instances here. mccboard never sees the credentials: Claude Code
    /// writes them.
    ///
    /// Errors before opening anything when a linked instance's folder is gone
    /// ([`Error::InstanceFolderMissing`]), when `claude` is neither on the PATH nor in
    /// `.local/bin` of the home ([`Error::ClaudeNotInstalled`]), and when there is no such
    /// terminal ([`Error::LoginNeedsTerminal`]: no terminal for the current one, no terminal
    /// emulator on Linux for a new window). The primary instance's folder may be gone: Claude
    /// Code makes it again on its login, and until it does the report has no account.
    pub fn login(&self, config_dir: &Path, terminal: LoginTerminal) -> Result<LoginReport, Error> {
        let config_dir = paths::resolve(&self.home, config_dir)?;
        let config_dir = ConfigFile::load(&self.paths().config_file())?
            .config
            .instance(&config_dir)
            .map(|entry| entry.config_dir.clone())
            .ok_or(Error::NotRegistered { path: config_dir })?;
        let run = self.open_login(&config_dir, terminal)?;
        self.after_login(&config_dir, run)
    }

    /// Opens Claude Code's official login in the primary instance (`~/.claude`), registered or
    /// not, in `terminal`, and waits for it to end: the first login on a machine where Claude
    /// Code never ran. Its folder may be missing. mccboard never makes it (it belongs to
    /// Claude Code), and the login then runs in the home, where Claude Code makes it.
    ///
    /// Afterwards the primary instance is registered as [`Core::register`] does, when its
    /// folder exists and an account is logged in there. A registered one is registered again,
    /// as [`Core::login`] does. Otherwise nothing is registered, and the report has no account.
    ///
    /// Errors as [`Core::login`] does, and with [`Error::NotADirectory`] when `~/.claude` is a
    /// file.
    pub fn login_primary(&self, terminal: LoginTerminal) -> Result<LoginReport, Error> {
        let primary = doctor::primary_dir(&self.home);
        if primary.exists() && !primary.is_dir() {
            return Err(Error::NotADirectory { path: primary });
        }
        let registered = ConfigFile::load(&self.paths().config_file())?
            .config
            .instance(&primary)
            .is_some();
        let run = self.open_login(&primary, terminal)?;
        if registered || (primary.is_dir() && self.observe(&primary).account.is_some()) {
            return self.after_login(&primary, run);
        }
        self.unregistered_login(&primary, run)
    }

    /// Refuses a folder that belongs to an account with another label than `label`: the
    /// account it is registered with or, unregistered, the known account logged in there.
    /// Adding it for `label` would register one account's instance under another's name. Two
    /// labels whose default folders are the same (`"Work"`, `"work!"`) meet here too. A folder
    /// whose account mccboard doesn't know yet is fine: the account gets the label.
    fn check_account(&self, config_dir: &Path, label: &str) -> Result<(), Error> {
        let config = ConfigFile::load(&self.paths().config_file())?.config;
        let owner = config
            .instance(config_dir)
            .and_then(|entry| entry.account.clone())
            .or_else(|| {
                self.observe(config_dir)
                    .account
                    .map(|observed| observed.uuid)
            });
        match owner.as_ref().and_then(|uuid| config.account(uuid)) {
            Some(account) if !account::same_label(&account.label, label) => {
                Err(Error::FolderHasAnotherAccount {
                    path: config_dir.to_path_buf(),
                    label: account.label.clone(),
                    requested: label.to_owned(),
                })
            }
            _ => Ok(()),
        }
    }

    /// Whether an add can skip the login. The credentials decide, as they do for the app's
    /// "Log in" ([`LoginState`]): a token, valid or expired, skips it, since Claude Code renews
    /// an expired one the next time it runs; none, emptied or unreadable ones open it.
    /// `oauthAccount` alone says nothing: it may outlive the login.
    ///
    /// One exception: on Windows Claude Code may keep the credentials in the Credential
    /// Manager instead of `.credentials.json`, and mccboard doesn't look there. With no
    /// credentials file and an account in `.claude.json`, the login can't be told, so it isn't
    /// forced; "Log in" stays available for the instance.
    fn logged_in_already(&self, config_dir: &Path) -> bool {
        let observation = self.observe(config_dir);
        match observation.login {
            LoginState::LoggedIn { .. } | LoginState::Expired { .. } => true,
            LoginState::NotLoggedIn => {
                self.env.os() == OsKind::Windows
                    && observation.account.is_some()
                    && !claude_files::credentials_path(config_dir).exists()
            }
            LoginState::Unknown => false,
        }
    }

    fn check_addable(&self, config_dir: &Path) -> Result<(), Error> {
        if self.kind_of(config_dir) == InstanceKind::Primary {
            return Err(Error::PrimaryInstance {
                path: config_dir.to_path_buf(),
            });
        }
        if config_dir.exists() && !config_dir.is_dir() {
            return Err(Error::NotADirectory {
                path: config_dir.to_path_buf(),
            });
        }
        if folder_guard::is_protected(&self.home, self.paths().root(), config_dir) {
            return Err(Error::FolderNotAddable {
                path: config_dir.to_path_buf(),
            });
        }
        let profile_folders = ConfigFile::load(&self.paths().config_file())?
            .config
            .desktop_profile_folders();
        if desktop_app::contains(&self.home, &profile_folders, config_dir) {
            return Err(Error::DesktopAppFolder {
                path: config_dir.to_path_buf(),
            });
        }
        if !self.env.file_links_allowed() && self.needs_file_links(config_dir)? {
            return Err(Error::DeveloperModeOff);
        }
        if config_dir.is_dir() {
            session_guard::ensure_idle(&self.home, self.env.as_ref(), config_dir)?;
        }
        Ok(())
    }

    /// Whether linking the instance in `config_dir` needs a file link: one of the doctor's
    /// link drift items whose repair makes a file link. Before the folder exists every linked
    /// item of the primary is missing there, so every file among them counts.
    fn needs_file_links(&self, config_dir: &Path) -> Result<bool, Error> {
        let answers = ConfigFile::load(&self.paths().config_file())?
            .config
            .sharing;
        let primary = doctor::primary_dir(&self.home);
        Ok(doctor::link_drift(&primary, config_dir, &answers)
            .iter()
            .any(|item| repair::needs_file_link(&item.repair)))
    }

    /// Opens the login through the port, in `terminal`. A primary instance, spelled however
    /// (a link to it too), logs in in Claude Code's default folder.
    fn open_login(&self, config_dir: &Path, terminal: LoginTerminal) -> Result<LoginRun, Error> {
        let kind = self.kind_of(config_dir);
        // The primary instance's folder may be missing: Claude Code makes it on its first
        // login, and the login then runs in the home.
        let first_login =
            kind == InstanceKind::Primary && !config_dir.exists() && self.home.is_dir();
        if !config_dir.is_dir() && !first_login {
            return Err(Error::InstanceFolderMissing {
                path: config_dir.to_path_buf(),
            });
        }
        let invocation = LoginInvocation {
            config_dir: match kind {
                InstanceKind::Primary => doctor::primary_dir(&self.home),
                InstanceKind::Linked => config_dir.to_path_buf(),
            },
            kind,
            terminal,
            home: self.home.clone(),
        };
        match self.env.open_login(&invocation) {
            Ok(exit) => Ok(LoginRun::Ran {
                exit_code: exit.exit_code,
            }),
            // The port answers `NotFound` only for `claude` itself: the folder is checked above.
            Err(error) => Err(match error.kind() {
                io::ErrorKind::NotFound => Error::ClaudeNotInstalled,
                io::ErrorKind::Unsupported => Error::LoginNeedsTerminal {
                    path: config_dir.to_path_buf(),
                },
                _ => Error::LoginNotOpened {
                    path: config_dir.to_path_buf(),
                    reason: error.to_string(),
                },
            }),
        }
    }

    /// Observes the account and registers the instance under it; an account new to mccboard
    /// gets the label the add asked for, if one is waiting (see [`Core::register`]).
    fn after_login(&self, config_dir: &Path, run: LoginRun) -> Result<LoginReport, Error> {
        // The primary instance's login made no folder: there is nothing to register.
        if !config_dir.is_dir() {
            return self.unregistered_login(config_dir, run);
        }
        let registration = self.register(config_dir)?;
        let uuid = registration
            .instance
            .observation
            .account
            .as_ref()
            .map(|observed| observed.uuid.clone());
        let new_account = registration.new_account.is_some();

        let registry = self.registry()?;
        let account = uuid
            .as_ref()
            .and_then(|uuid| registry.accounts.iter().find(|a| &a.uuid == uuid).cloned());

        let mut warnings = Vec::new();
        if let Some(uuid) = &uuid {
            let others: Vec<PathBuf> = registry
                .instances
                .iter()
                .filter(|i| !paths::same(&i.config_dir, config_dir))
                .filter(|i| {
                    i.registered_account.as_ref() == Some(uuid)
                        || i.observation.account.as_ref().map(|a| &a.uuid) == Some(uuid)
                })
                .map(|i| i.config_dir.clone())
                .collect();
            if !others.is_empty() {
                warnings.push(LoginWarning::AccountHasOtherInstances {
                    account: uuid.clone(),
                    label: account.as_ref().map(|a| a.label.clone()),
                    instances: others,
                });
            }
        }

        Ok(LoginReport {
            config_dir: config_dir.to_path_buf(),
            run,
            instance: registration.instance,
            account,
            new_account,
            warnings,
        })
    }

    /// A login after which nothing is registered: no account logged in, or no folder to
    /// register. The instance is reported as registered, or else as it is now.
    fn unregistered_login(&self, config_dir: &Path, run: LoginRun) -> Result<LoginReport, Error> {
        let registered = self
            .registry()?
            .instances
            .into_iter()
            .find(|instance| paths::same(&instance.config_dir, config_dir));
        let instance = registered.unwrap_or_else(|| {
            crate::registry::unregistered(&self.home, config_dir, self.env.as_ref())
        });
        Ok(LoginReport {
            config_dir: config_dir.to_path_buf(),
            run,
            instance,
            account: None,
            new_account: false,
            warnings: Vec::new(),
        })
    }
}
