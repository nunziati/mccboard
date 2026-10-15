//! Settings commands: for now the app's language, saved in `config.toml`.

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
