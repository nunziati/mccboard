use std::io;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde::Serialize;

/// The port through which the core reaches everything outside the filesystem.
///
/// One real implementation per OS ([`SystemEnvironment`]) and one fake for tests
/// (`testing::FakeEnvironment`). Later features add to it: the usage endpoint call,
/// running `claude` commands, listing processes, checking Windows Developer Mode.
pub trait Environment: Send + Sync {
    /// The current time.
    fn now(&self) -> Timestamp;

    /// The operating system of the local machine. It decides, among other things, what a
    /// launcher is: a shell alias on Linux, scripts on the user PATH on Windows.
    fn os(&self) -> OsKind;

    /// The user's login shell (`$SHELL`), which picks the rc file launchers go in on Linux.
    fn login_shell(&self) -> Option<PathBuf>;

    /// Whether `dir` is on the user PATH that new terminals get (on Windows, the `Path` value
    /// under `HKCU\Environment`).
    fn user_path_contains(&self, dir: &Path) -> io::Result<bool>;

    /// Appends `dir` to the user PATH that new terminals get, and tells running programs that
    /// it changed. Terminals already open keep their PATH.
    fn add_to_user_path(&self, dir: &Path) -> io::Result<()>;
}

/// The operating system family of a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OsKind {
    Linux,
    Windows,
}

/// The real environment of the machine mccboard runs on.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnvironment;

impl Environment for SystemEnvironment {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    fn os(&self) -> OsKind {
        if cfg!(windows) {
            OsKind::Windows
        } else {
            OsKind::Linux
        }
    }

    fn login_shell(&self) -> Option<PathBuf> {
        std::env::var_os("SHELL")
            .filter(|shell| !shell.is_empty())
            .map(PathBuf::from)
    }

    #[cfg(windows)]
    fn user_path_contains(&self, dir: &Path) -> io::Result<bool> {
        crate::windows_user_path::contains(dir)
    }

    #[cfg(not(windows))]
    fn user_path_contains(&self, _dir: &Path) -> io::Result<bool> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the user PATH is managed on Windows only",
        ))
    }

    /// Only ever adds mccboard's own bin folder in the real home (`~/.mccboard/bin`): a core
    /// working in another home (`mccboard --home`, tests) never changes the user's PATH.
    #[cfg(windows)]
    fn add_to_user_path(&self, dir: &Path) -> io::Result<()> {
        let own = std::env::home_dir().map(|home| crate::Paths::new(&home).bin_dir());
        if !own.is_some_and(|own| crate::paths::same(&own, dir)) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "only mccboard's bin folder in your home is added to the user PATH",
            ));
        }
        crate::windows_user_path::add(dir)
    }

    #[cfg(not(windows))]
    fn add_to_user_path(&self, _dir: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the user PATH is managed on Windows only",
        ))
    }
}
