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
//! label = "B"                   # the account label asked for when it was added, kept until its
//!                               # account is first observed (then that account gets it)
//!
//! [sharing.items]               # answers about unknown items of the primary instance
//! gsd-core = "link"             # "link" from the primary, or keep "private" to each instance
//!
//! [preferences]
//! language = "it"               # the app's language, "en" or "it"; absent: the system's
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml_edit::{DocumentMut, Item, TableLike, Value};

use crate::account::{self, Account, AccountUuid, Color};
use crate::sharing::{self, SharingAnswer};
use crate::{Error, Language, paths};

/// An instance as registered.
#[derive(Debug, Clone)]
pub(crate) struct InstanceEntry {
    pub config_dir: PathBuf,
    pub account: Option<AccountUuid>,
    /// The account label asked for when the instance was added, while no account has been
    /// observed in it yet.
    pub label: Option<String>,
}

/// The validated contents of `config.toml`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Config {
    pub accounts: Vec<Account>,
    pub instances: Vec<InstanceEntry>,
    /// Answers about unknown items, by item name.
    pub sharing: BTreeMap<String, SharingAnswer>,
    /// The app's language, when the user chose one.
    pub language: Option<Language>,
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

    /// Registers an instance, or updates the account it is registered with and the label
    /// waiting for its account (`None` removes it).
    pub fn set_instance(
        &mut self,
        config_dir: &Path,
        account: Option<&AccountUuid>,
        label: Option<&str>,
    ) {
        let uuid = account.map(AccountUuid::as_str);
        let mut found = false;
        for_each_instance(&mut self.doc, |entry| {
            if entry_is(entry, config_dir) {
                found = true;
                set_str(entry, "account", uuid);
                set_str(entry, "label", label);
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
                    if let Some(label) = label {
                        table.insert("label", label.into());
                    }
                    array.push(table);
                }
                _ => {
                    let mut table = toml_edit::Table::new();
                    table.insert("path", toml_edit::value(path.as_ref()));
                    if let Some(uuid) = uuid {
                        table.insert("account", toml_edit::value(uuid));
                    }
                    if let Some(label) = label {
                        table.insert("label", toml_edit::value(label));
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
            Some(existing) => {
                existing.account = account.cloned();
                existing.label = label.map(str::to_owned);
            }
            None => self.config.instances.push(InstanceEntry {
                config_dir: config_dir.to_path_buf(),
                account: account.cloned(),
                label: label.map(str::to_owned),
            }),
        }
    }

    /// Records the answer about an unknown item, keeping the comment after an existing one.
    pub fn set_sharing(&mut self, item: &str, answer: SharingAnswer) {
        let implicit_table = || {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            Item::Table(table)
        };
        let sharing = self.doc.entry("sharing").or_insert_with(implicit_table);
        let inline = sharing.is_inline_table();
        let items = sharing
            .as_table_like_mut()
            .expect("validated: sharing is a table")
            .entry("items")
            .or_insert_with(|| {
                if inline {
                    Item::Value(Value::InlineTable(Default::default()))
                } else {
                    Item::Table(toml_edit::Table::new())
                }
            })
            .as_table_like_mut()
            .expect("validated: sharing.items is a table");
        set_str(items, item, Some(answer.as_str()));
        self.config.sharing.insert(item.to_owned(), answer);
    }

    /// Sets the app's language in `[preferences]`, keeping the comment after an existing one.
    pub fn set_language(&mut self, language: Language) {
        let preferences = self
            .doc
            .entry("preferences")
            .or_insert_with(|| Item::Table(toml_edit::Table::new()))
            .as_table_like_mut()
            .expect("validated: preferences is a table");
        set_str(preferences, "language", Some(language.code()));
        self.config.language = Some(language);
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
    #[serde(default)]
    sharing: RawSharing,
    #[serde(default)]
    preferences: RawPreferences,
}

#[derive(Deserialize, Default)]
struct RawPreferences {
    language: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawSharing {
    #[serde(default)]
    items: BTreeMap<String, String>,
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
    label: Option<String>,
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
            let label = raw
                .label
                .map(|label| account::validate_label(&label))
                .transpose()
                .map_err(|e| format!("instances[{index}].label: {e}"))?;
            instances.push(InstanceEntry {
                config_dir: raw.path,
                account: raw
                    .account
                    .filter(|uuid| !uuid.is_empty())
                    .map(AccountUuid::new),
                label,
            });
        }
        let mut sharing = BTreeMap::new();
        for (item, answer) in self.sharing.items {
            let at = format!("sharing.items.{item}");
            sharing::check_plain(&item).map_err(|e| format!("{at}: {e}"))?;
            let answer = answer
                .parse::<SharingAnswer>()
                .map_err(|e| format!("{at}: {e}"))?;
            if sharing::is_private(&item) {
                // Answered while still unknown; the table has since learned it is private.
                continue;
            }
            sharing::check_item(&item).map_err(|e| format!("{at}: {e}"))?;
            sharing.insert(item, answer);
        }

        let language = self
            .preferences
            .language
            .map(|code| code.parse::<Language>())
            .transpose()
            .map_err(|e| format!("preferences.language: {e}"))?;

        Ok(Config {
            accounts,
            instances,
            sharing,
            language,
        })
    }
}
