//! The Global config keys of `.claude.json`: preferences Claude Code keeps per instance that
//! should behave the same in every instance (agent view, editor, copy behaviour).
//!
//! `.claude.json` also holds the instance's account, projects and Claude Code's bookkeeping,
//! all private: mccboard reads the listed keys only and, when aligning, rewrites the file
//! with only those keys changed, every other key kept in its place.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::{Map, Value};

use crate::InstanceKind;
use crate::claude_files::global_config_path;
use crate::doctor::{DriftItem, Problem, Repair};

/// The keys of `.claude.json` that must match the primary instance. Extend it as Claude Code
/// adds preferences worth sharing; keys holding credentials or environment (`apiKeyHelper`,
/// `env`) stay private.
pub const GLOBAL_CONFIG_KEYS: &[&str] = &[
    "defaultToAgentsView",
    "leftArrowOpensAgents",
    "autoConnectIde",
    "autoInstallIdeExtension",
    "diffTool",
    "claudeInChromeDefaultEnabled",
    "copyFullResponse",
    "copyOnSelect",
    "externalEditorContext",
    "prStatusFooterEnabled",
];

/// Keys the primary sets that the linked instance in `config_dir` doesn't, or sets to
/// another value. A key the primary doesn't set is left to each instance. An unreadable
/// `.claude.json`, in either instance, shows no key drift: it is never overwritten.
pub(crate) fn drift(home: &Path, config_dir: &Path) -> Vec<DriftItem> {
    let primary_path = global_config_path(home, &home.join(".claude"), InstanceKind::Primary);
    let Ok(Some(primary)) = read(&primary_path) else {
        return Vec::new();
    };
    let path = global_config_path(home, config_dir, InstanceKind::Linked);
    let Ok(linked) = read(&path) else {
        return Vec::new();
    };
    let linked = linked.unwrap_or_default();

    GLOBAL_CONFIG_KEYS
        .iter()
        .filter_map(|key| {
            let wanted = primary.get(*key)?;
            let found = linked.get(*key);
            if found == Some(wanted) {
                return None;
            }
            Some(DriftItem::new(
                format!("config:{key}"),
                key,
                &path,
                Problem::MisalignedKey {
                    key: (*key).to_owned(),
                    primary: wanted.clone(),
                    instance: found.cloned(),
                },
                Repair::AlignKey {
                    key: (*key).to_owned(),
                    value: wanted.clone(),
                },
            ))
        })
        .collect()
}

/// Sets `key` to `value` in the global config at `path`, keeping every other key, in its
/// place, and the file's layout (two-space indentation, as Claude Code writes it). Creates
/// the file when there is none. Refuses to touch a file it can't parse.
pub(crate) fn align_key(path: &Path, key: &str, value: &Value) -> io::Result<()> {
    let existing = fs::read_to_string(path);
    let (mut map, newline) = match &existing {
        Ok(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => (map, text.ends_with('\n')),
            // Never show what the file holds: it is Claude Code's, and private.
            _ => return Err(unreadable(path)),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => (Map::new(), true),
        Err(_) => return Err(unreadable(path)),
    };
    map.insert(key.to_owned(), value.clone());

    let mut text = serde_json::to_string_pretty(&Value::Object(map)).map_err(io::Error::other)?;
    if newline {
        text.push('\n');
    }
    write_replacing(path, &text)
}

/// Writes the file next to `path` first, then moves it in place, so an interrupted write
/// never leaves half a file. The new file keeps the old one's permissions.
fn write_replacing(path: &Path, text: &str) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".mccboard-tmp");
    let temporary = Path::new(&temporary);
    fs::write(temporary, text)?;
    if let Ok(meta) = fs::metadata(path) {
        let _ = fs::set_permissions(temporary, meta.permissions());
    }
    fs::rename(temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(temporary);
    })
}

fn unreadable(path: &Path) -> io::Error {
    io::Error::other(format!(
        "{} can't be read as JSON; it was left as it is",
        path.display()
    ))
}

/// The global config at `path` as a JSON object: `None` when there is no file, an error when
/// it can't be read or isn't a JSON object.
fn read(path: &Path) -> io::Result<Option<Map<String, Value>>> {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(Value::Object(map)) => Ok(Some(map)),
            _ => Err(unreadable(path)),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
