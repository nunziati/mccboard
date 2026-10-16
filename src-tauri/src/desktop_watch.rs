//! The desktop profile watch (ticket #40): polls the core every few seconds for a profile the
//! app reopened or closed on its own, shows the notices in the tray (a tooltip line and a
//! "Reopen B" menu item) and sends them to the windows. Reopening is the usual opening of a
//! profile; nothing here ever closes a process.

use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use mccboard_core::{AccountUuid, Core, DesktopNotices};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager};

use crate::CommandError;

/// How often the app looks at the desktop app's processes. A profile that vanished pairs with
/// a new default copy within a minute, so a few seconds keep both in view.
const POLL_EVERY: Duration = Duration::from_secs(5);

/// Emitted to the windows with the notices whenever they change.
const DESKTOP_NOTICES: &str = "desktop-notices";

/// Menu ids of the "Reopen B" items: the prefix, then the account UUID.
const REOPEN_PREFIX: &str = "reopen-profile:";

/// Managed state: the way to wake the watch thread, and the notices as of its last poll.
struct Watch {
    wake: Mutex<Sender<()>>,
    notices: Mutex<Option<DesktopNotices>>,
}

/// Starts the thread that polls the core.
pub(crate) fn setup(app: &tauri::App, core: &Core) -> tauri::Result<()> {
    let (sender, receiver) = mpsc::channel();
    app.manage(Watch {
        wake: Mutex::new(sender),
        notices: Mutex::new(None),
    });
    let handle = app.handle().clone();
    let core = core.clone();
    std::thread::Builder::new()
        .name("mccboard-desktop-watch".into())
        .spawn(move || run(&handle, &core, &receiver))?;
    Ok(())
}

fn run(app: &AppHandle, core: &Core, wakes: &Receiver<()>) {
    loop {
        let language = crate::tray::language(core);
        // A corrupt config.toml shows in the tray and the window already.
        if let Ok(notices) = core.poll_desktop_profiles(language) {
            publish(app, notices);
        }
        match wakes.recv_timeout(POLL_EVERY) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Keeps `notices` and, when they changed, tells the windows and redraws the tray.
fn publish(app: &AppHandle, notices: DesktopNotices) {
    let Some(watch) = app.try_state::<Watch>() else {
        return;
    };
    let mut last = watch.notices.lock().unwrap_or_else(|p| p.into_inner());
    if last.as_ref() == Some(&notices) {
        return;
    }
    let _ = app.emit(DESKTOP_NOTICES, &notices);
    *last = Some(notices);
    drop(last);
    crate::tray::redraw(app);
}

/// Polls again now: after a reopen or a dismissal.
fn wake(app: &AppHandle) {
    if let Some(watch) = app.try_state::<Watch>() {
        let sender = watch.wake.lock().unwrap_or_else(|p| p.into_inner());
        let _ = sender.send(());
    }
}

fn last(app: &AppHandle) -> Option<DesktopNotices> {
    let watch = app.try_state::<Watch>()?;
    let notices = watch.notices.lock().unwrap_or_else(|p| p.into_inner());
    notices.clone()
}

/// The tray tooltip: the notices' lines first, then what fits of the usage tooltip.
pub(crate) fn tray_tooltip(app: &AppHandle, usage: &str) -> String {
    match last(app) {
        Some(notices) => notices.tray_tooltip(usage),
        None => usage.to_owned(),
    }
}

/// Puts each notice at the top of the tray menu: its line, then "Reopen B" (disabled while
/// the stray copy still runs), then a separator.
pub(crate) fn append_to_menu(app: &AppHandle, menu: &Menu<tauri::Wry>) -> tauri::Result<()> {
    let Some(notices) = last(app) else {
        return Ok(());
    };
    if notices.notices.is_empty() {
        return Ok(());
    }
    for notice in &notices.notices {
        menu.append(&MenuItem::new(
            app,
            crate::tray::escape(&notice.tooltip),
            false,
            None::<&str>,
        )?)?;
        menu.append(&MenuItem::with_id(
            app,
            format!("{REOPEN_PREFIX}{}", notice.account),
            crate::tray::escape(&notice.reopen),
            notice.can_reopen,
            None::<&str>,
        )?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    Ok(())
}

/// Handles a "Reopen B" menu item; `false` for any other id.
pub(crate) fn on_menu_event(app: &AppHandle, id: &str) -> bool {
    let Some(uuid) = id.strip_prefix(REOPEN_PREFIX) else {
        return false;
    };
    let account = AccountUuid::new(uuid);
    let core = app.state::<Core>().inner().clone();
    let app = app.clone();
    // Finding and starting the app takes a moment: not on the main thread.
    let spawned = std::thread::Builder::new()
        .name("mccboard-reopen-profile".into())
        .spawn(move || {
            if let Err(error) = crate::desktop::open(&core, Some(&account)) {
                eprintln!("mccboard: cannot reopen the desktop profile: {error}");
            }
            wake(&app);
        });
    if let Err(error) = spawned {
        eprintln!("mccboard: cannot reopen the desktop profile: {error}");
    }
    true
}

/// The desktop profile notices as of the last poll, in the app's language.
#[tauri::command]
pub(crate) fn desktop_notices(
    core: tauri::State<'_, Core>,
) -> Result<DesktopNotices, CommandError> {
    Ok(core.desktop_notices(crate::tray::language(&core))?)
}

/// Hides `uuid`'s notice until it happens again; the windows and the tray follow.
#[tauri::command]
pub(crate) fn dismiss_desktop_notice(
    app: AppHandle,
    core: tauri::State<'_, Core>,
    uuid: String,
) -> bool {
    let dismissed = core.dismiss_desktop_notice(&AccountUuid::new(uuid));
    wake(&app);
    dismissed
}
