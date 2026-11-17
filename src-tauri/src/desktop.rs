//! Desktop profile commands (ADR 0008, spec #34), thin wrappers over the core, and the one way
//! the app opens a profile: from the Instances window, the tray menu, a Start-menu shortcut or
//! a pin, and at start.

use std::fmt::Display;
use std::path::PathBuf;

use mccboard_core::{
    Account, AccountUuid, Core, DesktopOpening, DesktopProfile, DesktopProfileRemoval,
    DesktopProfileUpdate, RemoveLevel, Saved,
};
use tauri::{AppHandle, Manager};

use crate::CommandError;

/// Opens the desktop profile of `account`, or the default desktop profile for `None`.
/// Every opening in the app goes through here, a profile's Start-menu shortcut included;
/// afterwards the taskbar watcher looks at once, to tag the new windows.
pub(crate) fn open(
    core: &Core,
    account: Option<&AccountUuid>,
) -> Result<DesktopOpening, mccboard_core::Error> {
    let opening = match account {
        Some(account) => core.open_desktop_profile(account),
        None => core.open_default_desktop_profile(),
    };
    if opening.is_ok() {
        crate::taskbar::refresh();
    }
    opening
}

/// Opens the desktop profile of `account` (the default one for `None`) off the main thread:
/// finding and starting the app takes a moment. The way the tray, a Start-menu shortcut or a
/// pin, and a notice's "Reopen" open a profile. A profile with no account signed in yet shows
/// the Instances window, with its first sign-in guide. A failure goes to stderr. `then` runs
/// afterwards, whatever the outcome.
pub(crate) fn open_in_background(
    app: &AppHandle,
    account: Option<AccountUuid>,
    then: impl FnOnce(&AppHandle) + Send + 'static,
) {
    let app = app.clone();
    in_background(
        "mccboard-open-profile",
        "cannot open the desktop profile",
        move || {
            let opened = open(&app.state::<Core>(), account.as_ref());
            if opened.as_ref().is_ok_and(|opening| opening.needs_sign_in) {
                show_sign_in_guide(&app);
            }
            then(&app);
            opened.map(drop)
        },
    );
}

/// Opens, off the main thread, the profiles set to open when mccboard starts. Called once,
/// from the app's setup. A profile that can't open is reported on stderr; one with no account
/// signed in yet shows the Instances window, with its first sign-in guide.
pub(crate) fn open_at_start(app: &AppHandle) {
    let app = app.clone();
    in_background(
        "mccboard-open-at-start",
        "cannot open desktop profiles at start",
        move || {
            let openings = app.state::<Core>().open_desktop_profiles_at_start()?;
            crate::taskbar::refresh();
            let mut needs_sign_in = false;
            for opening in openings {
                match opening.result {
                    Ok(opened) => needs_sign_in |= opened.needs_sign_in,
                    Err(error) => {
                        let name = opening.label.unwrap_or_else(|| opening.account.to_string());
                        eprintln!("mccboard: cannot open desktop profile {name}: {error}");
                    }
                }
            }
            if needs_sign_in {
                show_sign_in_guide(&app);
            }
            Ok::<(), mccboard_core::Error>(())
        },
    );
}

/// Shows the Instances window, where the first sign-in guide of a profile that runs with no
/// account signed in yet shows up.
fn show_sign_in_guide(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || crate::tray::open_instances(&handle));
}

/// Runs `work` on a new thread named `thread`; if the thread can't start or `work` fails, says
/// so on stderr as `mccboard: <failure>: <error>`.
pub(crate) fn in_background<E: Display>(
    thread: &str,
    failure: &'static str,
    work: impl FnOnce() -> Result<(), E> + Send + 'static,
) {
    let spawned = std::thread::Builder::new()
        .name(thread.into())
        .spawn(move || {
            if let Err(error) = work() {
                eprintln!("mccboard: {failure}: {error}");
            }
        });
    if let Err(error) = spawned {
        eprintln!("mccboard: {failure}: {error}");
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

/// Whether desktop profiles can open on this platform (Windows only). Elsewhere the window
/// shows no profile to open.
#[tauri::command]
pub(crate) fn desktop_profiles_supported(core: tauri::State<'_, Core>) -> bool {
    core.desktop_profiles_supported()
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

/// Adds a desktop profile for the account `uuid`, in `folder` or the default one. A Start-menu
/// shortcut that can't follow is a warning in the result.
#[tauri::command]
pub(crate) fn add_desktop_profile(
    core: tauri::State<'_, Core>,
    uuid: String,
    folder: Option<PathBuf>,
) -> Result<Saved<DesktopProfile>, CommandError> {
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
/// in it). Lists the desktop app's processes, so it runs off the main thread. A Start-menu
/// shortcut that can't follow is a warning in the result.
#[tauri::command]
pub(crate) async fn remove_desktop_profile(
    core: tauri::State<'_, Core>,
    uuid: String,
    delete_folder: bool,
) -> Result<Saved<DesktopProfileRemoval>, CommandError> {
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
