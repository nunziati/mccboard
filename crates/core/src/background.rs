//! Child processes mccboard runs in the background: no input, output piped, nothing for the
//! user to see. On Windows they open no console window: the app has no console of its own, so
//! Windows would otherwise show a new one for each child. The CLI loses nothing either, since
//! these children read and write only through their pipes.
//!
//! Children the user interacts with (the login) are built elsewhere.

use std::ffi::OsStr;
use std::process::Command;

/// A command for `program` to run in the background.
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATION_FLAGS);
    }
    command
}

/// A console without a window, which the child's own console children (git, cmd…) share
/// instead of opening theirs.
#[cfg(windows)]
const CREATION_FLAGS: u32 = windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

#[cfg(all(test, windows))]
mod tests {
    use windows_sys::Win32::System::Threading::{
        CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, DETACHED_PROCESS,
    };

    use super::*;

    #[test]
    fn on_windows_a_background_child_gets_a_console_without_a_window() {
        assert_ne!(CREATION_FLAGS & CREATE_NO_WINDOW, 0);
        // Either would undo it: a visible console, or none at all for children to share.
        assert_eq!(CREATION_FLAGS & (CREATE_NEW_CONSOLE | DETACHED_PROCESS), 0);
    }
}
