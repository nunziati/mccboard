//! Settings commands: the app's language, whether Claude Code may renew an expired token, and
//! the taskbar widget's preferences, saved in `config.toml`.

use mccboard_core::{Core, Language};
use serde::Serialize;

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

/// The taskbar widget's preferences, and whether this platform has the widget at all.
#[derive(Serialize)]
pub(crate) struct TaskbarWidgetSettings {
    /// The widget exists on Windows only; elsewhere the UI hides its preferences.
    supported: bool,
    /// "Show the widget on the taskbar".
    on: bool,
    /// "Show account labels".
    labels: bool,
}

#[tauri::command]
pub(crate) fn taskbar_widget(
    core: tauri::State<'_, Core>,
) -> Result<TaskbarWidgetSettings, CommandError> {
    Ok(TaskbarWidgetSettings {
        supported: cfg!(windows),
        on: core.taskbar_widget_on()?,
        labels: core.taskbar_widget_labels()?,
    })
}

/// Saves the taskbar widget's preferences given, and redraws the tray, which feeds the widget.
/// The windows follow through the config watcher.
#[tauri::command]
pub(crate) fn set_taskbar_widget(
    app: tauri::AppHandle,
    core: tauri::State<'_, Core>,
    on: Option<bool>,
    labels: Option<bool>,
) -> Result<(), CommandError> {
    if let Some(on) = on {
        core.set_taskbar_widget_on(on)?;
    }
    if let Some(labels) = labels {
        core.set_taskbar_widget_labels(labels)?;
    }
    crate::tray::redraw(&app);
    Ok(())
}
