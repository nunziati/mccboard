//! The sharing table (ADR 0005): which top-level items of the primary instance a linked
//! instance links, and which stay private to each instance. Aligned items (Global config
//! keys, plugins) are compared elsewhere.
//!
//! Items on neither list are *unknown*: the user is asked once whether to link them or keep
//! them private, and the answer, kept in `config.toml`, applies to every instance.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// Items a linked instance links from the primary instance, one link each, in the order the
/// doctor reports them.
pub(crate) const LINKED_ITEMS: &[&str] = &[
    "settings.json",
    "CLAUDE.md",
    "agents",
    "hooks",
    "output-styles",
    "scripts",
];

/// The skills folder: never linked whole. Each personal skill in it is linked separately.
pub(crate) const SKILLS: &str = "skills";

/// Entries of `skills/` that are not personal skills: the account's claude.ai skills and
/// Claude Code's trash. Both private.
pub(crate) const NOT_PERSONAL_SKILLS: &[&str] = &["synced", ".trash"];

/// The plugins folder: private to each instance (plugins are aligned by reinstalling them).
pub(crate) const PLUGINS: &str = "plugins";

/// Top-level items that belong to one instance: credentials, the rest of `.claude.json`,
/// history, sessions and the other state Claude Code keeps per config directory. Never
/// linked, never asked about. Extend it when Claude Code adds state of its own.
const PRIVATE_ITEMS: &[&str] = &[
    // Credentials and the instance's own global config.
    ".credentials.json",
    ".claude.json",
    ".config.json",
    ".session_ingress_token",
    "hfi-auth.json",
    // Sessions and history.
    "projects",
    "sessions",
    "session-env",
    "history.jsonl",
    "todos",
    "tasks",
    "plans",
    "file-history",
    "shell-snapshots",
    "jobs",
    "teams",
    "uploads",
    "shares",
    "local",
    "agent-memory-local",
    "agent-memory-project",
    "scratch",
    "storage-v2",
    "active-time.json",
    "server-sessions.json",
    // Instance-local settings and server-sent policy.
    "settings.local.json",
    "remote-settings.json",
    "policy-limits.json",
    "launcher-settings.json",
    "local-settings",
    "project-settings",
    // Caches, logs and Claude Code's bookkeeping.
    "backups",
    "cache",
    "state",
    "statsig",
    "telemetry",
    "usage-data",
    "startup-perf",
    "traces",
    "debug",
    "logs",
    "ide",
    "chrome",
    "ccr",
    "daemon",
    "daemon.json",
    "daemon.log",
    "daemon.status.json",
    "daemon.scheduled.status.json",
    "remote-control",
    "remote",
    "bridge-spawn",
    "systemd",
    "downloads",
    "file-transfers",
    "mcp-skill-archives",
    "antproto.json",
    "api-dumps",
    "feedback",
    "feedback-bundles",
    "dump-prompts",
    "dev-mods",
    "seed-admin",
    "cowork_plugins",
    "mcp-discovery-cache",
    "mcp-needs-auth-cache.json",
    "gh-pr-status-cache.json",
    "stats-cache.json",
    "paste-cache",
    "image-cache",
    ".last-cleanup",
    ".last-update-result.json",
    ".update.lock",
    ".deep-link-register-failed",
    // Operating-system litter.
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
];

/// Name patterns of Claude Code's private files that vary (backups, temporary and lock
/// files, the daemon's auth state): prefixes and suffixes.
const PRIVATE_PREFIXES: &[&str] = &[
    ".claude.json",
    ".credentials.json",
    "security_warnings_state",
    "daemon-auth-",
];
const PRIVATE_SUFFIXES: &[&str] = &[".lock", ".tmp", ".backup", ".bak"];

/// The user's answer about an unknown item, kept in `config.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharingAnswer {
    /// Link it from the primary instance, like the linked items.
    Link,
    /// Keep it private to each instance: mccboard leaves it alone.
    Private,
}

impl SharingAnswer {
    pub fn as_str(self) -> &'static str {
        match self {
            SharingAnswer::Link => "link",
            SharingAnswer::Private => "private",
        }
    }
}

impl FromStr for SharingAnswer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "link" => Ok(SharingAnswer::Link),
            "private" => Ok(SharingAnswer::Private),
            other => Err(format!(
                "{other:?} is not an answer: use \"link\" or \"private\""
            )),
        }
    }
}

impl std::fmt::Display for SharingAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An unknown top-level item of the primary instance, not answered yet: "share `gsd-core`?"
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SharingQuestion {
    /// The item's name in the config directory (`gsd-core`).
    pub item: String,
    /// Where it is in the primary instance.
    pub path: PathBuf,
    /// Whether it is a folder.
    pub is_dir: bool,
}

/// How the sharing table and the user's answers treat a top-level item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sharing {
    Linked,
    Private,
    Unknown,
}

/// Whether the sharing table keeps an item private: Claude Code's own state. An answer
/// stored for it, from before the table knew it, is ignored.
pub(crate) fn is_private(name: &str) -> bool {
    PRIVATE_ITEMS.contains(&name)
        || PRIVATE_PREFIXES.iter().any(|p| name.starts_with(p))
        || PRIVATE_SUFFIXES.iter().any(|s| name.ends_with(s))
}

/// Whether an item is in the sharing table, so it takes no answer.
fn in_table(name: &str) -> bool {
    LINKED_ITEMS.contains(&name) || name == SKILLS || name == PLUGINS || is_private(name)
}

pub(crate) fn classify(name: &str, answers: &BTreeMap<String, SharingAnswer>) -> Sharing {
    if LINKED_ITEMS.contains(&name) {
        return Sharing::Linked;
    }
    if name == SKILLS || name == PLUGINS || is_private(name) {
        // `skills/` is handled skill by skill; `plugins/` is private.
        return Sharing::Private;
    }
    match answers.get(name) {
        Some(SharingAnswer::Link) => Sharing::Linked,
        Some(SharingAnswer::Private) => Sharing::Private,
        None => Sharing::Unknown,
    }
}

/// Checks that `item` can take an answer: one plain name in the config directory, not in
/// the sharing table. The message says why not.
pub(crate) fn check_item(item: &str) -> Result<(), String> {
    check_plain(item)?;
    if in_table(item) {
        return Err(format!(
            "{item:?} is in the sharing table already; only unknown items take an answer"
        ));
    }
    Ok(())
}

/// Checks that `item` is one plain name in the config directory: no path, no `.` or `..`.
pub(crate) fn check_plain(item: &str) -> Result<(), String> {
    let plain = !item.is_empty()
        && item != "."
        && item != ".."
        && !item.contains(['/', '\\'])
        && !item.contains(|c: char| c.is_control());
    if !plain {
        return Err(format!(
            "{item:?} is not the name of an item in a config folder"
        ));
    }
    Ok(())
}

pub(crate) fn validate_item(item: &str) -> Result<(), Error> {
    check_item(item).map_err(|_| Error::InvalidSharingItem {
        item: item.to_owned(),
    })
}

/// The top-level entries of a config directory, sorted by name. A missing or unreadable
/// folder has none.
pub(crate) fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// Personal skills of the primary instance: every folder in its `skills/` but `synced/` and
/// `.trash/`, sorted by name.
pub(crate) fn personal_skills(primary: &Path) -> Vec<String> {
    let skills = primary.join(SKILLS);
    entries(&skills)
        .into_iter()
        .filter(|name| !NOT_PERSONAL_SKILLS.contains(&name.as_str()))
        .filter(|name| skills.join(name).is_dir())
        .collect()
}

/// The unknown items of the primary instance the user hasn't answered about yet.
pub(crate) fn questions(
    primary: &Path,
    answers: &BTreeMap<String, SharingAnswer>,
) -> Vec<SharingQuestion> {
    entries(primary)
        .into_iter()
        .filter(|name| classify(name, answers) == Sharing::Unknown)
        .map(|item| {
            let path = primary.join(&item);
            SharingQuestion {
                is_dir: path.is_dir(),
                path,
                item,
            }
        })
        .collect()
}
