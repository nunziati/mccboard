//! Backups of real files and folders that a change is about to replace (with a link, for
//! example). They go to `~/.mccboard/state/backups/<UTC time>/<instance>/<item>`, a fresh
//! folder per run, so nothing is ever overwritten; nothing is deleted.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiff::Timestamp;

/// One run's backups of one instance. The folder is created on the first item kept, so a run
/// that backs nothing up leaves nothing behind.
pub(crate) struct Backup {
    backups_dir: PathBuf,
    stamp: String,
    instance: String,
    dir: Option<PathBuf>,
}

impl Backup {
    /// Backups of the instance in `config_dir` made at `now`, under `backups_dir`.
    pub(crate) fn new(
        backups_dir: PathBuf,
        now: Timestamp,
        home: &Path,
        config_dir: &Path,
    ) -> Self {
        Self {
            backups_dir,
            stamp: now.strftime("%Y%m%dT%H%M%SZ").to_string(),
            instance: instance_name(home, config_dir),
            dir: None,
        }
    }

    /// Moves the real item at `path` (`item`, relative to the config directory with `/`
    /// separators) into the backup. Returns where it is now.
    pub(crate) fn keep(&mut self, path: &Path, item: &str) -> io::Result<PathBuf> {
        let mut destination = self.dir()?;
        destination.extend(item.split('/'));
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(path, &destination)?;
        Ok(destination)
    }

    /// Puts an item kept by [`Backup::keep`] back where it was.
    pub(crate) fn restore(&self, kept: &Path, path: &Path) -> io::Result<()> {
        fs::rename(kept, path)
    }

    fn dir(&mut self) -> io::Result<PathBuf> {
        if let Some(dir) = &self.dir {
            return Ok(dir.clone());
        }
        // Another run in the same second gets its own folder.
        let mut n = 1;
        let dir = loop {
            let stamp = if n == 1 {
                self.stamp.clone()
            } else {
                format!("{}-{n}", self.stamp)
            };
            let dir = self.backups_dir.join(stamp).join(&self.instance);
            if !dir.exists() {
                break dir;
            }
            n += 1;
        };
        fs::create_dir_all(&dir)?;
        self.dir = Some(dir.clone());
        Ok(dir)
    }
}

/// A folder name for the instance: its path relative to the home (`.claude-b`), or its full
/// path with separators replaced.
fn instance_name(home: &Path, config_dir: &Path) -> String {
    let relative = config_dir.strip_prefix(home).unwrap_or(config_dir);
    let name: String = relative
        .to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim_matches('_');
    if name.is_empty() || name == "." || name == ".." {
        "instance".to_owned()
    } else {
        name.to_owned()
    }
}
