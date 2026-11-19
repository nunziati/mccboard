//! The terminal front end (ADR 0010): interactive screens drawn with ratatui on the crossterm
//! backend, for a user with a terminal and no desktop. They are front ends like the tray and
//! the Instances window: every step calls [`Core`], and usage comes from the view models of
//! the tray and of the taskbar widget.
//!
//! - [`terminal`]: the guard that holds the terminal in raw mode on the alternate screen and
//!   always gives it back, and the way to hand it to an interactive program.
//! - [`app`]: the loop a screen runs in.
//! - [`texts`]: the labels, in English and Italian.
//! - [`widgets`]: pieces of drawing the screens share.
//! - [`repairs`]: repairs on threads of their own, waited for before a screen quits.
//! - [`dashboard`]: `mccboard dashboard`.
//! - [`setup`]: `mccboard setup`.
//!
//! The two screens open each other in the same terminal ([`show`]): `s` in the dashboard
//! opens the setup, and the setup ends on the dashboard.

pub mod app;
mod dashboard;
mod repairs;
mod setup;
pub mod terminal;
pub mod texts;
mod widgets;

use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Result, bail};
use mccboard_core::{Core, Language};
use ratatui::backend::Backend;

use app::Events;
pub use dashboard::run as dashboard;
use setup::Console;
pub use setup::run as setup;
use terminal::Tui;
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

/// What the TUI shows next, once a screen ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    Dashboard,
    Setup,
    Quit,
}

/// Shows `first` in `tui`, then each screen the user opens from the one before, until one
/// quits. Each screen starts in the language mccboard uses at that moment: the setup may
/// have changed it. `q` in a setup opened from the dashboard goes back to the dashboard.
pub fn show<B>(
    core: &Core,
    tui: &mut Tui<B>,
    events: &mut dyn Events,
    console: &mut dyn Console,
    first: Show,
) -> Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
{
    let mut next = first;
    // The screen shown before: `Quit` for the first one.
    let mut before = Show::Quit;
    loop {
        let shown = next;
        next = match shown {
            Show::Dashboard => dashboard::show(core, tui, events)?,
            Show::Setup => setup::show(core, tui, events, console, before)?,
            Show::Quit => return Ok(()),
        };
        before = shown;
    }
}

/// `path` as shown: `~/…` under `home`, else whole.
pub fn shown_path(home: &Path, path: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}
