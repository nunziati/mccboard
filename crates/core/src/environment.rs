use jiff::Timestamp;

use crate::session_guard::ClaudeProcess;

/// The port through which the core reaches everything outside the filesystem.
///
/// One real implementation per OS ([`SystemEnvironment`]) and one fake for tests
/// (`testing::FakeEnvironment`). Later features add to it: the usage endpoint call,
/// running `claude` commands.
pub trait Environment: Send + Sync {
    /// The current time.
    fn now(&self) -> Timestamp;

    /// Every running Claude Code process on this machine, each with the config directory it
    /// uses. The core matches them to instances (the session guard).
    fn claude_processes(&self) -> Vec<ClaudeProcess>;

    /// Whether file links (symlinks to files) can be created without elevation: on Windows,
    /// whether Developer Mode is on (ADR 0006); elsewhere always.
    fn file_links_allowed(&self) -> bool;
}

/// The real environment of the machine mccboard runs on.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnvironment;

impl Environment for SystemEnvironment {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    fn claude_processes(&self) -> Vec<ClaudeProcess> {
        crate::system::claude_processes()
    }

    fn file_links_allowed(&self) -> bool {
        crate::system::file_links_allowed()
    }
}
