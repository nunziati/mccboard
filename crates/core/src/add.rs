//! Adding a linked instance, and logging an instance in.
//!
//! Adding composes the other operations, in order:
//! 1. checks, before anything changes: the folder isn't the primary instance's, a file or in
//!    the Claude desktop app's data folder; file links can be made (Developer Mode on
//!    Windows) when the primary has files to link; no Claude Code session uses the folder;
//! 2. creates the folder (`~/.claude-<label>` by default);
//! 3. registers it, without an account until it is logged in: this creates its launcher;
//! 4. repairs its drift: the sharing table's links and the aligned items (Global config
//!    keys, plugins and marketplaces);
//! 5. opens Claude Code's official login through the environment port, unless the instance
//!    is logged in already;
//! 6. observes the account and registers the instance again, under it.
//!
//! Each step is idempotent, so running the add again after a failure at any step completes
//! what's missing. mccboard never reads or writes credentials here: Claude Code's login
//! writes them, and mccboard reads `oauthAccount` from `.claude.json` afterwards.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::account::{self, Account, AccountUpdate, AccountUuid};
use crate::config::ConfigFile;
use crate::instance::InstanceKind;
use crate::links::{self, Entry};
use crate::sharing::{self, Sharing};
use crate::{
    Core, Error, InstanceStatus, LoginInvocation, LoginTerminal, OsKind, RepairReport,
    SharingQuestion, desktop_app, doctor, paths, session_guard,
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
}

impl AddRequest {
    /// Adds `label`'s instance in the default folder, logging in.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            config_dir: None,
            login: true,
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
    /// Not opened: the instance was logged in already (an add run again).
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
    /// Refuses before changing anything for the primary instance's folder
    /// ([`Error::PrimaryInstance`]), a file, a folder in the Claude desktop app's data, when
    /// file links are needed and Windows Developer Mode is off, and while a Claude Code
    /// process uses the folder. Idempotent and resumable: after a failure at any step,
    /// adding again completes what's missing; an instance set up but not logged in is
    /// registered, and [`Core::login`] can finish it too.
    pub fn add(&self, request: &AddRequest) -> Result<AddReport, Error> {
        let label = account::validate_label(&request.label)?;
        let config_dir = match &request.config_dir {
            Some(path) => paths::resolve(&self.home, path)?,
            None => self.add_folder(&label)?,
        };
        self.check_addable(&config_dir)?;
        let known: Vec<AccountUuid> = self
            .registry()?
            .accounts
            .into_iter()
            .map(|a| a.uuid)
            .collect();

        let created = !config_dir.exists();
        fs::create_dir_all(&config_dir).map_err(|source| Error::FolderNotCreated {
            path: config_dir.clone(),
            source,
        })?;
        // An instance registered already keeps the spelling of its path in config.toml.
        let config_dir = self.register(&config_dir)?.instance.config_dir;
        let setup = self.repair(&config_dir, None)?;
        let sharing_questions = self.sharing_questions()?;

        let run = if !request.login {
            LoginRun::NotRequested
        } else if self.observe(&config_dir).account.is_some() {
            LoginRun::AlreadyLoggedIn
        } else {
            self.open_login(&config_dir)?
        };
        let login = self.after_login(&config_dir, run, Some((&label, &known)))?;
        Ok(AddReport {
            config_dir,
            created,
            setup,
            sharing_questions,
            login,
        })
    }

    /// Opens Claude Code's official login in the registered instance in `config_dir`
    /// (absolute, or starting with `~`), whatever its login state, and waits for it to end:
    /// in a new terminal window on Windows, in the current terminal on Linux
    /// ([`Error::LoginNeedsTerminal`] without one). Then registers the instance again, under
    /// the account it is now logged in with, warning when that account has other instances
    /// here. mccboard never sees the credentials: Claude Code writes them.
    pub fn login(&self, config_dir: &Path) -> Result<LoginReport, Error> {
        let config_dir = paths::resolve(&self.home, config_dir)?;
        let config_dir = ConfigFile::load(&self.paths().config_file())?
            .config
            .instance(&config_dir)
            .map(|entry| entry.config_dir.clone())
            .ok_or(Error::NotRegistered { path: config_dir })?;
        let run = self.open_login(&config_dir)?;
        self.after_login(&config_dir, run, None)
    }

    fn check_addable(&self, config_dir: &Path) -> Result<(), Error> {
        if InstanceKind::of(&self.home, config_dir) == InstanceKind::Primary {
            return Err(Error::PrimaryInstance {
                path: config_dir.to_path_buf(),
            });
        }
        if config_dir.exists() && !config_dir.is_dir() {
            return Err(Error::NotADirectory {
                path: config_dir.to_path_buf(),
            });
        }
        if desktop_app::contains(&self.home, config_dir) {
            return Err(Error::DesktopAppFolder {
                path: config_dir.to_path_buf(),
            });
        }
        if !self.env.file_links_allowed() && self.primary_has_files_to_link()? {
            return Err(Error::DeveloperModeOff);
        }
        if config_dir.is_dir() {
            session_guard::ensure_idle(&self.home, self.env.as_ref(), config_dir)?;
        }
        Ok(())
    }

    /// Whether linking the primary's items needs a file link: a linked item of the sharing
    /// table, or an unknown item answered "link", that is a file.
    fn primary_has_files_to_link(&self) -> Result<bool, Error> {
        let answers = ConfigFile::load(&self.paths().config_file())?
            .config
            .sharing;
        let primary = doctor::primary_dir(&self.home);
        let answered = answers
            .keys()
            .filter(|name| sharing::classify(name, &answers) == Sharing::Linked);
        Ok(sharing::LINKED_ITEMS
            .iter()
            .map(|name| (*name).to_owned())
            .chain(answered.cloned())
            .map(|name| primary.join(name))
            .filter(|item| links::inspect(item) != Entry::Missing)
            .any(|item| links::is_file_link(&links::real_item(&item))))
    }

    /// Opens the login through the port: a new window on Windows, the current terminal
    /// elsewhere.
    fn open_login(&self, config_dir: &Path) -> Result<LoginRun, Error> {
        let terminal = match self.env.os() {
            OsKind::Windows => LoginTerminal::NewWindow,
            OsKind::Linux => LoginTerminal::CurrentTerminal,
        };
        let invocation = LoginInvocation {
            config_dir: config_dir.to_path_buf(),
            kind: InstanceKind::of(&self.home, config_dir),
            terminal,
        };
        match self.env.open_login(&invocation) {
            Ok(exit) => Ok(LoginRun::Ran {
                exit_code: exit.exit_code,
            }),
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

    /// Observes the account and registers the instance under it. `add` is the label asked
    /// for and the accounts known before the add began: an account new since then gets the
    /// label, if no other account has it.
    fn after_login(
        &self,
        config_dir: &Path,
        run: LoginRun,
        add: Option<(&str, &[AccountUuid])>,
    ) -> Result<LoginReport, Error> {
        let registration = self.register(config_dir)?;
        let uuid = registration
            .instance
            .observation
            .account
            .as_ref()
            .map(|observed| observed.uuid.clone());
        let new_account = registration.new_account.is_some()
            || add.is_some_and(|(_, known)| uuid.as_ref().is_some_and(|u| !known.contains(u)));

        let registry = self.registry()?;
        let mut account = uuid
            .as_ref()
            .and_then(|uuid| registry.accounts.iter().find(|a| &a.uuid == uuid).cloned());
        if let (Some((label, _)), Some(current)) = (add, &account) {
            let taken = registry
                .accounts
                .iter()
                .any(|a| a.uuid != current.uuid && account::same_label(&a.label, label));
            if new_account && current.label != label && !taken {
                let update = AccountUpdate {
                    label: Some(label.to_owned()),
                    ..AccountUpdate::default()
                };
                account = Some(self.update_account(&current.uuid, update)?);
            }
        }

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
}
