//! The terminal front end (ADR 0010): interactive screens drawn with ratatui on the crossterm
//! backend, for a user with a terminal and no desktop. They are front ends like the tray and
//! the Instances window: every step calls [`Core`], and usage comes from the view models of
//! the tray and of the taskbar widget.
//!
//! - [`terminal`]: the guard that holds the terminal in raw mode on the alternate screen and
//!   always gives it back, and the way to hand it to an interactive program.
//! - [`app`]: the loop a screen runs in.
//! - [`texts`]: the labels, in English and Italian.
//! - [`dashboard`]: `mccboard dashboard`.

pub mod app;
mod dashboard;
pub mod terminal;
pub mod texts;

use std::io::IsTerminal;

use anyhow::{Result, bail};
use mccboard_core::{Core, Language};

pub use dashboard::run as dashboard;
pub use texts::Texts;

/// The language the app would show: the one in `config.toml`, else the system's.
pub fn language(core: &Core) -> Language {
    core.language()
        .ok()
        .flatten()
        .unwrap_or_else(|| Language::from_locale(&sys_locale::get_locale().unwrap_or_default()))
}

/// Stops with `message` unless this process can draw on a terminal and read its keys: the
/// TUI needs one, a script or a pipe gets the message instead.
pub fn require_terminal(message: impl FnOnce() -> String) -> Result<()> {
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        Ok(())
    } else {
        bail!(message())
    }
}
