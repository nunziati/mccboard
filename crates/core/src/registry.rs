//! Registered instances and accounts: what mccboard knows, as opposed to what a scan finds.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::account::{self, Account, AccountUpdate, AccountUuid};
use crate::config::{ConfigFile, InstanceEntry};
use crate::instance::{self, InstanceKind, Observation};
use crate::{Environment, Error, ObservedAccount, RenewalSituation, paths};

/// Everything registered: accounts by label, instances primary first then by path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Registry {
    pub accounts: Vec<Account>,
    pub instances: Vec<InstanceStatus>,
}

/// A registered instance as it is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceStatus {
    pub config_dir: PathBuf,
    pub kind: InstanceKind,
    /// The account the instance was registered with. It groups the instance; the observed
    /// account (in `observation`) may have moved on.
    pub registered_account: Option<AccountUuid>,
    #[serde(flatten)]
    pub observation: Observation,
    pub account_status: AccountStatus,
    /// The token renewal that runs in this instance: Claude Code renewing its account's
    /// expired token here. `Quiet` for every other instance, and in a registration's result.
    pub renewal: RenewalSituation,
}

/// How the observed account compares with the one the instance was registered with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AccountStatus {
    /// Logged in with the account it was registered with.
    Unchanged,
    /// Logged in with another account than the one it was registered with (story 11).
    /// Registering it again accepts the new account.
    Changed { registered: AccountUuid },
    /// Registered without an account and logged in since. Registering it again records it.
    Unrecorded,
    /// No account can be observed now (logged out, or never logged in). Not a change: the
    /// instance keeps the account it was registered with.
    NotObserved,
}

/// The outcome of registering an instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Registration {
    pub instance: InstanceStatus,
    /// The account added for a newly seen account UUID, with its suggested label and color.
    pub new_account: Option<Account>,
}

pub(crate) fn registry(
    home: &Path,
    config_file: &Path,
    env: &dyn Environment,
) -> Result<Registry, Error> {
    let config = ConfigFile::load(config_file)?.config;

    let mut accounts = config.shown_accounts(home);
    accounts
        .sort_by(|a, b| (a.label.to_lowercase(), &a.uuid).cmp(&(b.label.to_lowercase(), &b.uuid)));

    let mut instances: Vec<_> = config
        .instances
        .iter()
        .map(|entry| status(home, entry, env))
        .collect();
    instances.sort_by(|a, b| {
        (a.kind != InstanceKind::Primary, paths::key(&a.config_dir))
            .cmp(&(b.kind != InstanceKind::Primary, paths::key(&b.config_dir)))
    });
    instances.dedup_by(|a, b| paths::same(&a.config_dir, &b.config_dir));

    Ok(Registry {
        accounts,
        instances,
    })
}

/// `label` is the account label asked for by an add: it waits in `config.toml` until an
/// account is observed in the instance, and that account gets it if it is new to mccboard and
/// no other account has it. `None` keeps whatever label is waiting.
pub(crate) fn register(
    home: &Path,
    config_file: &Path,
    env: &dyn Environment,
    path: &Path,
    label: Option<&str>,
) -> Result<Registration, Error> {
    let config_dir = paths::resolve(home, path)?;
    let mut file = ConfigFile::load(config_file)?;
    if !config_dir.is_dir() {
        return Err(Error::NotADirectory { path: config_dir });
    }
    // An instance registered already keeps the spelling of its path in the file.
    let config_dir = file
        .config
        .instance(&config_dir)
        .map_or(config_dir, |entry| entry.config_dir.clone());

    let observation = instance::observe(home, &config_dir, env);
    let wanted = label.map(str::to_owned).or_else(|| {
        file.config
            .instance(&config_dir)
            .and_then(|entry| entry.label.clone())
    });
    let new_account = observation.account.as_ref().and_then(|observed| {
        record_account(&mut file, &config_dir, home, observed, wanted.as_deref())
    });
    // Once an account is observed the label has been used, or the account had one already.
    let waiting = wanted.filter(|_| observation.account.is_none());

    // A logged-out instance keeps the account it was registered with.
    let account = observation
        .account
        .as_ref()
        .map(|observed| observed.uuid.clone())
        .or_else(|| {
            file.config
                .instance(&config_dir)
                .and_then(|entry| entry.account.clone())
        });
    file.set_instance(&config_dir, account.as_ref(), waiting.as_deref());
    file.save()?;
    let new_account = new_account.map(|account| file.config.shown_account(home, &account));

    let entry = InstanceEntry {
        config_dir,
        account,
        label: waiting,
    };
    Ok(Registration {
        instance: status_with(home, &entry, observation),
        new_account,
    })
}

/// Adds a newly seen account, labelled `wanted` when given and free, else with a suggested
/// label, and a suggested color; or refreshes the email of a known one. Returns the account if
/// it is new.
fn record_account(
    file: &mut ConfigFile,
    config_dir: &Path,
    home: &Path,
    observed: &ObservedAccount,
    wanted: Option<&str>,
) -> Option<Account> {
    if let Some(known) = file.config.account(&observed.uuid) {
        if observed.email.is_some() && known.email != observed.email {
            let refreshed = Account {
                email: observed.email.clone(),
                ..known.clone()
            };
            file.set_account(&refreshed);
        }
        return None;
    }
    let labels: Vec<&str> = file
        .config
        .accounts
        .iter()
        .map(|a| a.label.as_str())
        .collect();
    let colors: Vec<_> = file.config.accounts.iter().map(|a| &a.color).collect();
    let primary = InstanceKind::of(home, config_dir) == InstanceKind::Primary;
    let label = wanted
        .filter(|wanted| {
            !labels
                .iter()
                .any(|taken| account::same_label(taken, wanted))
        })
        .map_or_else(
            || account::suggest_label(config_dir, primary, &labels),
            str::to_owned,
        );
    let account = Account {
        uuid: observed.uuid.clone(),
        label,
        color: account::suggest_color(&colors),
        email: observed.email.clone(),
        launcher: None,
    };
    file.set_account(&account);
    Some(account)
}

pub(crate) fn unregister(home: &Path, config_file: &Path, path: &Path) -> Result<(), Error> {
    let config_dir = paths::resolve(home, path)?;
    let mut file = ConfigFile::load(config_file)?;
    if !file.remove_instance(&config_dir) {
        return Err(Error::NotRegistered { path: config_dir });
    }
    file.save()
}

/// Refuses a new color for the default desktop profile's account
/// ([`Error::DefaultProfileColor`]): it is always shown in Claude's orange.
pub(crate) fn update_account(
    home: &Path,
    config_file: &Path,
    uuid: &AccountUuid,
    update: AccountUpdate,
) -> Result<Account, Error> {
    let mut file = ConfigFile::load(config_file)?;
    let mut account = file
        .config
        .account(uuid)
        .cloned()
        .ok_or_else(|| Error::AccountNotFound {
            account: uuid.to_string(),
        })?;

    if let Some(label) = update.label {
        let label = account::validate_label(&label)?;
        let taken = file
            .config
            .accounts
            .iter()
            .any(|other| &other.uuid != uuid && account::same_label(&other.label, &label));
        if taken {
            return Err(Error::LabelTaken { label });
        }
        account.label = label;
    }
    if let Some(color) = update.color {
        if file.config.default_profile_account(home) == Some(uuid) {
            return Err(Error::DefaultProfileColor {
                label: account.label,
            });
        }
        account.color = color;
    }
    if let Some(launcher) = update.launcher {
        account.launcher = launcher
            .map(|name| account::validate_launcher(&name))
            .transpose()?;
    }

    file.set_account(&account);
    file.save()?;
    Ok(file.config.shown_account(home, &account))
}

/// The config directories registered, for marking scan candidates.
pub(crate) fn registered_dirs(config_file: &Path) -> Result<Vec<PathBuf>, Error> {
    Ok(ConfigFile::load(config_file)?
        .config
        .instances
        .into_iter()
        .map(|entry| entry.config_dir)
        .collect())
}

fn status(home: &Path, entry: &InstanceEntry, env: &dyn Environment) -> InstanceStatus {
    status_with(home, entry, instance::observe(home, &entry.config_dir, env))
}

fn status_with(home: &Path, entry: &InstanceEntry, observation: Observation) -> InstanceStatus {
    let account_status = match (&entry.account, &observation.account) {
        (Some(registered), Some(observed)) if *registered == observed.uuid => {
            AccountStatus::Unchanged
        }
        (Some(registered), Some(_)) => AccountStatus::Changed {
            registered: registered.clone(),
        },
        (None, Some(_)) => AccountStatus::Unrecorded,
        (_, None) => AccountStatus::NotObserved,
    };
    InstanceStatus {
        kind: InstanceKind::of(home, &entry.config_dir),
        config_dir: entry.config_dir.clone(),
        registered_account: entry.account.clone(),
        observation,
        account_status,
        renewal: RenewalSituation::Quiet,
    }
}
