use std::path::{Path, PathBuf};

/// Errors from core operations. Front ends translate them by [`Error::code`]; messages carry
/// paths and mccboard's own settings only, never the contents of Claude Code's files, so no
/// credential can leak through them.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("cannot read the home directory {}: {source}", path.display())]
    HomeUnreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot read {}: {source}", path.display())]
    ConfigUnreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{} is not valid ({detail}); fix it by hand, mccboard won't overwrite it",
        path.display()
    )]
    ConfigCorrupt { path: PathBuf, detail: String },

    #[error("cannot write {}: {source}", path.display())]
    ConfigUnwritable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{} is a relative path; give a full path or one starting with ~", path.display())]
    RelativePath { path: PathBuf },

    #[error("{} is not a folder", path.display())]
    NotADirectory { path: PathBuf },

    #[error("{} is not a registered instance", path.display())]
    NotRegistered { path: PathBuf },

    #[error("no registered account {account}")]
    AccountNotFound { account: String },

    #[error("{label:?} is not a valid label: use 1 to 40 characters")]
    InvalidLabel { label: String },

    #[error("another account already has the label {label:?}")]
    LabelTaken { label: String },

    #[error("{color:?} is not a color: use #rrggbb")]
    InvalidColor { color: String },

    #[error(
        "{name:?} is not a valid launcher name: use letters, digits, '-', '_' and '.', \
         and not \"claude\""
    )]
    InvalidLauncher { name: String },

    #[error(
        "{item:?} takes no sharing answer: give the name of an unknown item in the primary \
         instance's folder, not one in the sharing table"
    )]
    InvalidSharingItem { item: String },
}

impl Error {
    /// A stable identifier for front ends (i18n keys, `--json` output).
    pub fn code(&self) -> &'static str {
        match self {
            Error::HomeUnreadable { .. } => "home_unreadable",
            Error::ConfigUnreadable { .. } => "config_unreadable",
            Error::ConfigCorrupt { .. } => "config_corrupt",
            Error::ConfigUnwritable { .. } => "config_unwritable",
            Error::RelativePath { .. } => "relative_path",
            Error::NotADirectory { .. } => "not_a_directory",
            Error::NotRegistered { .. } => "not_registered",
            Error::AccountNotFound { .. } => "account_not_found",
            Error::InvalidLabel { .. } => "invalid_label",
            Error::LabelTaken { .. } => "label_taken",
            Error::InvalidColor { .. } => "invalid_color",
            Error::InvalidLauncher { .. } => "invalid_launcher",
            Error::InvalidSharingItem { .. } => "invalid_sharing_item",
        }
    }

    /// The path the error is about, if any, for front ends to show next to the translation.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Error::HomeUnreadable { path, .. }
            | Error::ConfigUnreadable { path, .. }
            | Error::ConfigCorrupt { path, .. }
            | Error::ConfigUnwritable { path, .. }
            | Error::RelativePath { path }
            | Error::NotADirectory { path }
            | Error::NotRegistered { path } => Some(path),
            _ => None,
        }
    }

    /// Where `config.toml` is wrong (line and reason, from mccboard's own settings), for
    /// [`Error::ConfigCorrupt`].
    pub fn detail(&self) -> Option<&str> {
        match self {
            Error::ConfigCorrupt { detail, .. } => Some(detail),
            _ => None,
        }
    }
}
