//! Test support: the fake environment port. Enabled by the `testing` feature.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use jiff::{SignedDuration, Timestamp};

use crate::{Environment, OsKind};

/// A scriptable [`Environment`] for tests. Clones share state, so a test can keep a handle
/// after giving one to [`crate::Core::new`] and keep steering it.
///
/// It starts as a Linux machine with no login shell and an empty user PATH, whatever OS the
/// tests run on.
#[derive(Debug, Clone)]
pub struct FakeEnvironment {
    state: Arc<Mutex<State>>,
}

#[derive(Debug)]
struct State {
    now: Timestamp,
    os: OsKind,
    login_shell: Option<PathBuf>,
    user_path: Vec<PathBuf>,
    user_path_additions: usize,
    user_path_fails: bool,
}

impl FakeEnvironment {
    /// A fake whose clock reads `now` until moved.
    pub fn new(now: Timestamp) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                now,
                os: OsKind::Linux,
                login_shell: None,
                user_path: Vec::new(),
                user_path_additions: 0,
                user_path_fails: false,
            })),
        }
    }

    /// Sets the clock.
    pub fn set_now(&self, now: Timestamp) {
        self.state().now = now;
    }

    /// Moves the clock forward (or backward, with a negative duration).
    pub fn advance(&self, by: SignedDuration) {
        let mut state = self.state();
        state.now += by;
    }

    /// Makes the fake machine run `os`.
    pub fn set_os(&self, os: OsKind) {
        self.state().os = os;
    }

    /// Sets the login shell (`$SHELL`), e.g. `/usr/bin/zsh`; `None` when unset.
    pub fn set_login_shell(&self, shell: Option<&str>) {
        self.state().login_shell = shell.map(PathBuf::from);
    }

    /// The fake user PATH, in order.
    pub fn user_path(&self) -> Vec<PathBuf> {
        self.state().user_path.clone()
    }

    /// How many times a folder was added to the user PATH.
    pub fn user_path_additions(&self) -> usize {
        self.state().user_path_additions
    }

    /// Makes reading and changing the user PATH fail, as a locked-down registry would.
    pub fn set_user_path_fails(&self, fails: bool) {
        self.state().user_path_fails = fails;
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Environment for FakeEnvironment {
    fn now(&self) -> Timestamp {
        self.state().now
    }

    fn os(&self) -> OsKind {
        self.state().os
    }

    fn login_shell(&self) -> Option<PathBuf> {
        self.state().login_shell.clone()
    }

    fn user_path_contains(&self, dir: &Path) -> io::Result<bool> {
        let state = self.state();
        if state.user_path_fails {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fake: the user PATH can't be read",
            ));
        }
        Ok(state
            .user_path
            .iter()
            .any(|entry| crate::paths::same(entry, dir)))
    }

    fn add_to_user_path(&self, dir: &Path) -> io::Result<()> {
        let mut state = self.state();
        if state.user_path_fails {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fake: the user PATH can't be changed",
            ));
        }
        state.user_path.push(dir.to_path_buf());
        state.user_path_additions += 1;
        Ok(())
    }
}
