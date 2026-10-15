//! Usage commands: the core's usage readings, for the tray (#8) and the windows.

use mccboard_core::{Core, UsageReport};

use crate::CommandError;

/// The last known readings, without calling the endpoint.
#[tauri::command]
pub(crate) fn usage(core: tauri::State<'_, Core>) -> Result<UsageReport, CommandError> {
    Ok(core.usage()?)
}

/// Refreshes usage within the rate limits, then reports it. The endpoint call blocks, so it
/// runs off the main thread.
#[tauri::command]
pub(crate) async fn refresh_usage(
    core: tauri::State<'_, Core>,
) -> Result<UsageReport, CommandError> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.refresh_usage())
        .await
        .map_err(|_| CommandError::unknown())?
        .map_err(CommandError::from)
}
