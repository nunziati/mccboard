//! Child processes mccboard runs in the background: no input, output piped, nothing for the
//! user to see.
//!
//! On Windows a console child of a process without a console (the app) gets a new console
//! window of its own, so there it starts with a console that has no window. A child of the CLI
//! keeps sharing the CLI's console: no window opens, and Ctrl+C still reaches it.
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
        // SAFETY: takes no arguments and only reads this process's console.
        let window = unsafe { windows_sys::Win32::System::Console::GetConsoleWindow() };
        command.creation_flags(creation_flags(!window.is_null()));
    }
    command
}

/// The creation flags of a background child: none when mccboard has a console for it to
/// share, else a console without a window, which the child's own console children (git,
/// cmd…) share instead of opening theirs.
#[cfg(windows)]
fn creation_flags(has_console: bool) -> u32 {
    if has_console {
        0
    } else {
        windows_sys::Win32::System::Threading::CREATE_NO_WINDOW
    }
}

#[cfg(all(test, windows))]
mod tests {
    use windows_sys::Win32::System::Threading::{
        CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, DETACHED_PROCESS,
    };

    use super::*;

    #[test]
    fn on_windows_a_background_child_of_the_app_gets_a_console_without_a_window() {
        let flags = creation_flags(false);
        assert_ne!(flags & CREATE_NO_WINDOW, 0);
        // Either would undo it: a visible console, or none at all for children to share.
        assert_eq!(flags & (CREATE_NEW_CONSOLE | DETACHED_PROCESS), 0);
    }

    #[test]
    fn on_windows_a_background_child_of_the_cli_shares_its_console() {
        assert_eq!(creation_flags(true), 0);
    }
}
