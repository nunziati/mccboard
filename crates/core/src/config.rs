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
//! [[desktop_profiles]]          # desktop profiles other than the default one
//! account = "<account uuid>"    # the account it is bound to; one profile per account
//! data_folder = 'C:\Users\me\AppData\Local\Claude-B'  # absolute: its own data directory
//! open_at_start = false         # open it when mccboard starts; absent: false
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

/// A desktop profile as stored (never the default desktop profile, which is implicit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesktopProfileEntry {
    pub account: AccountUuid,
    pub data_folder: PathBuf,
    pub open_at_start: bool,
}

/// The validated contents of `config.toml`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Config {
    pub accounts: Vec<Account>,
    pub instances: Vec<InstanceEntry>,
    /// Desktop profiles other than the default one, at most one per account.
    pub desktop_profiles: Vec<DesktopProfileEntry>,
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

    pub fn desktop_profile(&self, account: &AccountUuid) -> Option<&DesktopProfileEntry> {
        self.desktop_profiles.iter().find(|p| &p.account == account)
    }

    /// The desktop profiles' data folders, where mccboard never creates links.
    pub fn desktop_profile_folders(&self) -> Vec<PathBuf> {
        self.desktop_profiles
            .iter()
            .map(|p| p.data_folder.clone())
            .collect()
    }

    /// The registered primary instance (`~/.claude` in `home`), if any.
    pub fn primary_instance(&self, home: &Path) -> Option<&InstanceEntry> {
        self.instance(&home.join(".claude"))
    }

    /// The account of the default desktop profile: the one the primary instance is registered
    /// with.
    pub fn default_profile_account(&self, home: &Path) -> Option<&AccountUuid> {
        self.primary_instance(home)?.account.as_ref()
    }

    /// The accounts as every view model shows them: the default desktop profile's account in
    /// Claude's orange, whatever color is stored for it.
    pub fn shown_accounts(&self, home: &Path) -> Vec<Account> {
        let default = self.default_profile_account(home);
        self.accounts
            .iter()
            .map(|account| shown(account, default))
            .collect()
    }

    /// `account` as view models show it (see [`Config::shown_accounts`]).
    pub fn shown_account(&self, home: &Path, account: &Account) -> Account {
        shown(account, self.default_profile_account(home))
    }
}

fn shown(account: &Account, default_profile_account: Option<&AccountUuid>) -> Account {
    let mut account = account.clone();
    if default_profile_account == Some(&account.uuid) {
        account.color = account::claude_orange();
    }
    account
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

    /// Stores a new desktop profile, `open_at_start` written out so it can be found by hand.
    pub fn add_desktop_profile(&mut self, entry: &DesktopProfileEntry) {
        let folder = entry.data_folder.to_string_lossy();
        match self.doc.get_mut(DESKTOP_PROFILES) {
            Some(Item::Value(Value::Array(array))) => {
                let mut table = toml_edit::InlineTable::new();
                table.insert("account", entry.account.as_str().into());
                table.insert("data_folder", folder.as_ref().into());
                table.insert("open_at_start", entry.open_at_start.into());
                array.push(table);
            }
            _ => {
                let mut table = toml_edit::Table::new();
                table.insert("account", toml_edit::value(entry.account.as_str()));
                table.insert("data_folder", toml_edit::value(folder.as_ref()));
                table.insert("open_at_start", toml_edit::value(entry.open_at_start));
                self.doc
                    .entry(DESKTOP_PROFILES)
                    .or_insert(Item::ArrayOfTables(Default::default()))
                    .as_array_of_tables_mut()
                    .expect("validated: desktop_profiles is a list of tables")
                    .push(table);
            }
        }
        self.config.desktop_profiles.push(entry.clone());
    }

    /// Sets whether the desktop profile of `account` opens when mccboard starts, keeping the
    /// comment after an existing value; `false` if there is no such profile.
    pub fn set_desktop_profile_open_at_start(&mut self, account: &AccountUuid, on: bool) -> bool {
        let Some(entry) = self
            .config
            .desktop_profiles
            .iter_mut()
            .find(|p| &p.account == account)
        else {
            return false;
        };
        entry.open_at_start = on;
        for_each_entry(&mut self.doc, DESKTOP_PROFILES, |table| {
            if profile_is(table, account) {
                set_bool(table, "open_at_start", on);
            }
        });
        true
    }

    /// Forgets the desktop profile of `account`; `false` if there was none.
    pub fn remove_desktop_profile(&mut self, account: &AccountUuid) -> bool {
        let before = self.config.desktop_profiles.len();
        self.config
            .desktop_profiles
            .retain(|p| &p.account != account);
        if self.config.desktop_profiles.len() == before {
            return false;
        }
        match self.doc.get_mut(DESKTOP_PROFILES) {
            Some(Item::ArrayOfTables(tables)) => {
                tables.retain(|t| !profile_is(t, account));
                if tables.is_empty() {
                    self.doc.remove(DESKTOP_PROFILES);
                }
            }
            Some(Item::Value(Value::Array(array))) => array.retain(|value| {
                !value
                    .as_inline_table()
                    .is_some_and(|t| profile_is(t, account))
            }),
            _ => {}
        }
        true
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

/// The key of the desktop profiles' list.
const DESKTOP_PROFILES: &str = "desktop_profiles";

fn for_each_instance(doc: &mut DocumentMut, f: impl FnMut(&mut dyn TableLike)) {
    for_each_entry(doc, "instances", f);
}

/// Every table in the list `key`, written as `[[key]]` tables or as an inline array.
fn for_each_entry(doc: &mut DocumentMut, key: &str, mut f: impl FnMut(&mut dyn TableLike)) {
    match doc.get_mut(key) {
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

fn profile_is(entry: &dyn TableLike, account: &AccountUuid) -> bool {
    entry
        .get("account")
        .and_then(Item::as_str)
        .is_some_and(|uuid| uuid.trim() == account.as_str())
}

/// Sets a boolean key, keeping the comment after an existing value.
fn set_bool(table: &mut dyn TableLike, key: &str, new: bool) {
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(value) if value.as_bool() == Some(new) => {}
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
    #[serde(default)]
    desktop_profiles: Vec<RawDesktopProfile>,
}

#[derive(Deserialize)]
struct RawDesktopProfile {
    account: String,
    data_folder: PathBuf,
    #[serde(default)]
    open_at_start: bool,
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

        let mut desktop_profiles: Vec<DesktopProfileEntry> = Vec::new();
        for (index, raw) in self.desktop_profiles.into_iter().enumerate() {
            let at = |field: &str| format!("desktop_profiles[{index}].{field}");
            let account = raw.account.trim();
            if account.is_empty() {
                return Err(format!("{}: an account UUID is required", at("account")));
            }
            let account = AccountUuid::new(account);
            if desktop_profiles.iter().any(|p| p.account == account) {
                return Err(format!(
                    "{}: another desktop profile has account {account}; one per account",
                    at("account")
                ));
            }
            if !raw.data_folder.is_absolute() {
                return Err(format!(
                    "{}: {} is not an absolute path",
                    at("data_folder"),
                    raw.data_folder.display()
                ));
            }
            desktop_profiles.push(DesktopProfileEntry {
                account,
                data_folder: raw.data_folder,
                open_at_start: raw.open_at_start,
            });
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
            desktop_profiles,
            sharing,
            language,
        })
    }
}
