//! `~/.mccboard/config.toml`: registrations and settings, hand-editable.
//!
//! The file is read into a typed, validated view for the operations, and edited through a
//! `toml_edit` document so comments, ordering and keys mccboard doesn't know survive every
//! write. A file that doesn't parse or validate is reported and never written. Every edit
//! re-reads the file first, so changes made by the CLI, the app or by hand are never lost to a
//! stale copy.
//!
//! Schema (later features add keys next to these):
//!
//! ```toml
//! [accounts.<account uuid>]
//! label = "A"            # required
//! color = "#2563eb"      # required, #rrggbb
//! email = "a@example.com"  # display only, refreshed on registration
//! launcher = "cb"        # optional launcher name override
//!
//! [[instances]]
//! path = "/home/me/.claude-b"   # absolute config directory
//! account = "<account uuid>"    # the account it was registered with; absent when not logged in
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml_edit::{DocumentMut, Item, TableLike, Value};

use crate::account::{self, Account, AccountUuid, Color};
use crate::{Error, paths};

/// An instance as registered.
#[derive(Debug, Clone)]
pub(crate) struct InstanceEntry {
    pub config_dir: PathBuf,
    pub account: Option<AccountUuid>,
}

/// The validated contents of `config.toml`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Config {
    pub accounts: Vec<Account>,
    pub instances: Vec<InstanceEntry>,
}

impl Config {
    pub fn account(&self, uuid: &AccountUuid) -> Option<&Account> {
        self.accounts.iter().find(|a| &a.uuid == uuid)
    }

    pub fn instance(&self, config_dir: &Path) -> Option<&InstanceEntry> {
        self.instances
            .iter()
            .find(|i| paths::same(&i.config_dir, config_dir))
    }
}

/// `config.toml` as loaded for an edit: the typed view plus the document to change.
pub(crate) struct ConfigFile {
    path: PathBuf,
    original: String,
    doc: DocumentMut,
    pub config: Config,
}

impl ConfigFile {
    /// Reads and validates the file. A missing file is an empty configuration.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let original = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(source) => {
                return Err(Error::ConfigUnreadable {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        let corrupt = |detail: String| Error::ConfigCorrupt {
            path: path.to_path_buf(),
            detail,
        };
        let raw: RawConfig = toml::from_str(&original).map_err(|e| {
            let line = e.span().map(|span| {
                original[..span.start.min(original.len())]
                    .lines()
                    .count()
                    .max(1)
            });
            let message = e.message().trim().to_owned();
            corrupt(match line {
                Some(line) => format!("line {line}: {message}"),
                None => message,
            })
        })?;
        let config = raw.validate().map_err(corrupt)?;
        let doc = original
            .parse::<DocumentMut>()
            .map_err(|e| corrupt(e.message().trim().to_owned()))?;
        Ok(Self {
            path: path.to_path_buf(),
            original,
            doc,
            config,
        })
    }

    /// Writes the document back if an edit changed it, atomically (a temporary file in the
    /// same folder, then a rename).
    pub fn save(&self) -> Result<(), Error> {
        let text = self.doc.to_string();
        if text == self.original {
            return Ok(());
        }
        let unwritable = |source| Error::ConfigUnwritable {
            path: self.path.clone(),
            source,
        };
        let dir = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir).map_err(unwritable)?;
        let temporary = dir.join(format!(".config.toml.{}.tmp", std::process::id()));
        fs::write(&temporary, text).map_err(unwritable)?;
        fs::rename(&temporary, &self.path).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            unwritable(e)
        })
    }

    /// Adds or updates an account's table, keeping keys and comments it already has.
    pub fn set_account(&mut self, account: &Account) {
        let accounts = self.doc.entry("accounts").or_insert_with(|| {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            Item::Table(table)
        });
        // Inside an inline table (`accounts = { … }`) every entry must be inline too.
        let inline = accounts.is_inline_table();
        let entry = accounts
            .as_table_like_mut()
            .expect("validated: accounts is a table")
            .entry(account.uuid.as_str())
            .or_insert_with(|| {
                if inline {
                    Item::Value(Value::InlineTable(Default::default()))
                } else {
                    Item::Table(toml_edit::Table::new())
                }
            })
            .as_table_like_mut()
            .expect("validated: an account is a table");
        set_str(entry, "label", Some(&account.label));
        set_str(entry, "color", Some(account.color.as_str()));
        if let Some(email) = &account.email {
            set_str(entry, "email", Some(email));
        }
        set_str(entry, "launcher", account.launcher.as_deref());

        match self
            .config
            .accounts
            .iter_mut()
            .find(|a| a.uuid == account.uuid)
        {
            Some(existing) => *existing = account.clone(),
            None => self.config.accounts.push(account.clone()),
        }
    }

    /// Registers an instance, or updates the account it is registered with.
    pub fn set_instance(&mut self, config_dir: &Path, account: Option<&AccountUuid>) {
        let uuid = account.map(AccountUuid::as_str);
        let mut found = false;
        for_each_instance(&mut self.doc, |entry| {
            if entry_is(entry, config_dir) {
                found = true;
                set_str(entry, "account", uuid);
            }
        });
        if !found {
            let path = config_dir.to_string_lossy();
            match self.doc.get_mut("instances") {
                Some(Item::Value(Value::Array(array))) => {
                    let mut table = toml_edit::InlineTable::new();
                    table.insert("path", path.as_ref().into());
                    if let Some(uuid) = uuid {
                        table.insert("account", uuid.into());
                    }
                    array.push(table);
                }
                _ => {
                    let mut table = toml_edit::Table::new();
                    table.insert("path", toml_edit::value(path.as_ref()));
                    if let Some(uuid) = uuid {
                        table.insert("account", toml_edit::value(uuid));
                    }
                    self.doc
                        .entry("instances")
                        .or_insert(Item::ArrayOfTables(Default::default()))
                        .as_array_of_tables_mut()
                        .expect("validated: instances is a list of tables")
                        .push(table);
                }
            }
        }

        match self
            .config
            .instances
            .iter_mut()
            .find(|i| paths::same(&i.config_dir, config_dir))
        {
            Some(existing) => existing.account = account.cloned(),
            None => self.config.instances.push(InstanceEntry {
                config_dir: config_dir.to_path_buf(),
                account: account.cloned(),
            }),
        }
    }

    /// Forgets an instance; `false` if it wasn't registered.
    pub fn remove_instance(&mut self, config_dir: &Path) -> bool {
        let before = self.config.instances.len();
        self.config
            .instances
            .retain(|i| !paths::same(&i.config_dir, config_dir));
        if self.config.instances.len() == before {
            return false;
        }
        match self.doc.get_mut("instances") {
            Some(Item::ArrayOfTables(tables)) => tables.retain(|t| !entry_is(t, config_dir)),
            Some(Item::Value(Value::Array(array))) => array.retain(|value| {
                !value
                    .as_inline_table()
                    .is_some_and(|t| entry_is(t, config_dir))
            }),
            _ => {}
        }
        true
    }
}

fn for_each_instance(doc: &mut DocumentMut, mut f: impl FnMut(&mut dyn TableLike)) {
    match doc.get_mut("instances") {
        Some(Item::ArrayOfTables(tables)) => tables.iter_mut().for_each(|t| f(t)),
        Some(Item::Value(Value::Array(array))) => array
            .iter_mut()
            .filter_map(Value::as_inline_table_mut)
            .for_each(|t| f(t)),
        _ => {}
    }
}

fn entry_is(entry: &dyn TableLike, config_dir: &Path) -> bool {
    entry
        .get("path")
        .and_then(Item::as_str)
        .is_some_and(|path| paths::same(Path::new(path), config_dir))
}

/// Sets (or with `None` removes) a string key, keeping the comment after an existing value.
fn set_str(table: &mut dyn TableLike, key: &str, new: Option<&str>) {
    let Some(new) = new else {
        table.remove(key);
        return;
    };
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(value) if value.as_str() == Some(new) => {}
        Some(value) => {
            let decor = value.decor().clone();
            *value = Value::from(new);
            *value.decor_mut() = decor;
        }
        None => {
            table.insert(key, toml_edit::value(new));
        }
    }
}

#[derive(Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    accounts: BTreeMap<String, RawAccount>,
    #[serde(default)]
    instances: Vec<RawInstance>,
}

#[derive(Deserialize)]
struct RawAccount {
    label: String,
    color: String,
    email: Option<String>,
    launcher: Option<String>,
}

#[derive(Deserialize)]
struct RawInstance {
    path: PathBuf,
    account: Option<String>,
}

impl RawConfig {
    fn validate(self) -> Result<Config, String> {
        let mut accounts = Vec::new();
        for (uuid, raw) in self.accounts {
            let at = |field: &str| format!("accounts.{uuid}.{field}");
            let label =
                account::validate_label(&raw.label).map_err(|e| format!("{}: {e}", at("label")))?;
            let color = raw
                .color
                .parse::<Color>()
                .map_err(|e| format!("{}: {e}", at("color")))?;
            let launcher = raw
                .launcher
                .map(|name| account::validate_launcher(&name))
                .transpose()
                .map_err(|e| format!("{}: {e}", at("launcher")))?;
            accounts.push(Account {
                uuid: AccountUuid::new(uuid),
                label,
                color,
                email: raw.email.filter(|email| !email.is_empty()),
                launcher,
            });
        }

        let mut instances = Vec::new();
        for (index, raw) in self.instances.into_iter().enumerate() {
            if !raw.path.is_absolute() {
                return Err(format!(
                    "instances[{index}].path: {} is not an absolute path",
                    raw.path.display()
                ));
            }
            instances.push(InstanceEntry {
                config_dir: raw.path,
                account: raw
                    .account
                    .filter(|uuid| !uuid.is_empty())
                    .map(AccountUuid::new),
            });
        }
        Ok(Config {
            accounts,
            instances,
        })
    }
}
