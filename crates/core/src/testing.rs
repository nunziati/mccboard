//! Test support: the fake environment port. Enabled by the `testing` feature.

use std::sync::{Arc, Mutex, MutexGuard};

use jiff::{SignedDuration, Timestamp};

use crate::{ClaudeProcess, Environment};

/// A scriptable [`Environment`] for tests. Clones share state, so a test can keep a handle
/// after giving one to [`crate::Core::new`] and keep steering it.
#[derive(Debug, Clone)]
pub struct FakeEnvironment {
    state: Arc<Mutex<State>>,
}

#[derive(Debug)]
struct State {
    now: Timestamp,
    processes: Vec<ClaudeProcess>,
    developer_mode: bool,
}

impl FakeEnvironment {
    /// A fake whose clock reads `now` until moved, with no Claude Code process running and
    /// Developer Mode on.
    pub fn new(now: Timestamp) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                now,
                processes: Vec::new(),
                developer_mode: true,
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

    /// Replaces the running Claude Code processes.
    pub fn set_processes(&self, processes: Vec<ClaudeProcess>) {
        self.state().processes = processes;
    }

    /// Starts one more Claude Code process.
    pub fn add_process(&self, process: ClaudeProcess) {
        self.state().processes.push(process);
    }

    /// Turns Windows Developer Mode on or off: off, file links can't be created.
    pub fn set_developer_mode(&self, on: bool) {
        self.state().developer_mode = on;
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

    fn claude_processes(&self) -> Vec<ClaudeProcess> {
        self.state().processes.clone()
    }

    fn file_links_allowed(&self) -> bool {
        self.state().developer_mode
    }
}
