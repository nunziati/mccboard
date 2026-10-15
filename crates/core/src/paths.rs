use std::path::{Path, PathBuf};

/// Where mccboard keeps its own files: `~/.mccboard/`, never inside `~/.claude`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub(crate) fn new(home: &Path) -> Self {
        Self {
            root: home.join(".mccboard"),
        }
    }

    /// `~/.mccboard/`
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `~/.mccboard/config.toml`: hand-editable settings and registrations.
    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    /// `~/.mccboard/state/`: data mccboard writes on its own (readings, backoff, backups).
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }
}
