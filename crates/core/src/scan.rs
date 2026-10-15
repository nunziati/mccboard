use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::instance::{self, InstanceKind, Observation};
use crate::{Environment, Error};

/// A config directory found by a scan and proposed as an instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Candidate {
    pub config_dir: PathBuf,
    pub kind: InstanceKind,
    #[serde(flatten)]
    pub observation: Observation,
}

pub(crate) fn scan(home: &Path, env: &dyn Environment) -> Result<Vec<Candidate>, Error> {
    let unreadable = |source| Error::HomeUnreadable {
        path: home.to_path_buf(),
        source,
    };

    let mut linked = Vec::new();
    let mut primary = None;
    for entry in fs::read_dir(home).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let is_primary = name == ".claude";
        let is_linked = name.strip_prefix(".claude-").is_some_and(|s| !s.is_empty());
        // `is_dir` follows links, so a config directory reached through a link still counts.
        if !(is_primary || is_linked) || !entry.path().is_dir() {
            continue;
        }
        if is_primary {
            primary = Some(entry.path());
        } else {
            linked.push(entry.path());
        }
    }
    linked.sort();

    Ok(primary
        .into_iter()
        .chain(linked)
        .map(|config_dir| Candidate {
            kind: InstanceKind::of(home, &config_dir),
            observation: instance::observe(home, &config_dir, env),
            config_dir,
        })
        .collect())
}
