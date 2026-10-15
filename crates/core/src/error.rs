use std::path::PathBuf;

/// Errors from core operations. Front ends translate them by [`Error::code`]; messages carry
/// paths only, never file contents, so no credential can leak through them.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("cannot read the home directory {}: {source}", path.display())]
    HomeUnreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl Error {
    /// A stable identifier for front ends (i18n keys, `--json` output).
    pub fn code(&self) -> &'static str {
        match self {
            Error::HomeUnreadable { .. } => "home_unreadable",
        }
    }
}
