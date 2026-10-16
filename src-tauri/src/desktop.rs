//! Desktop profile commands (ADR 0008, spec #34), thin wrappers over the core, and the one way
//! the app opens a profile: from the Instances window, the tray menu, and at start.

use std::path::PathBuf;

use mccboard_core::{
    Account, AccountUuid, Core, DesktopOpening, DesktopProfile, DesktopProfileRemoval,
    DesktopProfileUpdate, RemoveLevel,
};

use crate::CommandError;

/// Opens the desktop profile of `account`, or the default desktop profile for `None`.
/// Every opening in the app goes through here.
pub(crate) fn open(
    core: &Core,
    account: Option<&AccountUuid>,
) -> Result<DesktopOpening, mccboard_core::Error> {
    match account {
        Some(account) => core.open_desktop_profile(account),
        None => core.open_default_desktop_profile(),
    }
}

/// Opens, off the main thread, the profiles set to open when mccboard starts. Called once,
/// from the app's setup. A profile that can't open is reported on stderr.
pub(crate) fn open_at_start(core: &Core) {
    let core = core.clone();
    let spawned = std::thread::Builder::new()
        .name("mccboard-open-at-start".into())
        .spawn(move || match core.open_desktop_profiles_at_start() {
            Ok(openings) => {
                for opening in openings {
                    if let Err(error) = opening.result {
                        let name = opening.label.unwrap_or_else(|| opening.account.to_string());
                        eprintln!("mccboard: cannot open desktop profile {name}: {error}");
                    }
                }
            }
            Err(error) => eprintln!("mccboard: cannot open desktop profiles at start: {error}"),
        });
    if let Err(error) = spawned {
        eprintln!("mccboard: cannot open desktop profiles at start: {error}");
    }
}

/// Every desktop profile, the default one first. Lists the desktop app's processes, so it
/// runs off the main thread.
#[tauri::command]
pub(crate) async fn desktop_profiles(
    core: tauri::State<'_, Core>,
) -> Result<Vec<DesktopProfile>, CommandError> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.desktop_profiles())
        .await
        .map_err(|_| CommandError::unknown())?
        .map_err(CommandError::from)
}

/// The accounts that can get a desktop profile.
#[tauri::command]
pub(crate) fn desktop_profile_candidates(
    core: tauri::State<'_, Core>,
) -> Result<Vec<Account>, CommandError> {
    Ok(core.desktop_profile_candidates()?)
}

/// The data folder a new desktop profile for the account `uuid` gets by default.
#[tauri::command]
pub(crate) fn desktop_profile_folder(
    core: tauri::State<'_, Core>,
    uuid: String,
) -> Result<PathBuf, CommandError> {
    Ok(core.desktop_profile_folder(&AccountUuid::new(uuid))?)
}

/// Adds a desktop profile for the account `uuid`, in `folder` or the default one.
#[tauri::command]
pub(crate) fn add_desktop_profile(
    core: tauri::State<'_, Core>,
    uuid: String,
    folder: Option<PathBuf>,
) -> Result<DesktopProfile, CommandError> {
    Ok(core.add_desktop_profile(&AccountUuid::new(uuid), folder.as_deref())?)
}

/// Sets whether the desktop profile of the account `uuid` opens when mccboard starts.
#[tauri::command]
pub(crate) fn update_desktop_profile(
    core: tauri::State<'_, Core>,
    uuid: String,
    open_at_start: bool,
) -> Result<DesktopProfile, CommandError> {
    let update = DesktopProfileUpdate {
        open_at_start: Some(open_at_start),
    };
    Ok(core.update_desktop_profile(&AccountUuid::new(uuid), update)?)
}

/// Removes the desktop profile of the account `uuid` from mccboard; with `delete_folder`,
/// which the UI confirms with the user first, also deletes its data folder (and the sign-in
/// in it). Lists the desktop app's processes, so it runs off the main thread.
#[tauri::command]
pub(crate) async fn remove_desktop_profile(
    core: tauri::State<'_, Core>,
    uuid: String,
    delete_folder: bool,
) -> Result<DesktopProfileRemoval, CommandError> {
    let core = core.inner().clone();
    let level = if delete_folder {
        RemoveLevel::WithFolder
    } else {
        RemoveLevel::FromMccboard
    };
    tauri::async_runtime::spawn_blocking(move || {
        core.remove_desktop_profile(&AccountUuid::new(uuid), level)
    })
    .await
    .map_err(|_| CommandError::unknown())?
    .map_err(CommandError::from)
}
