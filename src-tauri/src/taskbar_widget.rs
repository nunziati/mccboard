//! The taskbar widget (#45, ADR 0009): feeds the core's view model to the docked window
//! (`mccboard_core::windows_taskbar_widget`) and turns its clicks into the app's actions.
//!
//! - The tray's thread hands over every usage report it draws ([`update`]); a thread of this
//!   module computes the widget from the last one ([`Core::taskbar_widget`]) and again when
//!   the view says a text changes ([`TaskbarWidget::redraw_in`]): every minute, every second
//!   in a window's last minute.
//! - The window says when it shows or hides; the tray then draws mccboard's logo, or the worst
//!   weekly window ([`shown`]).
//! - Left click opens the dashboard (the Instances window); right click opens the tray's menu.
//!
//! Off Windows nothing starts, and the tray always shows the numbers.

use std::sync::Mutex;
#[cfg(windows)]
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};

use mccboard_core::{Core, TaskbarTheme, UsageReport};
#[cfg(windows)]
use mccboard_core::{TaskbarWidget, windows_taskbar_widget as window};
use tauri::AppHandle;
#[cfg(windows)]
use tauri::Manager;

/// Managed state: the way to the feeding thread.
#[cfg(windows)]
struct Feed(Mutex<Sender<Option<UsageReport>>>);

/// Whether the widget is on the taskbar, and with which theme.
static SHOWN: Mutex<Option<TaskbarTheme>> = Mutex::new(None);

/// Starts the widget's window and the thread that feeds it. Called once from the app's setup.
#[cfg_attr(not(windows), allow(unused_variables))]
pub(crate) fn setup(app: &tauri::App, core: &Core) {
    #[cfg(windows)]
    {
        let clicks = app.handle().clone();
        let shows = app.handle().clone();
        let events = window::WidgetEvents {
            on_click: Box::new(move |click| on_click(&clicks, click)),
            on_shown: Box::new(move |theme| {
                *SHOWN.lock().unwrap_or_else(|p| p.into_inner()) = theme;
                // The tray draws the logo, or the numbers, from now on.
                crate::tray::redraw(&shows);
            }),
        };
        let widget = match window::TaskbarWidgetWindow::start(events) {
            Ok(widget) => widget,
            Err(error) => {
                eprintln!("mccboard: cannot start the taskbar widget: {error}");
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        app.manage(Feed(Mutex::new(sender)));
        let core = core.clone();
        let started = std::thread::Builder::new()
            .name("mccboard-widget-feed".into())
            .spawn(move || feed(&core, &widget, &receiver));
        if let Err(error) = started {
            eprintln!("mccboard: cannot start the taskbar widget: {error}");
        }
    }
}

/// The tray drew `report` (`None`: usage can't be shown): the widget follows.
#[cfg_attr(not(windows), allow(unused_variables))]
pub(crate) fn update(app: &AppHandle, report: Option<UsageReport>) {
    #[cfg(windows)]
    if let Some(feed) = app.try_state::<Feed>() {
        let sender = feed.0.lock().unwrap_or_else(|p| p.into_inner());
        let _ = sender.send(report);
    }
}

/// The taskbar's theme while the widget is on it; `None` while it isn't (preference off, no
/// room, a taskbar it doesn't dock on, not Windows).
pub(crate) fn shown() -> Option<TaskbarTheme> {
    *SHOWN.lock().unwrap_or_else(|p| p.into_inner())
}

/// The side of the tray's icon when it shows the logo, in pixels: the system's small icon.
#[cfg(windows)]
pub(crate) fn logo_size() -> u32 {
    window::small_icon_size()
}

/// The side of the tray's icon when it shows the logo, in pixels (never, off Windows).
#[cfg(not(windows))]
pub(crate) fn logo_size() -> u32 {
    32
}

/// Computes the widget from the last report whenever a report arrives or a text is due to
/// change, and shows it.
#[cfg(windows)]
fn feed(
    core: &Core,
    widget: &window::TaskbarWidgetWindow,
    reports: &Receiver<Option<UsageReport>>,
) {
    let mut report: Option<UsageReport> = None;
    let mut wait = std::time::Duration::from_secs(60);
    loop {
        match reports.recv_timeout(wait) {
            Ok(new) => report = new,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let language = crate::tray::language(core);
        let view = match &report {
            Some(report) => core
                .taskbar_widget(report, language, &jiff::tz::TimeZone::system())
                .unwrap_or_else(|_| TaskbarWidget::unavailable(language)),
            None => TaskbarWidget::unavailable(language),
        };
        wait = view.redraw_in;
        widget.show(view);
    }
}

/// Runs on the widget's thread, which must not wait: the work goes to another thread.
#[cfg(windows)]
fn on_click(app: &AppHandle, click: window::WidgetClick) {
    let app = app.clone();
    let _ = std::thread::Builder::new()
        .name("mccboard-widget-click".into())
        .spawn(move || match click {
            window::WidgetClick::Left => crate::tray::open_instances(&app),
            window::WidgetClick::Right => crate::tray::show_menu(&app),
        });
}
