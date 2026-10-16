//! Desktop profiles on the Windows taskbar (ticket #37): their Start-menu shortcuts, the
//! watcher that gives their windows their own button and icon, and `--open-profile <uuid>`
//! from a shortcut or a pin, which a second app instance hands to this one. The work is the
//! core's ([`Core::sync_desktop_shortcuts`], [`Core::running_desktop_profiles`],
//! `mccboard_core::windows_taskbar`); this module starts it and feeds it the app's events.

use mccboard_core::{AppArgs, Core};
use tauri::AppHandle;

#[cfg(windows)]
static WATCHER: std::sync::OnceLock<mccboard_core::windows_taskbar::ProfileWatcher> =
    std::sync::OnceLock::new();

/// Called once from the app's setup: syncs the shortcuts, starts the watcher, then does what
/// the app was started for: open a profile (`--open-profile`), else show the Instances
/// window.
pub(crate) fn setup(app: &tauri::App, core: &Core) {
    #[cfg(windows)]
    {
        let source_core = core.clone();
        let source = Box::new(move || match source_core.running_desktop_profiles() {
            Ok(profiles) => profiles,
            Err(error) => {
                eprintln!("mccboard: cannot list the running desktop profiles: {error}");
                Vec::new()
            }
        });
        match mccboard_core::windows_taskbar::ProfileWatcher::start(source, Vec::new()) {
            Ok(watcher) => {
                let _ = WATCHER.set(watcher);
            }
            Err(error) => eprintln!("mccboard: cannot watch desktop profile windows: {error}"),
        }
    }
    sync_shortcuts(core);
    let args = AppArgs::parse(std::env::args());
    on_args(app.handle(), core, args);
}

/// A second instance of the app was started with `argv` (a shortcut, a pin, a plain start):
/// it has handed them over and exited.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn on_second_instance(app: &AppHandle, core: &Core, argv: &[String]) {
    on_args(app, core, AppArgs::parse(argv));
}

fn on_args(app: &AppHandle, core: &Core, args: AppArgs) {
    let Some(account) = args.open_profile else {
        crate::tray::open_instances(app);
        return;
    };
    let core = core.clone();
    // Finding and starting the desktop app takes a moment: not on the main thread.
    let spawned = std::thread::Builder::new()
        .name("mccboard-open-profile".into())
        .spawn(move || {
            if let Err(error) = crate::desktop::open(&core, Some(&account)) {
                eprintln!("mccboard: cannot open the desktop profile {account}: {error}");
            }
        });
    if let Err(error) = spawned {
        eprintln!("mccboard: cannot open the desktop profile: {error}");
    }
}

/// `config.toml` changed (a label, a color, a profile added or removed, from this app, the
/// CLI or an editor): the shortcuts and the open windows follow.
pub(crate) fn config_changed(core: &Core) {
    sync_shortcuts(core);
    refresh();
}

/// Tells the watcher that profiles may have started or changed, so it looks now.
pub(crate) fn refresh() {
    #[cfg(windows)]
    if let Some(watcher) = WATCHER.get() {
        watcher.refresh();
    }
}

/// Syncs the Start-menu shortcuts off the calling thread; failures go to stderr (the UI
/// reports them on the operation that changed a profile).
fn sync_shortcuts(core: &Core) {
    let core = core.clone();
    let spawned = std::thread::Builder::new()
        .name("mccboard-shortcuts".into())
        .spawn(move || {
            if let Err(error) = core.sync_desktop_shortcuts() {
                eprintln!("mccboard: cannot update the desktop profile shortcuts: {error}");
            }
        });
    if let Err(error) = spawned {
        eprintln!("mccboard: cannot update the desktop profile shortcuts: {error}");
    }
}
