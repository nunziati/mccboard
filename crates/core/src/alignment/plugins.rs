//! Installed plugins and marketplaces, compared through the files Claude Code keeps in each
//! instance's `plugins/` folder and installed with Claude Code's own `claude plugin` commands.
//!
//! Only plugins installed for the user (scope `user`) from a marketplace are aligned: those
//! installed for a project, the skills folder's (`@skills-dir`), the account's claude.ai ones
//! (`@synced`) and a session's (`@inline`) belong to their instance.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::Path;

use serde_json::Value;

use crate::doctor::{DriftItem, Problem, Repair};
use crate::sharing::PLUGINS;
use crate::{ClaudeInvocation, Environment, links};

const INSTALLED: &str = "installed_plugins.json";
const MARKETPLACES: &str = "known_marketplaces.json";

/// Pseudo-marketplaces of plugins that belong to their instance.
const NOT_ALIGNED: &[&str] = &["skills-dir", "synced", "inline"];

/// What an instance has installed: marketplaces by name, each with the source `claude plugin
/// marketplace add` takes (when mccboard can tell it), and plugin ids installed for the user.
#[derive(Default)]
struct Installed {
    marketplaces: BTreeMap<String, Option<String>>,
    plugins: BTreeSet<String>,
}

impl Installed {
    fn read(plugins_dir: &Path) -> Self {
        Self {
            marketplaces: read_json(&plugins_dir.join(MARKETPLACES))
                .and_then(|value| value.as_object().cloned())
                .map(|map| {
                    map.into_iter()
                        .map(|(name, entry)| (name, entry.get("source").and_then(source)))
                        .collect()
                })
                .unwrap_or_default(),
            plugins: read_json(&plugins_dir.join(INSTALLED))
                .and_then(|value| value.get("plugins").and_then(Value::as_object).cloned())
                .map(|map| {
                    map.into_iter()
                        .filter(|(id, entries)| aligned(id) && for_user(entries))
                        .map(|(id, _)| id)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// What the linked instance in `config_dir` has. A `plugins/` linked as a whole folder
    /// counts as empty: it is the primary's, and migrating it leaves an empty folder.
    fn of_linked(config_dir: &Path) -> Self {
        let dir = config_dir.join(PLUGINS);
        if links::is_link(&dir) {
            Self::default()
        } else {
            Self::read(&dir)
        }
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// A marketplace's source, as `claude plugin marketplace add` takes it: `owner/repo[#ref]`
/// for GitHub, the URL (with `#ref`) for git and URLs, the path for local folders and files.
/// `None` for kinds it can't be told from (npm, settings).
fn source(value: &Value) -> Option<String> {
    let field = |name: &str| value.get(name).and_then(Value::as_str);
    let with_ref = |base: &str| match field("ref").filter(|r| !r.is_empty()) {
        Some(r) => format!("{base}#{r}"),
        None => base.to_owned(),
    };
    match field("source")? {
        "github" => field("repo").map(with_ref),
        "git" => field("url").map(with_ref),
        "url" => field("url").map(str::to_owned),
        "directory" | "file" => field("path").map(str::to_owned),
        _ => None,
    }
}

/// `name@marketplace` from a real marketplace.
fn aligned(id: &str) -> bool {
    marketplace_of(id).is_some_and(|m| !m.is_empty() && !NOT_ALIGNED.contains(&m))
}

fn marketplace_of(id: &str) -> Option<&str> {
    id.rsplit_once('@').map(|(_, marketplace)| marketplace)
}

/// Whether any of a plugin's installations is for the user. Version 1 of the file has one
/// entry per plugin, without a scope: those were all for the user.
fn for_user(entries: &Value) -> bool {
    let is_user = |entry: &Value| {
        entry
            .get("scope")
            .and_then(Value::as_str)
            .is_none_or(|scope| scope == "user")
    };
    match entries {
        Value::Array(list) => list.iter().any(is_user),
        Value::Object(_) => is_user(entries),
        _ => false,
    }
}

/// Marketplaces then plugins of the primary's that the linked instance in `config_dir` lacks.
pub(crate) fn drift(primary: &Path, config_dir: &Path) -> Vec<DriftItem> {
    let wanted = Installed::read(&primary.join(PLUGINS));
    let found = Installed::of_linked(config_dir);
    let at = config_dir.join(PLUGINS);

    let marketplaces = wanted
        .marketplaces
        .iter()
        .filter(|(name, _)| !found.marketplaces.contains_key(*name))
        .map(|(name, source)| {
            DriftItem::new(
                format!("marketplace:{name}"),
                name,
                &at,
                Problem::MissingMarketplace {
                    marketplace: name.clone(),
                },
                Repair::AddMarketplace {
                    marketplace: name.clone(),
                    source: source.clone(),
                },
            )
        });
    let plugins = wanted.plugins.difference(&found.plugins).map(|id| {
        DriftItem::new(
            format!("plugin:{id}"),
            id,
            &at,
            Problem::MissingPlugin { plugin: id.clone() },
            Repair::InstallPlugin { plugin: id.clone() },
        )
    });
    marketplaces.chain(plugins).collect()
}

/// One repair's plugin work in one instance. A marketplace is added once per run, whether
/// as its own item or for a plugin from it, and a failure is remembered, so the plugins from
/// a marketplace that couldn't be added fail with that reason without trying again.
pub(crate) struct PluginRun<'a> {
    env: &'a dyn Environment,
    primary: &'a Path,
    config_dir: &'a Path,
    added: HashMap<String, Result<(), String>>,
}

impl<'a> PluginRun<'a> {
    pub(crate) fn new(env: &'a dyn Environment, primary: &'a Path, config_dir: &'a Path) -> Self {
        Self {
            env,
            primary,
            config_dir,
            added: HashMap::new(),
        }
    }

    /// `claude plugin marketplace add <source>` in the instance.
    pub(crate) fn add_marketplace(&mut self, name: &str, source: Option<&str>) -> io::Result<()> {
        self.check_own_folder()?;
        if let Some(done) = self.added.get(name) {
            return done.clone().map_err(io::Error::other);
        }
        let result = match source {
            Some(source) => self.run(&["plugin", "marketplace", "add", source, "--json"]),
            None => Err(format!(
                "mccboard can't tell how to add the marketplace {name}; add it by hand with \
                 `claude plugin marketplace add` in this instance"
            )),
        };
        self.added.insert(name.to_owned(), result.clone());
        result.map_err(io::Error::other)
    }

    /// `claude plugin install <id> --scope user` in the instance, after adding its
    /// marketplace when the instance doesn't have it yet.
    pub(crate) fn install(&mut self, plugin: &str) -> io::Result<()> {
        self.check_own_folder()?;
        let marketplace = marketplace_of(plugin).unwrap_or_default();
        let missing = !Installed::of_linked(self.config_dir)
            .marketplaces
            .contains_key(marketplace);
        if missing {
            let primary = Installed::read(&self.primary.join(PLUGINS));
            if let Some(source) = primary.marketplaces.get(marketplace) {
                self.add_marketplace(marketplace, source.as_deref())
                    .map_err(|error| {
                        io::Error::other(format!(
                            "its marketplace {marketplace} couldn't be added: {error}"
                        ))
                    })?;
            }
        }
        self.run(&["plugin", "install", plugin, "--scope", "user", "--json"])
            .map_err(io::Error::other)
    }

    /// Installing through a `plugins/` linked as a whole folder would write into the
    /// primary's.
    fn check_own_folder(&self) -> io::Result<()> {
        if links::is_link(&self.config_dir.join(PLUGINS)) {
            return Err(io::Error::other(
                "plugins is linked as a whole folder: repair migrate:plugins first",
            ));
        }
        Ok(())
    }

    fn run(&self, args: &[&str]) -> Result<(), String> {
        let invocation = ClaudeInvocation {
            config_dir: self.config_dir.to_path_buf(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        };
        let run = match self.env.run_claude(&invocation) {
            Ok(run) => run,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err("Claude Code (claude) isn't installed or isn't on the PATH".to_owned());
            }
            Err(error) => return Err(format!("Claude Code couldn't run: {error}")),
        };
        // With --json, the last line Claude Code prints says how it went.
        let result = run
            .stdout
            .lines()
            .rev()
            .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
            .filter(|value| value.get("outcome").is_some());
        let outcome = result
            .as_ref()
            .and_then(|r| r.get("outcome"))
            .and_then(Value::as_str);
        if run.success() && outcome != Some("failed") {
            return Ok(());
        }
        let message = result
            .as_ref()
            .and_then(|r| r.get("message"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                run.stderr
                    .lines()
                    .rev()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| match run.exit_code {
                Some(code) => format!("claude {} exited with code {code}", args[..2].join(" ")),
                None => format!("claude {} was stopped", args[..2].join(" ")),
            });
        Err(shorten(&message))
    }
}

/// Claude Code's own message, cut to a readable length.
fn shorten(message: &str) -> String {
    const MAX: usize = 300;
    match message.char_indices().nth(MAX) {
        Some((at, _)) => format!("{}…", &message[..at]),
        None => message.to_owned(),
    }
}
