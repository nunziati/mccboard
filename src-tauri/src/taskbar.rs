//! Desktop profiles on the Windows taskbar (tickets #37, #38): their Start-menu shortcuts, the
//! watcher that gives their windows their own button and icon and their notification-area
//! icon the account's color, and `--open-profile <uuid>` from a shortcut or a pin, which a
//! second copy of mccboard's app hands to this one. The work is the core's
//! ([`Core::sync_desktop_shortcuts`], [`Core::running_desktop_profiles`],
//! `mccboard_core::windows_taskbar`, `mccboard_core::windows_profile_tray`); this module starts
//! it and feeds it the app's events.

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
        let language_core = core.clone();
        let tray = mccboard_core::windows_profile_tray::listener(Box::new(move || {
            crate::tray::language(&language_core)
        }));
        match mccboard_core::windows_taskbar::ProfileWatcher::start(source, vec![tray]) {
            Ok(watcher) => {
                let _ = WATCHER.set(watcher);
            }
            Err(error) => eprintln!("mccboard: cannot watch desktop profile windows: {error}"),
        }
    }
    sync_shortcuts(core);
    on_args(app.handle(), AppArgs::parse(std::env::args()));
}

/// A second copy of mccboard's app was started with `argv` (a shortcut, a pin, a plain
/// start): it has handed them over and exited.
pub(crate) fn on_second_copy(app: &AppHandle, argv: &[String]) {
    on_args(app, AppArgs::parse(argv));
}

fn on_args(app: &AppHandle, args: AppArgs) {
    match args.open_profile {
        Some(account) => crate::desktop::open_in_background(app, Some(account), |_| {}),
        None => crate::tray::open_instances(app),
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
    crate::desktop::in_background(
        "mccboard-shortcuts",
        "cannot update the desktop profile shortcuts",
        move || core.sync_desktop_shortcuts().map(drop),
    );
}
