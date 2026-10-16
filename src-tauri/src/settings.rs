//! Settings commands: the app's language and whether Claude Code may renew an expired token,
//! saved in `config.toml`.

use mccboard_core::{Core, Language};

use crate::CommandError;

/// The app's language: the one saved in `config.toml`, else the system's.
#[tauri::command]
pub(crate) fn language(core: tauri::State<'_, Core>) -> Language {
    crate::tray::language(&core)
}

/// Saves the app's language and redraws the tray in it. The windows follow through the
/// config watcher.
#[tauri::command]
pub(crate) fn set_language(
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
    language: Language,
) -> Result<(), CommandError> {
    core.set_language(language)?;
    crate::tray::redraw(&app);
    Ok(())
}

/// Whether a usage refresh may ask Claude Code to renew an expired token.
#[tauri::command]
pub(crate) fn renew_expired_tokens(core: tauri::State<'_, Core>) -> Result<bool, CommandError> {
    Ok(core.renew_expired_tokens()?)
}

/// Saves whether a usage refresh may ask Claude Code to renew an expired token, and redraws
/// the tray's menu with it. The windows follow through the config watcher.
#[tauri::command]
pub(crate) fn set_renew_expired_tokens(
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
    on: bool,
) -> Result<(), CommandError> {
    core.set_renew_expired_tokens(on)?;
    crate::tray::redraw(&app);
    Ok(())
}
